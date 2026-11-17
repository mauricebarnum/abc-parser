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

//! Chord-symbol parsing and `MusicXML` 4.0 `<harmony>` emission.
//!
//! Converts ABC 2.1 chord symbols (such as `"Cm7/G"`, `"F#m7b5"`, or `"G7(b9,#11)"`)
//! into structured [`HarmonySpec`] values and `<harmony>` elements, using
//! [`abc_parser::chord`] for root, accidental, and slash-bass recognition.

use std::fmt;

use abc_parser::PitchClass;
use abc_parser::chord::parse_accidental;
use abc_parser::chord::parse_bass_note;
use abc_parser::chord::starts_with_chord_root;

use crate::pitch::pitch_step_str;
use crate::writer::XmlWriter;

/// Type of a `<degree>` modification within a `<harmony>` element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DegreeType {
    /// `<degree-type>add</degree-type>`.
    Add,
    /// `<degree-type>alter</degree-type>`.
    Alter,
    /// `<degree-type>subtract</degree-type>`.
    Subtract,
}

impl DegreeType {
    /// Returns the canonical `MusicXML` `<degree-type>` text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Alter => "alter",
            Self::Subtract => "subtract",
        }
    }
}

/// A single `<degree>` alteration, addition, or omission on a chord.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DegreeSpec {
    /// Scale degree number (`2`, `3`, `4`, `5`, `6`, `7`, `9`, `11`, or `13`).
    pub value: u8,
    /// Semitone alteration (`-1` for flat, `0` for natural, `1` for sharp).
    pub alter: i8,
    /// Modification operation (`add`, `alter`, or `subtract`).
    pub degree_type: DegreeType,
}

/// A slash-bass note specification for `<bass>`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BassSpec {
    /// Diatonic step of the bass note.
    pub step: PitchClass,
    /// Optional chromatic alteration in semitones (`-1`, `0`, `1`).
    pub alter: Option<i8>,
}

/// Resolved `MusicXML` 4.0 `<harmony>` specification for an ABC chord symbol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarmonySpec {
    /// Diatonic step of the chord root.
    pub root_step: PitchClass,
    /// Optional chromatic alteration of the root in semitones (`-1`, `0`, `1`).
    pub root_alter: Option<i8>,
    /// Whether the `<root-step>` is suppressed via `text=""` (used for `"N.C."`).
    pub suppress_root_display: bool,
    /// Canonical `MusicXML` `<kind>` value (e.g. `"major"`, `"minor-seventh"`, `"dominant"`).
    pub kind: &'static str,
    /// Display text for the `text` attribute on `<kind>`.
    pub kind_text: String,
    /// Optional slash-bass note (`<bass>`).
    pub bass: Option<BassSpec>,
    /// Optional `<degree>` modifications.
    pub degrees: Vec<DegreeSpec>,
}

/// Parses an ABC chord-symbol string into a [`HarmonySpec`].
///
/// Returns `None` if `input` is not a valid chord symbol (for example, free text
/// such as `"Slowly"` or `"Fine"`), allowing the caller to fall back to a
/// `<direction><direction-type><words>` element per Decision 8.
#[must_use]
pub fn parse_chord_symbol(input: &str) -> Option<HarmonySpec> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.eq_ignore_ascii_case("N.C.") || trimmed.eq_ignore_ascii_case("NC") {
        return Some(HarmonySpec {
            root_step: PitchClass::C,
            root_alter: None,
            suppress_root_display: true,
            kind: "none",
            kind_text: trimmed.to_owned(),
            bass: None,
            degrees: Vec::new(),
        });
    }

    if !starts_with_chord_root(trimmed) {
        return None;
    }

    let mut chars = trimmed.chars();
    let root_ch = chars.next()?;
    let root_step = char_to_pitch_class(root_ch)?;
    let after_root_letter = chars.as_str();

    let (acc_len, acc_opt) = parse_accidental(after_root_letter);
    let root_alter = acc_opt.map(str_to_alter);
    let mut rest = &after_root_letter[acc_len..];

    // Extract a trailing slash-bass note (`/G`, `/F#`, `/Bb`) if present at the end.
    let mut bass = None;
    if let Some(slash_idx) = rest.rfind('/') {
        let candidate = &rest[slash_idx + 1..];
        if let Some((bass_len, (bass_ch, bass_acc))) = parse_bass_note(candidate)
            && bass_len == candidate.len()
            && let Some(bass_step) = char_to_pitch_class(bass_ch)
        {
            bass = Some(BassSpec {
                step: bass_step,
                alter: bass_acc.map(str_to_alter),
            });
            rest = &rest[..slash_idx];
        }
    }

    let kind_text = rest.to_owned();
    let (kind, mut degrees, tail) = parse_base_kind(rest);
    parse_degree_tail(tail, &mut degrees)?;

    Some(HarmonySpec {
        root_step,
        root_alter,
        suppress_root_display: false,
        kind,
        kind_text,
        bass,
        degrees,
    })
}

/// Emits a `<harmony>` element for `spec` into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to the underlying buffer fails.
pub fn write_harmony_xml<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    spec: &HarmonySpec,
) -> fmt::Result {
    writer.open_tag("harmony")?;

    writer.open_tag("root")?;
    if spec.suppress_root_display {
        writer.text_element_with_attrs(
            "root-step",
            &[("text", "")],
            pitch_step_str(spec.root_step),
        )?;
    } else {
        writer.text_element("root-step", pitch_step_str(spec.root_step))?;
    }
    if let Some(alter) = spec.root_alter {
        writer.display_element("root-alter", alter)?;
    }
    writer.close_tag("root")?;

    if spec.kind_text.is_empty() {
        writer.text_element("kind", spec.kind)?;
    } else {
        writer.text_element_with_attrs("kind", &[("text", &spec.kind_text)], spec.kind)?;
    }

    if let Some(bass) = spec.bass {
        writer.open_tag("bass")?;
        writer.text_element("bass-step", pitch_step_str(bass.step))?;
        if let Some(alter) = bass.alter {
            writer.display_element("bass-alter", alter)?;
        }
        writer.close_tag("bass")?;
    }

    for degree in &spec.degrees {
        writer.open_tag("degree")?;
        writer.display_element("degree-value", degree.value)?;
        writer.display_element("degree-alter", degree.alter)?;
        writer.text_element("degree-type", degree.degree_type.as_str())?;
        writer.close_tag("degree")?;
    }

    writer.close_tag("harmony")
}

const fn char_to_pitch_class(ch: char) -> Option<PitchClass> {
    match ch.to_ascii_uppercase() {
        'C' => Some(PitchClass::C),
        'D' => Some(PitchClass::D),
        'E' => Some(PitchClass::E),
        'F' => Some(PitchClass::F),
        'G' => Some(PitchClass::G),
        'A' => Some(PitchClass::A),
        'B' => Some(PitchClass::B),
        _ => None,
    }
}

fn str_to_alter(s: &str) -> i8 {
    match s {
        "##" => 2,
        "#" | "♯" => 1,
        "bb" => -2,
        "b" | "♭" => -1,
        _ => 0,
    }
}

#[allow(clippy::too_many_lines)]
fn parse_base_kind(s: &str) -> (&'static str, Vec<DegreeSpec>, &str) {
    // Order from longest/most specific prefix to shortest so e.g. "maj7" matches before "m" or "maj".
    const TABLE: &[(&str, &str)] = &[
        // Half-diminished
        ("min7(b5)", "half-diminished"),
        ("min7b5", "half-diminished"),
        ("m7(b5)", "half-diminished"),
        ("m7b5", "half-diminished"),
        ("-7(b5)", "half-diminished"),
        ("-7b5", "half-diminished"),
        ("ø7", "half-diminished"),
        ("ø", "half-diminished"),
        ("07", "half-diminished"),
        // Major-minor seventh
        ("min(maj9)", "major-minor"),
        ("m(maj9)", "major-minor"),
        ("m/maj9", "major-minor"),
        ("m/maj7", "major-minor"),
        ("min(maj7)", "major-minor"),
        ("m(maj7)", "major-minor"),
        ("m(M7)", "major-minor"),
        ("mMaj7", "major-minor"),
        ("minMaj7", "major-minor"),
        ("mM7", "major-minor"),
        ("-maj7", "major-minor"),
        ("-(M7)", "major-minor"),
        ("-M7", "major-minor"),
        // Major 13th / 11th / 9th / 7th / 6th
        ("maj13", "major-13th"),
        ("Maj13", "major-13th"),
        ("M13", "major-13th"),
        ("^13", "major-13th"),
        ("Δ13", "major-13th"),
        ("maj11", "major-11th"),
        ("Maj11", "major-11th"),
        ("M11", "major-11th"),
        ("^11", "major-11th"),
        ("Δ11", "major-11th"),
        ("maj9", "major-ninth"),
        ("Maj9", "major-ninth"),
        ("M9", "major-ninth"),
        ("^9", "major-ninth"),
        ("Δ9", "major-ninth"),
        ("maj7", "major-seventh"),
        ("Maj7", "major-seventh"),
        ("ma7", "major-seventh"),
        ("M7", "major-seventh"),
        ("^7", "major-seventh"),
        ("Δ7", "major-seventh"),
        ("Δ", "major-seventh"),
        ("maj6", "major-sixth"),
        ("Maj6", "major-sixth"),
        ("M6", "major-sixth"),
        // Minor 13th / 11th / 9th / 7th / 6th
        ("min13", "minor-13th"),
        ("m13", "minor-13th"),
        ("-13", "minor-13th"),
        ("min11", "minor-11th"),
        ("m11", "minor-11th"),
        ("-11", "minor-11th"),
        ("min9", "minor-ninth"),
        ("m9", "minor-ninth"),
        ("-9", "minor-ninth"),
        ("min7", "minor-seventh"),
        ("m7", "minor-seventh"),
        ("-7", "minor-seventh"),
        ("min6", "minor-sixth"),
        ("m6", "minor-sixth"),
        ("-6", "minor-sixth"),
        // Diminished
        ("diminished", "diminished"),
        ("dim7", "diminished-seventh"),
        ("o7", "diminished-seventh"),
        ("°7", "diminished-seventh"),
        ("dim", "diminished"),
        ("°", "diminished"),
        ("o", "diminished"),
        ("0", "diminished"),
        // Augmented
        ("augmented", "augmented"),
        ("aug7", "augmented-seventh"),
        ("+7", "augmented-seventh"),
        ("7#5", "augmented-seventh"),
        ("7(#5)", "augmented-seventh"),
        ("7+5", "augmented-seventh"),
        ("7+", "augmented-seventh"),
        ("aug", "augmented"),
        ("+", "augmented"),
        // Suspended
        ("sus4", "suspended-fourth"),
        ("sus2", "suspended-second"),
        ("sus", "suspended-fourth"),
        // Dominant 13 / 11 / 9 / 7
        ("13", "dominant-13th"),
        ("11", "dominant-11th"),
        ("9", "dominant-ninth"),
        ("7alt", "dominant"),
        ("7", "dominant"),
        // Sixths
        ("6", "major-sixth"),
        // Power
        ("5", "power"),
        // Triads & Altered
        ("major", "major"),
        ("minor", "minor"),
        ("alt", "dominant"),
        ("maj", "major"),
        ("Maj", "major"),
        ("min", "minor"),
        ("m", "minor"),
        ("-", "minor"),
        ("M", "major"),
    ];

    for &(prefix, kind) in TABLE {
        if let Some(tail) = s.strip_prefix(prefix) {
            let mut degrees = Vec::new();
            // Handle "6/9" or "69" right after a sixth chord.
            if matches!(kind, "major-sixth" | "minor-sixth")
                && let Some(after_9) = tail.strip_prefix("/9").or_else(|| tail.strip_prefix('9'))
            {
                degrees.push(DegreeSpec {
                    value: 9,
                    alter: 0,
                    degree_type: DegreeType::Add,
                });
                return (kind, degrees, after_9);
            }
            return (kind, degrees, tail);
        }
    }

    ("major", Vec::new(), s)
}

fn parse_degree_tail(mut s: &str, degrees: &mut Vec<DegreeSpec>) -> Option<()> {
    while !s.is_empty() {
        // Strip leading separators or parentheses
        if let Some(rest) = s.strip_prefix(['(', ')', ',', ' ']) {
            s = rest;
            continue;
        }

        // Check for "add"
        if let Some(after_add) = s.strip_prefix("add") {
            let (alter, after_alt) = parse_degree_alter(after_add);
            let (deg_val, rest) = parse_degree_number(after_alt)?;
            degrees.push(DegreeSpec {
                value: deg_val,
                alter,
                degree_type: DegreeType::Add,
            });
            s = rest;
            continue;
        }

        // Check for "no" / "omit"
        if let Some(after_omit) = s.strip_prefix("no").or_else(|| s.strip_prefix("omit")) {
            let (deg_val, rest) = parse_degree_number(after_omit)?;
            degrees.push(DegreeSpec {
                value: deg_val,
                alter: 0,
                degree_type: DegreeType::Subtract,
            });
            s = rest;
            continue;
        }

        // Check for trailing "sus4", "sus2", "sus" (e.g. "7sus4")
        if let Some(rest) = s.strip_prefix("sus4").or_else(|| s.strip_prefix("sus")) {
            degrees.push(DegreeSpec {
                value: 4,
                alter: 0,
                degree_type: DegreeType::Add,
            });
            s = rest;
            continue;
        }
        if let Some(rest) = s.strip_prefix("sus2") {
            degrees.push(DegreeSpec {
                value: 2,
                alter: 0,
                degree_type: DegreeType::Add,
            });
            s = rest;
            continue;
        }
        if let Some(rest) = s.strip_prefix("alt") {
            s = rest;
            continue;
        }

        // Check for altered degree: `b5`, `#5`, `-5`, `+5`, `b9`, `#9`, `#11`, `b13`, etc.
        let (alter, after_alt) = parse_degree_alter(s);
        if alter != 0
            && let Some((deg_val, rest)) = parse_degree_number(after_alt)
        {
            degrees.push(DegreeSpec {
                value: deg_val,
                alter,
                degree_type: DegreeType::Alter,
            });
            s = rest;
            continue;
        }

        // Unrecognized trailing text means this is not a valid chord symbol.
        return None;
    }

    Some(())
}

fn parse_degree_alter(s: &str) -> (i8, &str) {
    if let Some(rest) = s
        .strip_prefix('#')
        .or_else(|| s.strip_prefix('\u{266F}'))
        .or_else(|| s.strip_prefix('+'))
    {
        (1, rest)
    } else if let Some(rest) = s
        .strip_prefix('b')
        .or_else(|| s.strip_prefix('\u{266D}'))
        .or_else(|| s.strip_prefix('-'))
    {
        (-1, rest)
    } else {
        (0, s)
    }
}

fn parse_degree_number(s: &str) -> Option<(u8, &str)> {
    for &(prefix, num) in &[
        ("13", 13_u8),
        ("11", 11_u8),
        ("9", 9_u8),
        ("7", 7_u8),
        ("6", 6_u8),
        ("5", 5_u8),
        ("4", 4_u8),
        ("3", 3_u8),
        ("2", 2_u8),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some((num, rest));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use abc_parser::PitchClass;

    use super::BassSpec;
    use super::DegreeSpec;
    use super::DegreeType;
    use super::parse_chord_symbol;
    use super::write_harmony_xml;
    use crate::writer::XmlWriter;

    #[test]
    fn parses_triads_sevenths_slash_bass_and_altered_degrees() {
        let cm7_g = parse_chord_symbol("Cm7/G").unwrap();
        assert_eq!(cm7_g.root_step, PitchClass::C);
        assert_eq!(cm7_g.root_alter, None);
        assert_eq!(cm7_g.kind, "minor-seventh");
        assert_eq!(cm7_g.kind_text, "m7");
        assert_eq!(
            cm7_g.bass,
            Some(BassSpec {
                step: PitchClass::G,
                alter: None,
            })
        );

        let mut buf = String::new();
        let mut writer = XmlWriter::new(&mut buf, 2);
        write_harmony_xml(&mut writer, &cm7_g).unwrap();
        assert_eq!(
            buf,
            concat!(
                "<harmony>\n",
                "  <root>\n",
                "    <root-step>C</root-step>\n",
                "  </root>\n",
                "  <kind text=\"m7\">minor-seventh</kind>\n",
                "  <bass>\n",
                "    <bass-step>G</bass-step>\n",
                "  </bass>\n",
                "</harmony>\n",
            )
        );

        let alt_dom = parse_chord_symbol("F#7(b9,#11)/C#").unwrap();
        assert_eq!(alt_dom.root_step, PitchClass::F);
        assert_eq!(alt_dom.root_alter, Some(1));
        assert_eq!(alt_dom.kind, "dominant");
        assert_eq!(
            alt_dom.bass,
            Some(BassSpec {
                step: PitchClass::C,
                alter: Some(1),
            })
        );
        assert_eq!(
            alt_dom.degrees,
            vec![
                DegreeSpec {
                    value: 9,
                    alter: -1,
                    degree_type: DegreeType::Alter,
                },
                DegreeSpec {
                    value: 11,
                    alter: 1,
                    degree_type: DegreeType::Alter,
                },
            ]
        );

        let six_nine = parse_chord_symbol("Bb6/9").unwrap();
        assert_eq!(six_nine.root_step, PitchClass::B);
        assert_eq!(six_nine.root_alter, Some(-1));
        assert_eq!(six_nine.kind, "major-sixth");
        assert_eq!(six_nine.bass, None);
        assert_eq!(
            six_nine.degrees,
            vec![DegreeSpec {
                value: 9,
                alter: 0,
                degree_type: DegreeType::Add,
            }]
        );
    }

    #[test]
    fn parses_word_kinds_slash_minor_major_and_altered_chords() {
        assert_eq!(parse_chord_symbol("Cmajor").unwrap().kind, "major");
        assert_eq!(parse_chord_symbol("Cminor").unwrap().kind, "minor");
        assert_eq!(
            parse_chord_symbol("Cdiminished").unwrap().kind,
            "diminished"
        );
        assert_eq!(parse_chord_symbol("Caugmented").unwrap().kind, "augmented");
        assert_eq!(parse_chord_symbol("Cm/maj7").unwrap().kind, "major-minor");
        assert_eq!(parse_chord_symbol("Cm/maj9").unwrap().kind, "major-minor");
        assert_eq!(
            parse_chord_symbol("Co7").unwrap().kind,
            "diminished-seventh"
        );
        assert_eq!(parse_chord_symbol("G7alt").unwrap().kind, "dominant");
        assert_eq!(parse_chord_symbol("Galt").unwrap().kind, "dominant");
    }

    #[test]
    fn rejects_non_chord_annotations_that_begin_with_note_letters() {
        assert_eq!(parse_chord_symbol("Slowly"), None);
        assert_eq!(parse_chord_symbol("Fine"), None);
        assert_eq!(parse_chord_symbol("Chorus"), None);
        assert_eq!(parse_chord_symbol("D.C. al Fine"), None);
        assert_eq!(parse_chord_symbol("Allegro"), None);
        assert_eq!(parse_chord_symbol(""), None);
    }
}
