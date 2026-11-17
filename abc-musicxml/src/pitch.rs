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

//! Key signatures, modal/bagpipe keys, per-measure accidental state,
//! microtonal alterations, clefs, and transposition mapping for `MusicXML` 4.0.

use std::borrow::Cow;
use std::fmt;

use abc_parser::Accidental;
use abc_parser::FieldParameter;
use abc_parser::KeySignature;
use abc_parser::Pitch;
use abc_parser::PitchClass;
use abc_parser::key::conventional_key_fifths;
use abc_parser::key::explicit_key_accidental;
use abc_parser::pitch::Offset;
use abc_parser::pitch::accidental_offset;
use abc_parser::pitch::class_index;
use abc_parser::pitch::class_number;
use abc_parser::pitch::parse_single_pitch;

use crate::writer::XmlWriter;

/// `MusicXML` 4.0 `<accidental>` / `<key-accidental>` text value (`accidental-value`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccidentalValue {
    /// `"natural"` (`=`).
    Natural,
    /// `"sharp"` (`^`).
    Sharp,
    /// `"double-sharp"` (`^^`).
    DoubleSharp,
    /// `"flat"` (`_`).
    Flat,
    /// `"flat-flat"` (`__`).
    FlatFlat,
    /// `"quarter-sharp"` (`^/` or `^1/2`).
    QuarterSharp,
    /// `"quarter-flat"` (`_/` or `_1/2`).
    QuarterFlat,
    /// `"three-quarters-sharp"` (`^3/2`).
    ThreeQuartersSharp,
    /// `"three-quarters-flat"` (`_3/2`).
    ThreeQuartersFlat,
    /// `"other"` (non-quarter microtonal accidental).
    Other,
}

impl AccidentalValue {
    /// Returns the `MusicXML` 4.0 `accidental-value` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Natural => "natural",
            Self::Sharp => "sharp",
            Self::DoubleSharp => "double-sharp",
            Self::Flat => "flat",
            Self::FlatFlat => "flat-flat",
            Self::QuarterSharp => "quarter-sharp",
            Self::QuarterFlat => "quarter-flat",
            Self::ThreeQuartersSharp => "three-quarters-sharp",
            Self::ThreeQuartersFlat => "three-quarters-flat",
            Self::Other => "other",
        }
    }
}

/// Maps an [`abc_parser::Accidental`] to its rational semitone [`Offset`] and
/// corresponding `MusicXML` [`AccidentalValue`].
#[must_use]
pub fn accidental_to_musicxml(accidental: Accidental) -> (Offset, AccidentalValue) {
    let offset = accidental_offset(accidental);
    let value = offset_to_accidental_value(offset);
    (offset, value)
}

/// Maps a signed semitone [`Offset`] to its `MusicXML` [`AccidentalValue`].
#[must_use]
pub const fn offset_to_accidental_value(offset: Offset) -> AccidentalValue {
    match (offset.numerator, offset.denominator) {
        (0, _) => AccidentalValue::Natural,
        (1, 1) => AccidentalValue::Sharp,
        (2, 1) => AccidentalValue::DoubleSharp,
        (-1, 1) => AccidentalValue::Flat,
        (-2, 1) => AccidentalValue::FlatFlat,
        (1, 2) => AccidentalValue::QuarterSharp,
        (-1, 2) => AccidentalValue::QuarterFlat,
        (3, 2) => AccidentalValue::ThreeQuartersSharp,
        (-3, 2) => AccidentalValue::ThreeQuartersFlat,
        _ => AccidentalValue::Other,
    }
}

/// Formats a semitone [`Offset`] for `<alter>` or `<key-alter>`, returning `None` when zero.
#[must_use]
pub fn format_alter(offset: Offset) -> Option<Cow<'static, str>> {
    if offset.is_zero() {
        return None;
    }
    Some(format_alter_including_zero(offset))
}

/// Formats a semitone [`Offset`] as an integer or decimal string, returning `"0"` when zero.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn format_alter_including_zero(offset: Offset) -> Cow<'static, str> {
    if offset.is_zero() {
        return Cow::Borrowed("0");
    }
    let num = offset.numerator;
    let den = offset.denominator;
    match (num, den) {
        (1, 1) => Cow::Borrowed("1"),
        (-1, 1) => Cow::Borrowed("-1"),
        (2, 1) => Cow::Borrowed("2"),
        (-2, 1) => Cow::Borrowed("-2"),
        (1, 2) => Cow::Borrowed("0.5"),
        (-1, 2) => Cow::Borrowed("-0.5"),
        (3, 2) => Cow::Borrowed("1.5"),
        (-3, 2) => Cow::Borrowed("-1.5"),
        (_, 1) => Cow::Owned(num.to_string()),
        (_, 2) => {
            let sign = if num < 0 { "-" } else { "" };
            let abs = num.unsigned_abs();
            Cow::Owned(format!("{sign}{}.5", abs / 2))
        }
        (_, 4) => {
            let sign = if num < 0 { "-" } else { "" };
            let abs = num.unsigned_abs();
            let frac = if abs % 4 == 1 { "25" } else { "75" };
            Cow::Owned(format!("{sign}{}.{frac}", abs / 4))
        }
        _ => {
            let value = (num as f64) / (den as f64);
            Cow::Owned(format!("{value}"))
        }
    }
}

/// Returns the `MusicXML` `<step>` / `<key-step>` letter (`"A"`..`"G"`) for `class`.
#[must_use]
pub const fn pitch_step_str(class: PitchClass) -> &'static str {
    match class {
        PitchClass::C => "C",
        PitchClass::D => "D",
        PitchClass::E => "E",
        PitchClass::F => "F",
        PitchClass::G => "G",
        PitchClass::A => "A",
        PitchClass::B => "B",
    }
}

/// Intra-measure accidental propagation rule (`%%propagate-accidentals`, ABC 2.1 §11.3).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AccidentalPropagation {
    /// Accidentals apply across all octaves of that pitch class within the bar (ABC 2.1 §11.3 default).
    #[default]
    Pitch,
    /// Accidentals apply only to the same pitch class and octave within the bar.
    Octave,
    /// Accidentals apply only to the single note on which they are written.
    Not,
}

/// Parses the argument of a `%%propagate-accidentals` directive.
#[must_use]
pub fn parse_propagate_accidentals(arguments: &str) -> Option<AccidentalPropagation> {
    let token = arguments.split_whitespace().next()?;
    if token.eq_ignore_ascii_case("not") || token == "0" || token.eq_ignore_ascii_case("false") {
        Some(AccidentalPropagation::Not)
    } else if token.eq_ignore_ascii_case("octave") {
        Some(AccidentalPropagation::Octave)
    } else if token.eq_ignore_ascii_case("pitch")
        || token.eq_ignore_ascii_case("all")
        || token == "1"
        || token.eq_ignore_ascii_case("true")
    {
        Some(AccidentalPropagation::Pitch)
    } else {
        None
    }
}

/// Resolved `MusicXML` pitch and optional written `<accidental>` for one note.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedPitch {
    /// Diatonic step (`C`..`B`).
    pub step: PitchClass,
    /// `MusicXML` octave (`4` = Middle C octave).
    pub octave: i32,
    /// Sounding chromatic alteration relative to natural diatonic step.
    pub alter: Offset,
    /// Written `<accidental>` element, if one should be emitted.
    pub written_accidental: Option<AccidentalValue>,
}

const MIN_TRACKED_OCTAVE: i8 = -8;
const MAX_TRACKED_OCTAVE: i8 = 7;
const OCTAVE_SLOTS: usize = 16;

const fn octave_slot(octave: i8) -> Option<usize> {
    if octave >= MIN_TRACKED_OCTAVE && octave <= MAX_TRACKED_OCTAVE {
        Some((octave - MIN_TRACKED_OCTAVE).cast_unsigned() as usize)
    } else {
        None
    }
}

/// Per-measure accidental tracker supporting `pitch`, `octave`, and `not` propagation modes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MeasureAccidentalState {
    abc_by_class: [Option<Offset>; 7],
    abc_by_octave: [[Option<Offset>; OCTAVE_SLOTS]; 7],
    displayed_by_octave: [[Option<Offset>; OCTAVE_SLOTS]; 7],
}

impl MeasureAccidentalState {
    /// Creates an empty measure accidental state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            abc_by_class: [None; 7],
            abc_by_octave: [[None; OCTAVE_SLOTS]; 7],
            displayed_by_octave: [[None; OCTAVE_SLOTS]; 7],
        }
    }

    /// Resets all intra-measure accidentals at a barline boundary.
    pub const fn clear(&mut self) {
        self.abc_by_class = [None; 7];
        self.abc_by_octave = [[None; OCTAVE_SLOTS]; 7];
        self.displayed_by_octave = [[None; OCTAVE_SLOTS]; 7];
    }

    /// Resolves an [`abc_parser::Pitch`] into a [`ResolvedPitch`] and updates measure state.
    ///
    /// If `tied_alter` is `Some(offset)` and `pitch.accidental` is `None`, the tied
    /// note's sounding alteration is preserved across barlines per ABC 2.1 §4.2.
    pub fn resolve_pitch(
        &mut self,
        pitch: Pitch,
        voice_octave_shift: i8,
        key_offsets: &[Offset; 7],
        propagation: AccidentalPropagation,
        tied_alter: Option<Offset>,
    ) -> ResolvedPitch {
        let octave = 4 + i32::from(pitch.octave) + i32::from(voice_octave_shift);
        let idx = class_index(pitch.class);
        let oct_slot = octave_slot(pitch.octave);
        let displayed_current = oct_slot
            .and_then(|slot| self.displayed_by_octave[idx][slot])
            .unwrap_or(key_offsets[idx]);

        if let Some(accidental) = pitch.accidental {
            let (alter, acc_val) = accidental_to_musicxml(accidental);
            match propagation {
                AccidentalPropagation::Pitch => self.abc_by_class[idx] = Some(alter),
                AccidentalPropagation::Octave => {
                    if let Some(slot) = oct_slot {
                        self.abc_by_octave[idx][slot] = Some(alter);
                    }
                }
                AccidentalPropagation::Not => {}
            }
            if let Some(slot) = oct_slot {
                self.displayed_by_octave[idx][slot] = Some(alter);
            }
            return ResolvedPitch {
                step: pitch.class,
                octave,
                alter,
                written_accidental: Some(acc_val),
            };
        }

        if let Some(tied) = tied_alter {
            return ResolvedPitch {
                step: pitch.class,
                octave,
                alter: tied,
                written_accidental: None,
            };
        }

        let alter = match propagation {
            AccidentalPropagation::Pitch => self.abc_by_class[idx].unwrap_or(key_offsets[idx]),
            AccidentalPropagation::Octave => oct_slot
                .and_then(|slot| self.abc_by_octave[idx][slot])
                .unwrap_or(key_offsets[idx]),
            AccidentalPropagation::Not => key_offsets[idx],
        };
        let written_accidental = if alter == displayed_current {
            None
        } else {
            if let Some(slot) = oct_slot {
                self.displayed_by_octave[idx][slot] = Some(alter);
            }
            Some(offset_to_accidental_value(alter))
        };

        ResolvedPitch {
            step: pitch.class,
            octave,
            alter,
            written_accidental,
        }
    }
}

/// Stem direction policy implied by the active key signature (`K:HP` / `K:Hp`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StemPolicy {
    /// Standard automatic stem direction (`<stem>` omitted unless overridden).
    #[default]
    Auto,
    /// Highland bagpipe policy (`K:HP` / `K:Hp`): normal notes `<stem>down</stem>`,
    /// grace notes `<stem>up</stem>`.
    Bagpipe,
}

/// One altered step in a non-traditional `MusicXML` `<key>` element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyStepAlter {
    /// Diatonic step (`C`..`B`).
    pub step: PitchClass,
    /// Chromatic alteration relative to natural step.
    pub alter: Offset,
    /// Optional explicit `<key-accidental>` (for natural cancellations or microtones).
    pub accidental: Option<AccidentalValue>,
}

/// Representation of a `MusicXML` 4.0 `<key>` element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyKind {
    /// Traditional `<key><fifths>...</fifths><mode>...</mode></key>`.
    Traditional {
        /// Circle-of-fifths count ($-7 \dots +7$).
        fifths: i8,
        /// Canonical `MusicXML` mode name (`"major"`, `"minor"`, `"dorian"`, etc.).
        mode: &'static str,
    },
    /// Unprinted/empty key (`<key print-object="no"><fifths>0</fifths><mode>none</mode></key>`).
    None,
    /// Non-traditional `<key>` with explicit `<key-step>`, `<key-alter>`, and `<key-accidental>`.
    NonTraditional(Vec<KeyStepAlter>),
}

/// `MusicXML` 4.0 `<clef>` specification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClefSpec {
    /// `<sign>` (`"G"`, `"F"`, `"C"`, `"percussion"`).
    pub sign: &'static str,
    /// Optional `<line>` (`1..=5`).
    pub line: Option<u8>,
    /// Optional `<clef-octave-change>` (`-1`, `1`, `-2`, `2`).
    pub octave_change: Option<i8>,
    /// Whether the clef is printed (`false` for `clef=none`).
    pub print_object: bool,
}

impl Default for ClefSpec {
    fn default() -> Self {
        Self::TREBLE
    }
}

impl ClefSpec {
    /// Standard treble clef (`G` on line 2).
    pub const TREBLE: Self = Self {
        sign: "G",
        line: Some(2),
        octave_change: None,
        print_object: true,
    };

    /// Standard bass clef (`F` on line 4).
    pub const BASS: Self = Self {
        sign: "F",
        line: Some(4),
        octave_change: None,
        print_object: true,
    };

    /// Standard percussion clef (`percussion`).
    pub const PERCUSSION: Self = Self {
        sign: "percussion",
        line: None,
        octave_change: None,
        print_object: true,
    };
}

/// `MusicXML` 4.0 `<transpose>` specification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransposeSpec {
    /// `<chromatic>` semitones within a single octave (`-11..=11`).
    pub chromatic: i16,
    /// Optional `<octave-change>` (emitted only when `|total_semitones| >= 12`).
    pub octave_change: Option<i16>,
}

impl TransposeSpec {
    /// Constructs a [`TransposeSpec`] from a signed semitone count (`transpose=<semitones>`).
    #[must_use]
    pub const fn from_semitones(semitones: i16) -> Self {
        let chromatic = semitones % 12;
        let octaves = semitones / 12;
        Self {
            chromatic,
            octave_change: if octaves == 0 { None } else { Some(octaves) },
        }
    }
}

/// Modifiers parsed from `K:` or `V:` parameters (`clef=`, `middle=`, `transpose=`, `octave=`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StaffParameters {
    /// Explicit clef override, if specified.
    pub clef: Option<ClefSpec>,
    /// Explicit playback transposition, if specified.
    pub transpose: Option<TransposeSpec>,
    /// Explicit written octave shift (`octave=<n>`), if specified.
    pub octave_shift: Option<i8>,
}

/// Complete resolved semantics of a `K:` key signature field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedKey {
    /// `MusicXML` `<key>` representation.
    pub kind: KeyKind,
    /// Effective accidental offsets for the 7 diatonic pitch classes (`C`..`B`).
    pub offsets: [Offset; 7],
    /// Stem direction policy (`Auto` or `Bagpipe` for `K:HP`/`K:Hp`).
    pub stem_policy: StemPolicy,
    /// Optional clef, transpose, and octave parameters attached to the `K:` field.
    pub parameters: StaffParameters,
}

/// Parses `clef=`, `middle=`/`m=`, `transpose=`/`t=`, and `octave=` from a slice of [`FieldParameter`]s.
#[must_use]
pub fn parse_staff_parameters(parameters: &[FieldParameter<String>]) -> StaffParameters {
    let mut result = StaffParameters::default();
    let mut middle_pitch: Option<Pitch> = None;

    for param in parameters {
        match param.name.as_deref() {
            Some(name) if name.eq_ignore_ascii_case("clef") => {
                if let Some(clef) = parse_clef_token(&param.value) {
                    result.clef = Some(clef);
                }
            }
            Some(name) if name.eq_ignore_ascii_case("middle") || name.eq_ignore_ascii_case("m") => {
                middle_pitch = parse_single_pitch(&param.value);
            }
            Some(name)
                if name.eq_ignore_ascii_case("transpose") || name.eq_ignore_ascii_case("t") =>
            {
                if let Ok(semitones) = param.value.parse::<i16>() {
                    result.transpose = Some(TransposeSpec::from_semitones(semitones));
                }
            }
            Some(name) if name.eq_ignore_ascii_case("octave") => {
                if let Ok(oct) = param.value.parse::<i8>() {
                    result.octave_shift = Some(oct);
                }
            }
            None => {
                if let Some(clef) = parse_clef_token(&param.value) {
                    result.clef = Some(clef);
                }
            }
            Some(_) => {}
        }
    }

    if let Some(middle) = middle_pitch {
        let mut clef = result.clef.unwrap_or(ClefSpec::TREBLE);
        apply_middle_pitch(&mut clef, middle);
        result.clef = Some(clef);
    }

    result
}

/// Resolves an [`abc_parser::KeySignature`] (with optional raw `K:` source slice to
/// disambiguate `K:none`, `K:perc`, `K:HP`, and `K:Hp`) into a [`ResolvedKey`].
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn resolve_key_signature(
    key: &KeySignature<String>,
    raw_source_slice: Option<&str>,
) -> ResolvedKey {
    let (mode_token, mut remaining_params) = extract_mode_token(key);
    let mut is_explicit = mode_token.eq_ignore_ascii_case("exp");
    if !is_explicit
        && let Some(first) = remaining_params.first()
        && first.name.is_none()
        && first.value.eq_ignore_ascii_case("exp")
    {
        is_explicit = true;
        remaining_params = &remaining_params[1..];
    }

    let mut staff_params = parse_staff_parameters(remaining_params);

    // Handle tonic-less keys (`K:none`, `K:perc`, `K:HP`, `K:Hp`, or keyless `K:clef=...`).
    let Some(tonic) = key.tonic else {
        let special = detect_special_key_token(mode_token, raw_source_slice);
        let explicit_mods = collect_explicit_key_accidentals(remaining_params);

        match special {
            SpecialKey::BagpipeHpUpper => {
                let mut offsets = [Offset::ZERO; 7];
                offsets[class_index(PitchClass::F)] = Offset::integer(1);
                offsets[class_index(PitchClass::C)] = Offset::integer(1);
                return ResolvedKey {
                    kind: KeyKind::None,
                    offsets,
                    stem_policy: StemPolicy::Bagpipe,
                    parameters: staff_params,
                };
            }
            SpecialKey::BagpipeHpMixed => {
                let mut offsets = [Offset::ZERO; 7];
                offsets[class_index(PitchClass::F)] = Offset::integer(1);
                offsets[class_index(PitchClass::C)] = Offset::integer(1);
                let steps = vec![
                    KeyStepAlter {
                        step: PitchClass::F,
                        alter: Offset::integer(1),
                        accidental: None,
                    },
                    KeyStepAlter {
                        step: PitchClass::C,
                        alter: Offset::integer(1),
                        accidental: None,
                    },
                    KeyStepAlter {
                        step: PitchClass::G,
                        alter: Offset::ZERO,
                        accidental: Some(AccidentalValue::Natural),
                    },
                ];
                return ResolvedKey {
                    kind: KeyKind::NonTraditional(steps),
                    offsets,
                    stem_policy: StemPolicy::Bagpipe,
                    parameters: staff_params,
                };
            }
            SpecialKey::Percussion => {
                if staff_params.clef.is_none() {
                    staff_params.clef = Some(ClefSpec::PERCUSSION);
                }
            }
            SpecialKey::None => {}
        }

        let mut offsets = [Offset::ZERO; 7];
        if !explicit_mods.is_empty() {
            for &(step, alter, _) in &explicit_mods {
                offsets[class_index(step)] = alter;
            }
            let steps = explicit_mods
                .into_iter()
                .map(|(step, alter, acc)| KeyStepAlter {
                    step,
                    alter,
                    accidental: acc,
                })
                .collect();
            return ResolvedKey {
                kind: KeyKind::NonTraditional(steps),
                offsets,
                stem_policy: StemPolicy::Auto,
                parameters: staff_params,
            };
        }

        return ResolvedKey {
            kind: KeyKind::None,
            offsets,
            stem_policy: StemPolicy::Auto,
            parameters: staff_params,
        };
    };

    // Pitched tonic (`A`..`G` with optional `#`/`b`).
    let fifths = if is_explicit {
        0
    } else {
        conventional_key_fifths(tonic, mode_token).unwrap_or(0)
    };
    let musicxml_mode = canonical_musicxml_mode(mode_token);

    let mut offsets = [Offset::ZERO; 7];
    let base_steps = fifths_to_steps(fifths);
    for &(step, alter) in &base_steps {
        offsets[class_index(step)] = alter;
    }

    let explicit_mods = collect_explicit_key_accidentals(remaining_params);
    if is_explicit || !explicit_mods.is_empty() {
        for &(step, alter, _) in &explicit_mods {
            offsets[class_index(step)] = alter;
        }
        let mut steps = Vec::new();
        if !is_explicit {
            for &(step, alter) in &base_steps {
                if !explicit_mods.iter().any(|&(s, _, _)| s == step) {
                    steps.push(KeyStepAlter {
                        step,
                        alter,
                        accidental: None,
                    });
                }
            }
        }
        for (step, alter, acc) in explicit_mods {
            steps.push(KeyStepAlter {
                step,
                alter,
                accidental: acc,
            });
        }
        return ResolvedKey {
            kind: KeyKind::NonTraditional(steps),
            offsets,
            stem_policy: StemPolicy::Auto,
            parameters: staff_params,
        };
    }

    ResolvedKey {
        kind: KeyKind::Traditional {
            fifths,
            mode: musicxml_mode,
        },
        offsets,
        stem_policy: StemPolicy::Auto,
        parameters: staff_params,
    }
}

/// Writes a `MusicXML` 4.0 `<key>` element into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `writer` fails.
pub fn write_key_xml<W: fmt::Write>(writer: &mut XmlWriter<W>, key: &KeyKind) -> fmt::Result {
    match key {
        KeyKind::Traditional { fifths, mode } => {
            writer.open_tag("key")?;
            writer.display_element("fifths", *fifths)?;
            writer.text_element("mode", mode)?;
        }
        KeyKind::None => {
            writer.open_tag_with_attrs("key", &[("print-object", "no")])?;
            writer.text_element("fifths", "0")?;
            writer.text_element("mode", "none")?;
        }
        KeyKind::NonTraditional(steps) => {
            writer.open_tag("key")?;
            for step in steps {
                writer.text_element("key-step", pitch_step_str(step.step))?;
                let alter_str = format_alter_including_zero(step.alter);
                writer.text_element("key-alter", &alter_str)?;
                if let Some(acc) = step.accidental {
                    writer.text_element("key-accidental", acc.as_str())?;
                }
            }
        }
    }
    writer.close_tag("key")
}

/// Writes a `MusicXML` 4.0 `<clef>` element with an optional staff number into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `writer` fails.
pub fn write_clef_xml_with_number<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    clef: &ClefSpec,
    number: Option<u8>,
) -> fmt::Result {
    let mut num_buf = [0_u8; 40];
    let num_str = number.map(|n| XmlWriter::<W>::format_u128(u128::from(n), &mut num_buf));
    match (num_str, clef.print_object) {
        (Some(num), true) => {
            writer.open_tag_with_attrs("clef", &[("number", num)])?;
        }
        (Some(num), false) => {
            writer.open_tag_with_attrs("clef", &[("number", num), ("print-object", "no")])?;
        }
        (None, true) => {
            writer.open_tag("clef")?;
        }
        (None, false) => {
            writer.open_tag_with_attrs("clef", &[("print-object", "no")])?;
        }
    }
    writer.text_element("sign", clef.sign)?;
    if let Some(line) = clef.line {
        writer.display_element("line", line)?;
    }
    if let Some(oct) = clef.octave_change
        && oct != 0
    {
        writer.display_element("clef-octave-change", oct)?;
    }
    writer.close_tag("clef")
}

/// Writes a `MusicXML` 4.0 `<clef>` element into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `writer` fails.
pub fn write_clef_xml<W: fmt::Write>(writer: &mut XmlWriter<W>, clef: &ClefSpec) -> fmt::Result {
    write_clef_xml_with_number(writer, clef, None)
}

/// Writes a `MusicXML` 4.0 `<transpose>` element into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `writer` fails.
pub fn write_transpose_xml<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    transpose: &TransposeSpec,
) -> fmt::Result {
    writer.open_tag("transpose")?;
    writer.display_element("chromatic", transpose.chromatic)?;
    if let Some(oct) = transpose.octave_change {
        writer.display_element("octave-change", oct)?;
    }
    writer.close_tag("transpose")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpecialKey {
    None,
    Percussion,
    BagpipeHpUpper,
    BagpipeHpMixed,
}

fn detect_special_key_token(mode_token: &str, raw_source_slice: Option<&str>) -> SpecialKey {
    for candidate in [Some(mode_token), raw_source_slice.map(extract_k_body)]
        .into_iter()
        .flatten()
    {
        let first_word = candidate.split_whitespace().next().unwrap_or("");
        if first_word == "HP" {
            return SpecialKey::BagpipeHpUpper;
        }
        if first_word == "Hp" {
            return SpecialKey::BagpipeHpMixed;
        }
        if first_word.eq_ignore_ascii_case("perc") {
            return SpecialKey::Percussion;
        }
        if first_word.eq_ignore_ascii_case("none") {
            return SpecialKey::None;
        }
    }
    SpecialKey::None
}

fn extract_k_body(raw: &str) -> &str {
    let trimmed = raw.trim();
    let trimmed = trimmed.strip_prefix('[').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix(']').unwrap_or(trimmed);
    trimmed
        .find(':')
        .map_or(trimmed, |idx| trimmed[idx + 1..].trim_start())
}

fn extract_mode_token(key: &KeySignature<String>) -> (&str, &[FieldParameter<String>]) {
    if !key.mode.is_empty() {
        return (&key.mode, &key.parameters);
    }
    if let Some(first) = key.parameters.first()
        && first.name.is_none()
        && is_mode_or_exp_keyword(&first.value)
    {
        return (&first.value, &key.parameters[1..]);
    }
    ("", &key.parameters)
}

fn is_mode_or_exp_keyword(token: &str) -> bool {
    let lower = token.trim().to_ascii_lowercase();
    lower == "m"
        || lower == "exp"
        || lower.starts_with("maj")
        || lower.starts_with("min")
        || lower.starts_with("ion")
        || lower.starts_with("aeo")
        || lower.starts_with("dor")
        || lower.starts_with("phr")
        || lower.starts_with("lyd")
        || lower.starts_with("mix")
        || lower.starts_with("loc")
}

fn canonical_musicxml_mode(mode: &str) -> &'static str {
    let lower = mode.trim().to_ascii_lowercase();
    if lower.is_empty() || lower.starts_with("maj") {
        "major"
    } else if lower == "m" || lower.starts_with("min") {
        "minor"
    } else if lower.starts_with("ion") {
        "ionian"
    } else if lower.starts_with("aeo") {
        "aeolian"
    } else if lower.starts_with("dor") {
        "dorian"
    } else if lower.starts_with("phr") {
        "phrygian"
    } else if lower.starts_with("lyd") {
        "lydian"
    } else if lower.starts_with("mix") {
        "mixolydian"
    } else if lower.starts_with("loc") {
        "locrian"
    } else {
        "major"
    }
}

fn fifths_to_steps(fifths: i8) -> Vec<(PitchClass, Offset)> {
    const SHARPS: [PitchClass; 7] = [
        PitchClass::F,
        PitchClass::C,
        PitchClass::G,
        PitchClass::D,
        PitchClass::A,
        PitchClass::E,
        PitchClass::B,
    ];
    const FLATS: [PitchClass; 7] = [
        PitchClass::B,
        PitchClass::E,
        PitchClass::A,
        PitchClass::D,
        PitchClass::G,
        PitchClass::C,
        PitchClass::F,
    ];

    let count = usize::from(fifths.unsigned_abs()).min(7);
    match fifths.cmp(&0) {
        std::cmp::Ordering::Greater => SHARPS[..count]
            .iter()
            .map(|&step| (step, Offset::integer(1)))
            .collect(),
        std::cmp::Ordering::Less => FLATS[..count]
            .iter()
            .map(|&step| (step, Offset::integer(-1)))
            .collect(),
        std::cmp::Ordering::Equal => Vec::new(),
    }
}

fn collect_explicit_key_accidentals(
    parameters: &[FieldParameter<String>],
) -> Vec<(PitchClass, Offset, Option<AccidentalValue>)> {
    let mut out = Vec::new();
    for param in parameters {
        if param.name.is_none()
            && let Some((class, offset)) = explicit_key_accidental(&param.value)
        {
            let acc_val = offset_to_accidental_value(offset);
            let explicit_acc = match acc_val {
                AccidentalValue::Natural
                | AccidentalValue::QuarterSharp
                | AccidentalValue::QuarterFlat
                | AccidentalValue::ThreeQuartersSharp
                | AccidentalValue::ThreeQuartersFlat
                | AccidentalValue::Other => Some(acc_val),
                AccidentalValue::Sharp
                | AccidentalValue::DoubleSharp
                | AccidentalValue::Flat
                | AccidentalValue::FlatFlat => None,
            };
            out.push((class, offset, explicit_acc));
        }
    }
    out
}

fn parse_clef_token(raw: &str) -> Option<ClefSpec> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (base, octave_change) = if let Some(prefix) = trimmed.strip_suffix("+8") {
        (prefix, Some(1_i8))
    } else if let Some(prefix) = trimmed.strip_suffix("-8") {
        (prefix, Some(-1_i8))
    } else if let Some(prefix) = trimmed.strip_suffix("+15") {
        (prefix, Some(2_i8))
    } else if let Some(prefix) = trimmed.strip_suffix("-15") {
        (prefix, Some(-2_i8))
    } else {
        (trimmed, None)
    };

    // Optional trailing line digit (1..=5), e.g. `bass3`, `alto1`, `alto2`, `C3`, `F4`, `G2`.
    let (name, explicit_line) = if let Some(last_byte) = base.as_bytes().last()
        && (b'1'..=b'5').contains(last_byte)
        && base.len() > 1
    {
        (&base[..base.len() - 1], Some(last_byte - b'0'))
    } else {
        (base, None)
    };

    let lower = name.to_ascii_lowercase();
    let (sign, default_line, print_object) = match lower.as_str() {
        "treble" | "g" => ("G", Some(2), true),
        "bass" | "f" => ("F", Some(4), true),
        "baritone" => ("F", Some(3), true),
        "tenor" => ("C", Some(4), true),
        "alto" | "c" => ("C", Some(3), true),
        "mezzosoprano" => ("C", Some(2), true),
        "soprano" => ("C", Some(1), true),
        "perc" | "percussion" => ("percussion", None, true),
        "none" => ("G", Some(2), false),
        _ => return None,
    };

    Some(ClefSpec {
        sign,
        line: explicit_line.or(default_line),
        octave_change,
        print_object,
    })
}

fn apply_middle_pitch(clef: &mut ClefSpec, middle: Pitch) {
    let target_step = i16::from(middle.octave) * 7 + i16::from(class_number(middle.class));
    // Default 3rd-line diatonic step (where C4 = 0, B4 = 6, D3 = -6, A3 = -2):
    let default_middle_step = match (clef.sign, clef.line) {
        ("G", Some(line)) => 6 - (i16::from(line) - 2) * 2,
        ("C", Some(line)) => -(i16::from(line) - 3) * 2,
        ("F", Some(line)) => -6 - (i16::from(line) - 4) * 2,
        _ => return,
    };

    let delta = target_step - default_middle_step;
    if delta == 0 {
        return;
    }
    if delta % 7 == 0 {
        let oct_shift = i8::try_from(delta / 7).unwrap_or(0);
        let new_oct = clef.octave_change.unwrap_or(0).saturating_add(oct_shift);
        clef.octave_change = (new_oct != 0).then_some(new_oct);
    } else if delta % 2 == 0
        && let Some(line) = clef.line
    {
        let new_line = i16::from(line) - (delta / 2);
        if (1..=5).contains(&new_line) {
            clef.line = u8::try_from(new_line).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use abc_parser::Accidental;
    use abc_parser::FieldValue;
    use abc_parser::Fraction;
    use abc_parser::KeySignature;
    use abc_parser::Pitch;
    use abc_parser::PitchClass;
    use abc_parser::parse_field;
    use abc_parser::pitch::Offset;
    use abc_parser::pitch::class_index;

    use super::AccidentalPropagation;
    use super::AccidentalValue;
    use super::ClefSpec;
    use super::KeyKind;
    use super::MeasureAccidentalState;
    use super::StemPolicy;
    use super::TransposeSpec;
    use super::format_alter;
    use super::parse_propagate_accidentals;
    use super::resolve_key_signature;
    use super::write_clef_xml;
    use super::write_key_xml;
    use super::write_transpose_xml;
    use crate::writer::XmlWriter;

    fn parse_k(source: &str) -> KeySignature<String> {
        let field = parse_field(source).unwrap();
        let FieldValue::Key(key) = field.value else {
            panic!("expected K: field");
        };
        key
    }

    #[test]
    fn resolves_modal_bagpipe_and_modified_key_signatures() {
        // Space-separated modal key: `K:G mixolydian` -> 0 fifths, mode "mixolydian".
        let g_mix = resolve_key_signature(&parse_k("K:G mixolydian"), Some("K:G mixolydian"));
        assert_eq!(
            g_mix.kind,
            KeyKind::Traditional {
                fifths: 0,
                mode: "mixolydian"
            }
        );

        // Bagpipe `K:HP` vs `K:Hp`: both set F# and C# playback offsets and Bagpipe stems,
        // while `K:HP` prints no key and `K:Hp` emits non-traditional F# C# G=.
        let hp_upper = resolve_key_signature(&parse_k("K:HP"), Some("K:HP"));
        assert_eq!(hp_upper.kind, KeyKind::None);
        assert_eq!(hp_upper.stem_policy, StemPolicy::Bagpipe);
        assert_eq!(
            hp_upper.offsets[class_index(PitchClass::F)],
            Offset::integer(1)
        );
        assert_eq!(
            hp_upper.offsets[class_index(PitchClass::C)],
            Offset::integer(1)
        );

        let hp_mixed = resolve_key_signature(&parse_k("K:Hp"), Some("K:Hp"));
        assert_eq!(hp_mixed.stem_policy, StemPolicy::Bagpipe);
        let mut writer = XmlWriter::new(String::new(), 0);
        write_key_xml(&mut writer, &hp_mixed.kind).unwrap();
        assert_eq!(
            writer.into_inner(),
            concat!(
                "<key>\n",
                "<key-step>F</key-step>\n<key-alter>1</key-alter>\n",
                "<key-step>C</key-step>\n<key-alter>1</key-alter>\n",
                "<key-step>G</key-step>\n<key-alter>0</key-alter>\n<key-accidental>natural</key-accidental>\n",
                "</key>\n",
            )
        );

        // Modified key `K:D =c`: retains F# from D major and overrides C to natural.
        let d_nat_c = resolve_key_signature(&parse_k("K:D =c"), Some("K:D =c"));
        assert_eq!(
            d_nat_c.offsets[class_index(PitchClass::F)],
            Offset::integer(1)
        );
        assert_eq!(d_nat_c.offsets[class_index(PitchClass::C)], Offset::ZERO);

        // Explicit key `K:D exp _b _e ^f`: ignores D major's C# and sets Bb, Eb, F#.
        let d_exp = resolve_key_signature(&parse_k("K:D exp _b _e ^f"), Some("K:D exp _b _e ^f"));
        assert_eq!(d_exp.offsets[class_index(PitchClass::C)], Offset::ZERO);
        assert_eq!(
            d_exp.offsets[class_index(PitchClass::B)],
            Offset::integer(-1)
        );
        assert_eq!(
            d_exp.offsets[class_index(PitchClass::E)],
            Offset::integer(-1)
        );
        assert_eq!(
            d_exp.offsets[class_index(PitchClass::F)],
            Offset::integer(1)
        );
    }

    #[test]
    fn tracks_measure_accidental_propagation_cross_octave_and_microtones() {
        let mut key_offsets = [Offset::ZERO; 7];
        key_offsets[class_index(PitchClass::F)] = Offset::integer(1); // G major (F#)

        let mut state = MeasureAccidentalState::new();
        let prop = parse_propagate_accidentals("pitch").unwrap();
        assert_eq!(prop, AccidentalPropagation::Pitch);

        // 1. Written quarter-sharp on F4 (`^/F`): emits `quarter-sharp` and `0.5` alter.
        let f4_qsharp = state.resolve_pitch(
            Pitch {
                class: PitchClass::F,
                octave: 0,
                accidental: Some(Accidental::Sharp(Fraction {
                    numerator: 1,
                    denominator: 2,
                })),
            },
            0,
            &key_offsets,
            prop,
            None,
        );
        assert_eq!(f4_qsharp.octave, 4);
        assert_eq!(format_alter(f4_qsharp.alter).as_deref(), Some("0.5"));
        assert_eq!(
            f4_qsharp.written_accidental,
            Some(AccidentalValue::QuarterSharp)
        );

        // 2. Unwritten F4 in same octave: inherits `0.5` alter without re-emitting `<accidental>`.
        let f4_repeat = state.resolve_pitch(
            Pitch {
                class: PitchClass::F,
                octave: 0,
                accidental: None,
            },
            0,
            &key_offsets,
            prop,
            None,
        );
        assert_eq!(f4_repeat.alter, Offset::new(1, 2));
        assert_eq!(f4_repeat.written_accidental, None);

        // 3. Unwritten F5 (`f`) in a different octave under `Pitch` propagation:
        //    inherits `0.5` alter AND emits explicit `<accidental>quarter-sharp</accidental>`.
        let f5_cross = state.resolve_pitch(
            Pitch {
                class: PitchClass::F,
                octave: 1,
                accidental: None,
            },
            0,
            &key_offsets,
            prop,
            None,
        );
        assert_eq!(f5_cross.octave, 5);
        assert_eq!(f5_cross.alter, Offset::new(1, 2));
        assert_eq!(
            f5_cross.written_accidental,
            Some(AccidentalValue::QuarterSharp)
        );

        // 4. After barline clear, unwritten F4 reverts to key signature (F#, alter +1),
        //    unless tied across the barline (`tied_alter: Some(-1)`).
        state.clear();
        let f4_tied = state.resolve_pitch(
            Pitch {
                class: PitchClass::F,
                octave: 0,
                accidental: None,
            },
            0,
            &key_offsets,
            prop,
            Some(Offset::integer(-1)),
        );
        assert_eq!(f4_tied.alter, Offset::integer(-1));
        assert_eq!(f4_tied.written_accidental, None);
    }

    #[test]
    fn resolves_alternating_octaves_in_pitch_mode_and_natural_cancellation_in_not_mode() {
        let key_offsets = [Offset::ZERO; 7];
        let c4_sharp = Pitch {
            class: PitchClass::C,
            octave: 0,
            accidental: Some(Accidental::Sharp(Fraction {
                numerator: 1,
                denominator: 1,
            })),
        };
        let c4 = Pitch {
            class: PitchClass::C,
            octave: 0,
            accidental: None,
        };
        let c5 = Pitch {
            class: PitchClass::C,
            octave: 1,
            accidental: None,
        };

        let mut pitch_state = MeasureAccidentalState::new();
        assert_eq!(
            pitch_state
                .resolve_pitch(
                    c4_sharp,
                    0,
                    &key_offsets,
                    AccidentalPropagation::Pitch,
                    None
                )
                .written_accidental,
            Some(AccidentalValue::Sharp)
        );
        assert_eq!(
            pitch_state
                .resolve_pitch(c5, 0, &key_offsets, AccidentalPropagation::Pitch, None)
                .written_accidental,
            Some(AccidentalValue::Sharp)
        );
        assert_eq!(
            pitch_state
                .resolve_pitch(c4, 0, &key_offsets, AccidentalPropagation::Pitch, None)
                .written_accidental,
            None
        );
        assert_eq!(
            pitch_state
                .resolve_pitch(c5, 0, &key_offsets, AccidentalPropagation::Pitch, None)
                .written_accidental,
            None
        );

        let mut not_state = MeasureAccidentalState::new();
        let first =
            not_state.resolve_pitch(c4_sharp, 0, &key_offsets, AccidentalPropagation::Not, None);
        assert_eq!(first.alter, Offset::integer(1));
        assert_eq!(first.written_accidental, Some(AccidentalValue::Sharp));

        let second = not_state.resolve_pitch(c4, 0, &key_offsets, AccidentalPropagation::Not, None);
        assert_eq!(second.alter, Offset::ZERO);
        assert_eq!(second.written_accidental, Some(AccidentalValue::Natural));
    }

    #[test]
    fn parses_clef_middle_transpose_and_octave_parameters() {
        // `K:bass middle=d transpose=-24 octave=-1`
        let resolved =
            resolve_key_signature(&parse_k("K:bass middle=d transpose=-24 octave=-1"), None);
        assert_eq!(
            resolved.parameters.clef,
            Some(ClefSpec {
                sign: "F",
                line: Some(4),
                octave_change: Some(2),
                print_object: true,
            })
        );
        assert_eq!(
            resolved.parameters.transpose,
            Some(TransposeSpec {
                chromatic: 0,
                octave_change: Some(-2),
            })
        );
        assert_eq!(resolved.parameters.octave_shift, Some(-1));

        // `K:Am transpose=-2 clef=none`: `<octave-change>` omitted when |semitones| < 12;
        // `clef=none` emits `print-object="no"`.
        let am_trans = resolve_key_signature(&parse_k("K:Am transpose=-2 clef=none"), None);
        let mut writer = XmlWriter::new(String::new(), 0);
        write_clef_xml(&mut writer, &am_trans.parameters.clef.unwrap()).unwrap();
        write_transpose_xml(&mut writer, &am_trans.parameters.transpose.unwrap()).unwrap();
        assert_eq!(
            writer.into_inner(),
            concat!(
                "<clef print-object=\"no\">\n<sign>G</sign>\n<line>2</line>\n</clef>\n",
                "<transpose>\n<chromatic>-2</chromatic>\n</transpose>\n",
            )
        );
    }
}
