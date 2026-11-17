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

//! Chord symbol recognition, root/accidental extraction, and extension validation.

/// List of recognized chord type tokens sorted so longer prefixes match first.
pub const CHORD_TYPE_TOKENS: &[&str] = &[
    "diminished",
    "augmented",
    "minor",
    "major",
    "m/maj7",
    "m/maj9",
    "m(maj7)",
    "m(maj9)",
    "add11",
    "add13",
    "omit3",
    "omit5",
    "sus4",
    "sus2",
    "add2",
    "add4",
    "add9",
    "maj7",
    "maj9",
    "maj11",
    "maj13",
    "min7",
    "min9",
    "min11",
    "min13",
    "dim7",
    "aug7",
    "sus",
    "min",
    "maj",
    "dim",
    "aug",
    "add",
    "alt",
    "no3",
    "no5",
    "6/9",
    "69",
    "11",
    "13",
    "#11",
    "b11",
    "+11",
    "♯11",
    "♭11",
    "#13",
    "b13",
    "-13",
    "♯13",
    "♭13",
    "#5",
    "b5",
    "+5",
    "-5",
    "♯5",
    "♭5",
    "#9",
    "b9",
    "+9",
    "-9",
    "♯9",
    "♭9",
    "o7",
    "°7",
    "ø7",
    "Δ7",
    "^7",
    "Δ9",
    "^9",
    "M7",
    "M9",
    "M11",
    "M13",
    "m7",
    "m9",
    "m11",
    "m13",
    "m6",
    "m2",
    "m4",
    "m5",
    "m8",
    "m",
    "M",
    "2",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "+",
    "-",
    "°",
    "ø",
    "0",
    "Δ",
    "^",
    "#",
    "b",
];

/// Returns whether the string begins with a valid chord symbol.
///
/// # Examples
///
/// ```
/// use abc_parser::chord::starts_with_chord_root;
///
/// assert!(starts_with_chord_root("Cmaj7"));
/// assert!(starts_with_chord_root("F#m7/A"));
/// assert!(!starts_with_chord_root("Hmaj7"));
/// ```
#[must_use]
pub fn starts_with_chord_root(s: &str) -> bool {
    let s = s.trim_start();
    let Some(first) = s.chars().next() else {
        return false;
    };
    if !matches!(first, 'A'..='G') {
        return false;
    }
    let after_root = &s[first.len_utf8()..];
    let (acc_len, _) = parse_accidental(after_root);
    let after_acc = &after_root[acc_len..];
    if let Some(slash_idx) = after_acc.rfind('/') {
        let before_slash = &after_acc[..slash_idx];
        let after_slash = &after_acc[slash_idx + 1..];
        if let Some((bass_len, _)) = parse_bass_note(after_slash)
            && bass_len == after_slash.len()
        {
            return is_valid_chord_type(before_slash);
        }
    }
    is_valid_chord_type(after_acc)
}

/// Parses an accidental prefix from a note or bass-note string.
#[must_use]
pub fn parse_accidental(s: &str) -> (usize, Option<&str>) {
    if s.starts_with("##") {
        (2, Some("##"))
    } else if s.starts_with('#') {
        (1, Some("#"))
    } else if s.starts_with("bb") {
        (2, Some("bb"))
    } else if s.starts_with('b') {
        (1, Some("b"))
    } else if s.starts_with('♯') {
        ('♯'.len_utf8(), Some("♯"))
    } else if s.starts_with('♭') {
        ('♭'.len_utf8(), Some("♭"))
    } else if s.starts_with('♮') {
        ('♮'.len_utf8(), Some("♮"))
    } else {
        (0, None)
    }
}

/// Parses a bass note and optional accidental, verifying it ends at a boundary.
#[must_use]
pub fn parse_bass_note(s: &str) -> Option<(usize, (char, Option<&str>))> {
    let bass_char = s.chars().next()?;
    if !matches!(bass_char, 'A'..='G' | 'a'..='g') {
        return None;
    }
    let after_bass_char = &s[bass_char.len_utf8()..];
    let (acc_len, acc) = parse_accidental(after_bass_char);
    let after_bass = &after_bass_char[acc_len..];
    if !is_chord_boundary(after_bass) {
        return None;
    }
    Some((bass_char.len_utf8() + acc_len, (bass_char, acc)))
}

/// Returns whether a slice starts at a valid boundary between chord tokens.
#[must_use]
pub fn is_chord_boundary(s: &str) -> bool {
    let Some(first) = s.chars().next() else {
        return true;
    };
    first.is_ascii_whitespace()
        || matches!(
            first,
            ')' | '(' | '[' | ']' | '{' | '}' | ',' | ';' | '|' | ':' | '"' | '<' | '>'
        )
        || (first == '-' && s.len() > 1 && matches!(s.as_bytes()[1], b'A'..=b'G'))
}

/// Returns whether the string is composed entirely of valid chord-type descriptors.
#[must_use]
pub fn is_valid_chord_type(mut s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    while !s.is_empty() {
        if let Some(rest) = s.strip_prefix('(') {
            let Some(close_idx) = rest.find(')') else {
                return false;
            };
            let inside = &rest[..close_idx];
            if !is_valid_chord_extension_list(inside) {
                return false;
            }
            s = &rest[close_idx + 1..];
            continue;
        }

        let Some(len) = match_chord_type_token(s) else {
            return false;
        };
        s = &s[len..];
    }
    true
}

/// Returns whether a parenthesized list contains only valid chord extensions.
#[must_use]
pub fn is_valid_chord_extension_list(mut s: &str) -> bool {
    s = s.trim();
    if s.is_empty() {
        return false;
    }
    while !s.is_empty() {
        let Some(len) = match_chord_type_token(s) else {
            return false;
        };
        s = s[len..].trim_start();
        if s.is_empty() {
            break;
        }
        if let Some(rest) = s.strip_prefix([',', '/']) {
            s = rest.trim_start();
            if s.is_empty() {
                return false;
            }
        }
    }
    true
}

/// Matches the longest recognized chord-type token at the start of a slice.
#[must_use]
pub fn match_chord_type_token(s: &str) -> Option<usize> {
    for &token in CHORD_TYPE_TOKENS {
        if s.starts_with(token) {
            return Some(token.len());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::is_chord_boundary;
    use super::is_valid_chord_extension_list;
    use super::is_valid_chord_type;
    use super::parse_accidental;
    use super::parse_bass_note;
    use super::starts_with_chord_root;

    #[test]
    fn chord_recognition() {
        assert!(starts_with_chord_root("C"));
        assert!(starts_with_chord_root("Am7"));
        assert!(starts_with_chord_root("G7(b9,#11)"));
        assert!(starts_with_chord_root("Bbmaj7/D"));
        assert!(starts_with_chord_root("Cm/maj7"));
        assert!(starts_with_chord_root("C6/9"));
        assert!(starts_with_chord_root("Cm7/G"));
        assert!(!starts_with_chord_root("C/not_a_bass"));
        assert!(!starts_with_chord_root("C/Abc"));
        assert!(!starts_with_chord_root("C/"));
        assert!(!starts_with_chord_root("Hello"));
    }

    #[test]
    fn accidental_and_bass_parsing() {
        assert_eq!(parse_accidental("##x"), (2, Some("##")));
        assert_eq!(parse_accidental("b9"), (1, Some("b")));
        assert_eq!(parse_accidental(""), (0, None));

        let (len, (note, acc)) = parse_bass_note("F# ").unwrap();
        assert_eq!(len, 2);
        assert_eq!(note, 'F');
        assert_eq!(acc, Some("#"));
        assert!(is_chord_boundary(" "));
    }

    #[test]
    fn chord_type_validations() {
        assert!(is_valid_chord_type("maj7"));
        assert!(is_valid_chord_type("m(maj7)"));
        assert!(is_valid_chord_type("7(#9,b13)"));
        assert!(is_valid_chord_extension_list("b9, #11"));
        assert!(!is_valid_chord_extension_list(",,,"));
        assert!(!is_valid_chord_extension_list("b9,"));
        assert!(!is_valid_chord_extension_list(",b9"));
        assert!(!is_valid_chord_type("xyz"));
    }
}
