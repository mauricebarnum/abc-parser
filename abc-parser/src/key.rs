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

//! Key signatures, circle-of-fifths calculation, and mode normalization.

use crate::ast::KeyAccidental;
use crate::ast::KeySignature;
use crate::ast::KeyTonic;
use crate::ast::PitchClass;
use crate::emit::ToAbc;
use crate::pitch::Offset;
use crate::pitch::accidental_offset;
use crate::pitch::chromatic_spelling;
use crate::pitch::class_index;
use crate::pitch::natural_semitone;
use crate::pitch::normalize_accidental;
use crate::pitch::parse_single_pitch;
use crate::pitch::shift_class;

/// Creates a key tonic from one sharp- or flat-oriented chromatic spelling.
#[must_use]
pub const fn chromatic_tonic(chromatic: u8, prefer_flats: bool) -> KeyTonic {
    let (class, accidental) = chromatic_spelling(chromatic, prefer_flats);
    KeyTonic {
        class,
        accidental: match accidental {
            -1 => Some(KeyAccidental::Flat),
            1 => Some(KeyAccidental::Sharp),
            _ => None,
        },
    }
}

/// Finds the shortest signed tonic interval, preferring upward tritones.
#[must_use]
pub const fn signed_tonic_interval(source: KeyTonic, destination: KeyTonic) -> i16 {
    let upward = (tonic_semitone(destination) - tonic_semitone(source)).rem_euclid(12);
    if upward > 6 { upward - 12 } else { upward }
}

/// Returns the chromatic semitone number of a key tonic.
#[must_use]
pub const fn tonic_semitone(tonic: KeyTonic) -> i16 {
    natural_semitone(tonic.class)
        + match tonic.accidental {
            Some(KeyAccidental::Sharp) => 1,
            Some(KeyAccidental::Flat) => -1,
            None => 0,
        }
}

/// Builds the seven implicit accidental offsets of a conventional key.
pub fn signature_offsets(key: &KeySignature<String>) -> Result<[Offset; 7], String> {
    let tonic = key
        .tonic
        .ok_or_else(|| "cannot resolve a key signature without a pitched tonic".to_owned())?;
    let fifths = conventional_key_fifths(tonic, &key.mode).ok_or_else(|| {
        format!(
            "key {} has no conventional seven-accidental signature",
            key.to_abc()
        )
    })?;
    let mut offsets = [Offset::ZERO; 7];
    let sharps = [
        PitchClass::F,
        PitchClass::C,
        PitchClass::G,
        PitchClass::D,
        PitchClass::A,
        PitchClass::E,
        PitchClass::B,
    ];
    let flats = [
        PitchClass::B,
        PitchClass::E,
        PitchClass::A,
        PitchClass::D,
        PitchClass::G,
        PitchClass::C,
        PitchClass::F,
    ];
    if fifths > 0 {
        for class in sharps.iter().take(fifths.unsigned_abs() as usize) {
            offsets[class_index(*class)] = Offset::integer(1);
        }
    } else {
        for class in flats.iter().take(fifths.unsigned_abs() as usize) {
            offsets[class_index(*class)] = Offset::integer(-1);
        }
    }
    for parameter in &key.parameters {
        if parameter.name.is_none()
            && let Some((class, offset)) = explicit_key_accidental(&parameter.value)
        {
            offsets[class_index(class)] = offset;
        }
    }
    Ok(offsets)
}

/// Returns whether a key's signature or explicit tonic spelling favors flats.
#[must_use]
pub fn key_prefers_flats(key: &KeySignature<String>) -> bool {
    key.tonic.is_some_and(|tonic| {
        conventional_key_fifths(tonic, &key.mode).map_or_else(
            || tonic.accidental == Some(KeyAccidental::Flat),
            |fifths| fifths < 0,
        )
    })
}

/// Finds a modal key's relative-major position on the conventional circle of fifths.
#[must_use]
pub fn conventional_key_fifths(tonic: KeyTonic, mode: &str) -> Option<i8> {
    let degree = mode_degree(mode);
    let major_class = shift_class(tonic.class, -degree);
    let major_pitch = (tonic_semitone(tonic) - mode_semitones(degree)).rem_euclid(12);
    let accidental = normalize_accidental(major_pitch - natural_semitone(major_class));
    major_key_fifths(major_class, accidental)
}

/// Parses a positional key parameter such as `^F`, `_B`, or `^1/2c`.
#[must_use]
pub fn explicit_key_accidental(value: &str) -> Option<(PitchClass, Offset)> {
    if !matches!(value.as_bytes().first(), Some(b'^' | b'_' | b'=')) {
        return None;
    }
    let pitch = parse_single_pitch(value)?;
    pitch
        .accidental
        .map(|accidental| (pitch.class, accidental_offset(accidental)))
}

/// Maps common ABC mode spellings to their scale degree above relative major.
#[must_use]
pub fn mode_degree(mode: &str) -> i8 {
    let mode = mode.trim().to_ascii_lowercase();
    if mode == "m" || mode.starts_with("min") || mode.starts_with("aeo") {
        5
    } else if mode.starts_with("dor") {
        1
    } else if mode.starts_with("phr") {
        2
    } else if mode.starts_with("lyd") {
        3
    } else if mode.starts_with("mix") {
        4
    } else if mode.starts_with("loc") {
        6
    } else {
        0
    }
}

/// Replaces a recognized ABC mode alias with its canonical short spelling.
pub fn normalize_key_mode(key: &mut KeySignature<String>) {
    let mode = key.mode.trim().to_ascii_lowercase();
    key.mode = if mode.is_empty() || mode.starts_with("maj") || mode.starts_with("ion") {
        String::new()
    } else if mode == "m" || mode.starts_with("min") || mode.starts_with("aeo") {
        "m".to_owned()
    } else if mode.starts_with("dor") {
        "dor".to_owned()
    } else if mode.starts_with("phr") {
        "phr".to_owned()
    } else if mode.starts_with("lyd") {
        "lyd".to_owned()
    } else if mode.starts_with("mix") {
        "mix".to_owned()
    } else if mode.starts_with("loc") {
        "loc".to_owned()
    } else {
        key.mode.clone()
    };
}

/// Returns the major-scale semitone distance for a diatonic degree.
#[must_use]
pub const fn mode_semitones(degree: i8) -> i16 {
    match degree {
        1 => 2,
        2 => 4,
        3 => 5,
        4 => 7,
        5 => 9,
        6 => 11,
        _ => 0,
    }
}

/// Maps a conventional written major tonic to its circle-of-fifths count.
#[must_use]
pub const fn major_key_fifths(class: PitchClass, accidental: i16) -> Option<i8> {
    match (class, accidental) {
        (PitchClass::C, -1) => Some(-7),
        (PitchClass::G, -1) => Some(-6),
        (PitchClass::D, -1) => Some(-5),
        (PitchClass::A, -1) => Some(-4),
        (PitchClass::E, -1) => Some(-3),
        (PitchClass::B, -1) => Some(-2),
        (PitchClass::F, 0) => Some(-1),
        (PitchClass::C, 0) => Some(0),
        (PitchClass::G, 0) => Some(1),
        (PitchClass::D, 0) => Some(2),
        (PitchClass::A, 0) => Some(3),
        (PitchClass::E, 0) => Some(4),
        (PitchClass::B, 0) => Some(5),
        (PitchClass::F, 1) => Some(6),
        (PitchClass::C, 1) => Some(7),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::conventional_key_fifths;
    use super::key_prefers_flats;
    use super::mode_degree;
    use super::normalize_key_mode;
    use super::signature_offsets;
    use crate::ast::KeySignature;
    use crate::ast::KeyTonic;
    use crate::ast::PitchClass;
    use crate::pitch::Offset;

    #[test]
    fn c_major_and_a_minor_fifths() {
        let c = KeyTonic {
            class: PitchClass::C,
            accidental: None,
        };
        assert_eq!(conventional_key_fifths(c, "").unwrap(), 0);
        let a = KeyTonic {
            class: PitchClass::A,
            accidental: None,
        };
        assert_eq!(conventional_key_fifths(a, "m").unwrap(), 0);
        assert_eq!(conventional_key_fifths(a, "aeolian").unwrap(), 0);
    }

    #[test]
    fn g_major_and_f_major_signatures() {
        let g_major = KeySignature {
            tonic: Some(KeyTonic {
                class: PitchClass::G,
                accidental: None,
            }),
            mode: String::new(),
            parameters: Vec::new(),
        };
        let offsets = signature_offsets(&g_major).unwrap();
        assert_eq!(
            offsets[crate::pitch::class_index(PitchClass::F)],
            Offset::integer(1)
        );
        assert_eq!(
            offsets[crate::pitch::class_index(PitchClass::C)],
            Offset::ZERO
        );
        assert!(!key_prefers_flats(&g_major));

        let f_major = KeySignature {
            tonic: Some(KeyTonic {
                class: PitchClass::F,
                accidental: None,
            }),
            mode: String::new(),
            parameters: Vec::new(),
        };
        let f_offsets = signature_offsets(&f_major).unwrap();
        assert_eq!(
            f_offsets[crate::pitch::class_index(PitchClass::B)],
            Offset::integer(-1)
        );
        assert!(key_prefers_flats(&f_major));
    }

    #[test]
    fn mode_normalization_and_degrees() {
        let mut key = KeySignature {
            tonic: Some(KeyTonic {
                class: PitchClass::D,
                accidental: None,
            }),
            mode: "Dorian".to_owned(),
            parameters: Vec::new(),
        };
        assert_eq!(mode_degree(&key.mode), 1);
        normalize_key_mode(&mut key);
        assert_eq!(key.mode, "dor");
    }
}
