// Copyright 2026 Maurice S. Barnum
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Note, grace group, ornament, articulation, and direction lowering.

use abc_parser::AnnotationPlacement;
use abc_parser::ErrorKind;
use abc_parser::Fraction;
use abc_parser::GraceElement;
use abc_parser::GraceGroup;
use abc_parser::ParseWarning;
use abc_parser::Pitch;
use abc_parser::Tempo;

use crate::harmony::parse_chord_symbol;
use crate::lower::beams::apply_beams_to_notes;
use crate::lower::types::GraceNoteStyle;
use crate::lower::types::LoweredDirection;
use crate::lower::types::LoweredDirectionKind;
use crate::lower::types::LoweredHarmony;
use crate::lower::types::LoweredNotations;
use crate::lower::types::LoweredNote;
use crate::lower::types::LoweredNoteKind;
use crate::lower::types::LoweredOrnament;
use crate::lower::types::LoweredSound;
use crate::lower::types::LoweredTechnical;
use crate::lower::types::MeasureItem;
use crate::lower::types::Span;
use crate::lower::types::VoiceLayerKey;
use crate::pitch::AccidentalPropagation;
use crate::pitch::ClefSpec;
use crate::pitch::MeasureAccidentalState;
use crate::pitch::ResolvedKey;
use crate::pitch::ResolvedPitch;
use crate::pitch::StemPolicy;
use crate::source_map::SourceProvenance;
use crate::timing::Duration;
use crate::timing::broken_rhythm_factors;
use crate::timing::decompose_graphic_duration;

/// Classification of ABC decorations into `MusicXML` semantic categories.
pub enum DecorationClassification {
    DirectionDynamics(&'static str),
    DirectionWedge(&'static str),
    DirectionSegno,
    DirectionCoda,
    DirectionNavWords {
        text: &'static str,
        sound: LoweredSound,
    },
    DirectionOctaveShift {
        shift_type: &'static str,
        size: u8,
    },
    Articulation(&'static str),
    Ornament(LoweredOrnament),
    Technical(LoweredTechnical),
    NoteFermata(&'static str),
    Arpeggio,
    Slide,
    Other(String),
}

/// Classifies an ABC decoration token name (e.g. `trill`, `crescendo(`, `f`, `fermata`)
/// into its corresponding `MusicXML` element classification.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn classify_decoration(name: &str) -> DecorationClassification {
    match name {
        "pppp" => DecorationClassification::DirectionDynamics("pppp"),
        "ppp" => DecorationClassification::DirectionDynamics("ppp"),
        "pp" => DecorationClassification::DirectionDynamics("pp"),
        "p" => DecorationClassification::DirectionDynamics("p"),
        "mp" => DecorationClassification::DirectionDynamics("mp"),
        "mf" => DecorationClassification::DirectionDynamics("mf"),
        "f" => DecorationClassification::DirectionDynamics("f"),
        "ff" => DecorationClassification::DirectionDynamics("ff"),
        "fff" => DecorationClassification::DirectionDynamics("fff"),
        "ffff" => DecorationClassification::DirectionDynamics("ffff"),
        "sfz" => DecorationClassification::DirectionDynamics("sfz"),

        "crescendo(" | "<(" => DecorationClassification::DirectionWedge("crescendo"),
        "crescendo)" | "<)" | "diminuendo)" | ">)" => {
            DecorationClassification::DirectionWedge("stop")
        }
        "diminuendo(" | ">(" => DecorationClassification::DirectionWedge("diminuendo"),

        "segno" | "S" => DecorationClassification::DirectionSegno,
        "coda" | "O" => DecorationClassification::DirectionCoda,

        "D.S." | "dalsegno" => DecorationClassification::DirectionNavWords {
            text: "D.S.",
            sound: LoweredSound::DalSegno("segno"),
        },
        "D.C." | "dacapo" => DecorationClassification::DirectionNavWords {
            text: "D.C.",
            sound: LoweredSound::DaCapo,
        },
        "dacoda" => DecorationClassification::DirectionNavWords {
            text: "Da Coda",
            sound: LoweredSound::ToCoda("coda"),
        },
        "fine" => DecorationClassification::DirectionNavWords {
            text: "Fine",
            sound: LoweredSound::Fine,
        },

        "8va(" => DecorationClassification::DirectionOctaveShift {
            shift_type: "down",
            size: 8,
        },
        "8va)" | "8vb)" => DecorationClassification::DirectionOctaveShift {
            shift_type: "stop",
            size: 8,
        },
        "8vb(" => DecorationClassification::DirectionOctaveShift {
            shift_type: "up",
            size: 8,
        },

        "staccato" | "." => DecorationClassification::Articulation("staccato"),
        "accent" | ">" | "emphasis" | "L" => DecorationClassification::Articulation("accent"),
        "^" | "marcato" => DecorationClassification::Articulation("strong-accent"),
        "tenuto" => DecorationClassification::Articulation("tenuto"),
        "wedge" => DecorationClassification::Articulation("staccatissimo"),
        "breath" => DecorationClassification::Articulation("breath-mark"),

        "trill" | "T" => DecorationClassification::Ornament(LoweredOrnament::TrillMark),
        "trill(" => DecorationClassification::Ornament(LoweredOrnament::TrillWithWavyStart),
        "trill)" => DecorationClassification::Ornament(LoweredOrnament::WavyStop),
        "lowermordent" | "mordent" | "M" => {
            DecorationClassification::Ornament(LoweredOrnament::Mordent)
        }
        "uppermordent" | "pralltriller" | "P" => {
            DecorationClassification::Ornament(LoweredOrnament::InvertedMordent)
        }
        "turn" => DecorationClassification::Ornament(LoweredOrnament::Turn { slash: false }),
        "turnx" => DecorationClassification::Ornament(LoweredOrnament::Turn { slash: true }),
        "invertedturn" => {
            DecorationClassification::Ornament(LoweredOrnament::InvertedTurn { slash: false })
        }
        "invertedturnx" => {
            DecorationClassification::Ornament(LoweredOrnament::InvertedTurn { slash: true })
        }
        "roll" | "~" => DecorationClassification::Ornament(LoweredOrnament::TremoloSingle(3)),

        "upbow" | "u" => DecorationClassification::Technical(LoweredTechnical::UpBow),
        "downbow" | "v" => DecorationClassification::Technical(LoweredTechnical::DownBow),
        "open" => DecorationClassification::Technical(LoweredTechnical::OpenString),
        "thumb" => DecorationClassification::Technical(LoweredTechnical::ThumbPosition),
        "snap" => DecorationClassification::Technical(LoweredTechnical::SnapPizzicato),
        "plus" | "+" => DecorationClassification::Technical(LoweredTechnical::Stopped),
        "0" | "1" | "2" | "3" | "4" | "5" => {
            DecorationClassification::Technical(LoweredTechnical::Fingering(name.to_owned()))
        }

        "fermata" | "H" => DecorationClassification::NoteFermata("upright"),
        "invertedfermata" => DecorationClassification::NoteFermata("inverted"),

        "arpeggio" => DecorationClassification::Arpeggio,
        "slide" => DecorationClassification::Slide,

        other => DecorationClassification::Other(other.to_owned()),
    }
}

/// Parses free-text annotation coordinate offsets like `"@10,-20 text"`.
#[must_use]
pub fn parse_free_annotation(s: &str) -> (Option<i32>, Option<i32>, String) {
    let rest = s.strip_prefix('@').unwrap_or(s);
    let mut parts = rest.splitn(2, ',');
    if let (Some(x_str), Some(after_comma)) = (parts.next(), parts.next())
        && let Ok(x) = x_str.trim().parse::<i32>()
    {
        let mut y_parts = after_comma.splitn(2, |c: char| c.is_whitespace());
        if let Some(y_str) = y_parts.next()
            && let Ok(y) = y_str.trim().parse::<i32>()
        {
            let rem = y_parts.next().unwrap_or("").trim().to_owned();
            return (Some(x), Some(y), rem);
        }
    }
    (None, None, s.to_owned())
}

/// Builds a [`LoweredNoteKind`] for a pitched or percussion note.
#[must_use]
pub fn build_note_kind(
    active_clef: &ClefSpec,
    resolved: ResolvedPitch,
    show_accidental: bool,
) -> LoweredNoteKind {
    if active_clef.sign == "percussion" {
        LoweredNoteKind::Unpitched {
            display_step: resolved.step,
            display_octave: resolved.octave,
        }
    } else {
        LoweredNoteKind::Pitched {
            step: resolved.step,
            octave: resolved.octave,
            alter: resolved.alter,
            accidental: show_accidental
                .then(|| {
                    resolved
                        .written_accidental
                        .map(crate::pitch::AccidentalValue::as_str)
                })
                .flatten(),
        }
    }
}

/// Converts an ABC [`Tempo`] into a `MusicXML` [`LoweredDirection`].
#[must_use]
pub fn tempo_to_direction(
    tempo: &Tempo<String>,
    prov: Option<&SourceProvenance>,
) -> LoweredDirection {
    match tempo {
        Tempo::MetronomeMark {
            prelude,
            beats,
            bpm,
            postlude,
        } => {
            let default_beat = Fraction {
                numerator: 1,
                denominator: 4,
            };
            let first_beat = beats.first().copied().unwrap_or(default_beat);
            let first_dur = Duration::from_fraction(first_beat);
            let (beat_unit, dots) = decompose_graphic_duration(first_dur, first_dur)
                .into_iter()
                .next()
                .map_or(("quarter", 0), |s| (s.note_type.as_str(), s.dots));
            let total_beat_dur = if beats.is_empty() {
                first_dur
            } else {
                beats.iter().fold(Duration::ZERO, |acc, &frac| {
                    acc.add(Duration::from_fraction(frac))
                })
            };
            let quarter_multiple = total_beat_dur.quarter_note_multiple();
            let quarter_bpm = u128::from(*bpm).saturating_mul(quarter_multiple.numerator())
                / quarter_multiple.denominator().max(1);
            let sound_tempo = u32::try_from(quarter_bpm).unwrap_or(*bpm);

            LoweredDirection::new(
                Some("above"),
                LoweredDirectionKind::Metronome {
                    prelude: prelude.clone(),
                    beat_unit,
                    dots,
                    bpm: *bpm,
                    postlude: postlude.clone(),
                },
                prov.cloned(),
            )
            .with_sound(LoweredSound::Tempo(sound_tempo))
        }
        Tempo::TextOnly(text) | Tempo::Deprecated(text) => {
            LoweredDirection::words(Some("above"), text.clone(), None, None, None, prov.cloned())
        }
    }
}

/// Applies a classified ABC decoration to `notations` if it belongs on `<notations>`,
/// or returns a standalone [`LoweredDirection`] if it is a measure-level direction.
pub fn apply_decoration_or_direction(
    name: &str,
    prov: Option<SourceProvenance>,
    notations: &mut LoweredNotations,
) -> Option<LoweredDirection> {
    match classify_decoration(name) {
        DecorationClassification::DirectionDynamics(dyn_str) => Some(LoweredDirection::new(
            Some("below"),
            LoweredDirectionKind::Dynamics(dyn_str),
            prov,
        )),
        DecorationClassification::DirectionWedge(wedge_type) => Some(LoweredDirection::new(
            Some("below"),
            LoweredDirectionKind::Wedge(wedge_type),
            prov,
        )),
        DecorationClassification::DirectionSegno => Some(
            LoweredDirection::new(Some("above"), LoweredDirectionKind::Segno, prov)
                .with_sound(LoweredSound::Segno("segno")),
        ),
        DecorationClassification::DirectionCoda => Some(
            LoweredDirection::new(Some("above"), LoweredDirectionKind::Coda, prov)
                .with_sound(LoweredSound::Coda("coda")),
        ),
        DecorationClassification::DirectionNavWords { text, sound } => Some(
            LoweredDirection::words(Some("above"), text.to_owned(), None, None, None, prov)
                .with_sound(sound),
        ),
        DecorationClassification::DirectionOctaveShift { shift_type, size } => {
            Some(LoweredDirection::new(
                None,
                LoweredDirectionKind::OctaveShift { shift_type, size },
                prov,
            ))
        }
        DecorationClassification::Articulation(art) => {
            notations.articulations.push(art);
            None
        }
        DecorationClassification::Ornament(orn) => {
            notations.ornaments.push(orn);
            None
        }
        DecorationClassification::Technical(tech) => {
            notations.technical.push(tech);
            None
        }
        DecorationClassification::NoteFermata(ferm) => {
            notations.fermata = Some(ferm);
            None
        }
        DecorationClassification::Arpeggio => {
            notations.arpeggiate = true;
            None
        }
        DecorationClassification::Slide => {
            notations.slide_start = true;
            None
        }
        DecorationClassification::Other(other) => {
            notations.other.push(other);
            None
        }
    }
}

/// Lowers an ABC annotation (`"Am"`, `"^pizz"`, `"@10,-20 text"`) into a
/// [`MeasureItem::Harmony`] or [`MeasureItem::Direction`].
pub fn lower_annotation(
    placement: AnnotationPlacement,
    text: &str,
    prov: SourceProvenance,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> MeasureItem {
    match placement {
        AnnotationPlacement::ChordSymbol => {
            if let Some(spec) = parse_chord_symbol(text) {
                return MeasureItem::Harmony(LoweredHarmony {
                    spec,
                    provenance: Some(prov),
                });
            }
            warnings.push(ParseWarning {
                kind: ErrorKind::InvalidMusic,
                message: format!(
                    "annotation \"{text}\" is not a valid chord symbol; emitted as text direction"
                ),
                span: prov.element_span(),
                related: Vec::new(),
            });
            MeasureItem::Direction(LoweredDirection::words(
                Some("above"),
                text.to_owned(),
                None,
                None,
                None,
                Some(prov),
            ))
        }
        AnnotationPlacement::Above => MeasureItem::Direction(LoweredDirection::words(
            Some("above"),
            text.to_owned(),
            None,
            None,
            None,
            Some(prov),
        )),
        AnnotationPlacement::Below => MeasureItem::Direction(LoweredDirection::words(
            Some("below"),
            text.to_owned(),
            None,
            None,
            None,
            Some(prov),
        )),
        AnnotationPlacement::Left => MeasureItem::Direction(LoweredDirection::words(
            Some("above"),
            text.to_owned(),
            Some("right"),
            None,
            None,
            Some(prov),
        )),
        AnnotationPlacement::Right => MeasureItem::Direction(LoweredDirection::words(
            Some("above"),
            text.to_owned(),
            Some("left"),
            None,
            None,
            Some(prov),
        )),
        AnnotationPlacement::Free => {
            let (rx, ry, rem) = parse_free_annotation(text);
            MeasureItem::Direction(LoweredDirection::words(None, rem, None, rx, ry, Some(prov)))
        }
    }
}

/// Lowers an ABC [`GraceGroup`] into a sequence of [`LoweredNote`] items.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn lower_grace_group(
    grace: &GraceGroup,
    unit_length: Fraction,
    active_octave_shift: i8,
    active_key: &ResolvedKey,
    active_clef: &ClefSpec,
    active_propagation: AccidentalPropagation,
    measure_accidentals: &mut MeasureAccidentalState,
    voice: u32,
    voice_key: VoiceLayerKey,
    line_span: Span,
    element_span: Span,
) -> Vec<LoweredNote> {
    let mut notes_in_grace: Vec<(Pitch, Duration, bool)> = Vec::new();
    let mut next_broken = Duration::ONE;
    for elem in &grace.elements {
        match elem {
            GraceElement::Note(n) => {
                let broken = std::mem::replace(&mut next_broken, Duration::ONE);
                let has_explicit =
                    n.length.numerator != 1 || n.length.denominator != 1 || broken != Duration::ONE;
                let base_dur = Duration::from_fraction(unit_length)
                    .multiply(Duration::from_note_length(n.length))
                    .multiply(broken);
                notes_in_grace.push((n.pitch, base_dur, has_explicit));
            }
            GraceElement::BrokenRhythm(rhythm) => {
                if let Some((f1, f2)) = broken_rhythm_factors(*rhythm)
                    && let Some(prev) = notes_in_grace.last_mut()
                {
                    prev.1 = prev.1.multiply(f1);
                    prev.2 = true;
                    next_broken = f2;
                }
            }
        }
    }

    let note_count = notes_in_grace.len();
    let mut lowered = Vec::with_capacity(note_count);
    let stem_str = match active_key.stem_policy {
        StemPolicy::Auto => None,
        StemPolicy::Bagpipe => Some("up"),
    };

    let elem_prov = SourceProvenance::new(line_span, element_span);

    for (idx, (pitch, dur, has_explicit)) in notes_in_grace.into_iter().enumerate() {
        let is_slash = grace.acciaccatura && (idx == 0 || note_count == 1);
        let resolved = measure_accidentals.resolve_pitch(
            pitch,
            active_octave_shift,
            &active_key.offsets,
            active_propagation,
            None,
        );

        let (note_type, dots) = if has_explicit {
            decompose_graphic_duration(dur, dur)
                .into_iter()
                .next()
                .map_or((None, 0), |s| (Some(s.note_type.as_str()), s.dots))
        } else if note_count == 1 {
            (Some("eighth"), 0)
        } else {
            (Some("16th"), 0)
        };

        let kind = build_note_kind(active_clef, resolved, true);

        lowered.push(LoweredNote {
            print_object: true,
            grace: Some(if is_slash {
                GraceNoteStyle::Acciaccatura
            } else {
                GraceNoteStyle::Appoggiatura
            }),
            is_chord_member: false,
            kind,
            sounding_duration: Duration::ZERO,
            voice,
            voice_key,
            staff: None,
            note_type,
            dots,
            time_modification: None,
            stem: stem_str,
            beams: Vec::new(),
            notations: None,
            lyrics: Vec::new(),
            provenance: Some(elem_prov.clone()),
        });
    }

    if lowered.len() >= 2 {
        let mut note_refs: Vec<&mut LoweredNote> = lowered.iter_mut().collect();
        apply_beams_to_notes(&mut note_refs);
    }

    lowered
}
