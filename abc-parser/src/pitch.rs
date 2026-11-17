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

//! Pitch, accidental offset arithmetic, and chromatic spelling helpers.

use std::ops::Add;
use std::ops::Sub;

use crate::ast::Accidental;
use crate::ast::Fraction;
use crate::ast::Pitch;
use crate::ast::PitchClass;

/// A rational accidental displacement measured in semitones.
///
/// Displacements support fractional semitones to accommodate quarter-tone and
/// microtonal accidentals without losing precision.
///
/// # Examples
///
/// ```
/// use abc_parser::pitch::Offset;
///
/// let whole_step = Offset::integer(2);
/// let half_step = Offset::integer(1);
/// assert_eq!(whole_step.subtract(half_step), half_step);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Offset {
    /// The signed numerator of the semitone fraction.
    pub numerator: i128,
    /// The positive denominator of the semitone fraction.
    pub denominator: i128,
}

impl Offset {
    /// Zero semitone offset.
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    /// Creates a rational displacement reduced to lowest terms with a positive
    /// denominator.
    ///
    /// Returns [`Self::ZERO`] when either `numerator` or `denominator` is zero.
    #[must_use]
    pub const fn new(mut numerator: i128, mut denominator: i128) -> Self {
        if numerator == 0 || denominator == 0 {
            return Self::ZERO;
        }
        if denominator < 0 {
            numerator = numerator.saturating_neg();
            denominator = denominator.saturating_neg();
        }
        let divisor = Fraction::gcd(numerator.unsigned_abs(), denominator.cast_unsigned());
        Self {
            numerator: numerator / divisor.cast_signed(),
            denominator: denominator / divisor.cast_signed(),
        }
    }

    /// Creates an integral semitone displacement.
    #[must_use]
    pub const fn integer(value: i16) -> Self {
        Self {
            numerator: value as i128,
            denominator: 1,
        }
    }

    /// Adds two rational semitone displacements in lowest terms without losing precision.
    #[must_use]
    pub const fn add(self, other: Self) -> Self {
        if self.numerator == 0 || self.denominator == 0 {
            return Self::new(other.numerator, other.denominator);
        }
        if other.numerator == 0 || other.denominator == 0 {
            return Self::new(self.numerator, self.denominator);
        }
        let divisor = Fraction::gcd(
            self.denominator.unsigned_abs(),
            other.denominator.unsigned_abs(),
        )
        .cast_signed();
        let self_den = self.denominator / divisor;
        let other_den = other.denominator / divisor;
        Self::new(
            self.numerator * other_den + other.numerator * self_den,
            self.denominator * other_den,
        )
    }

    /// Returns the same displacement relative to another displacement.
    #[must_use]
    pub const fn subtract(self, other: Self) -> Self {
        self.add(Self {
            numerator: other.numerator.saturating_neg(),
            denominator: other.denominator,
        })
    }

    /// Returns whether this displacement is exactly zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }
}

impl Default for Offset {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Add for Offset {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        self.add(rhs)
    }
}

impl Sub for Offset {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self.subtract(rhs)
    }
}

/// Accidental state for each written pitch class within the current bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MeasureAccidentals([Option<Offset>; 7]);

impl MeasureAccidentals {
    /// Creates an empty measure accidental state.
    #[must_use]
    pub const fn new() -> Self {
        Self([None; 7])
    }

    /// Returns the active accidental for one written pitch class.
    #[must_use]
    pub const fn get(self, class: PitchClass) -> Option<Offset> {
        self.0[class_index(class)]
    }

    /// Records an accidental for one written pitch class.
    pub const fn set(&mut self, class: PitchClass, offset: Offset) {
        self.0[class_index(class)] = Some(offset);
    }

    /// Clears accidental propagation at a measure boundary.
    pub fn clear(&mut self) {
        self.0.fill(None);
    }
}

/// Converts a parsed accidental to its signed rational semitone offset.
#[must_use]
pub fn accidental_offset(accidental: Accidental) -> Offset {
    match accidental {
        Accidental::Natural => Offset::ZERO,
        Accidental::Sharp(value) => {
            Offset::new(i128::from(value.numerator), i128::from(value.denominator))
        }
        Accidental::Flat(value) => {
            Offset::new(-i128::from(value.numerator), i128::from(value.denominator))
        }
    }
}

/// Produces an explicit accidental from a rational semitone offset.
pub fn offset_accidental(offset: Offset) -> Result<Accidental, String> {
    if offset.is_zero() {
        return Ok(Accidental::Natural);
    }
    let divisor = Fraction::gcd(
        offset.numerator.unsigned_abs(),
        offset.denominator.cast_unsigned(),
    );
    let numerator = u32::try_from(offset.numerator.unsigned_abs() / divisor)
        .map_err(|_| "transposed accidental numerator exceeds u32".to_owned())?;
    let denominator = u32::try_from(offset.denominator.cast_unsigned() / divisor)
        .map_err(|_| "transposed accidental denominator exceeds u32".to_owned())?;
    let fraction = Fraction {
        numerator,
        denominator,
    };
    Ok(if offset.numerator > 0 {
        Accidental::Sharp(fraction)
    } else {
        Accidental::Flat(fraction)
    })
}

/// Returns the mathematical ceiling of a rational value.
#[must_use]
pub const fn ceiling_fraction(value: Offset) -> i128 {
    -(-value.numerator).div_euclid(value.denominator)
}

/// Selects a conventional sharp- or flat-oriented chromatic spelling.
#[must_use]
pub const fn chromatic_spelling(chromatic: u8, prefer_flats: bool) -> (PitchClass, i16) {
    if prefer_flats {
        match chromatic {
            0 => (PitchClass::C, 0),
            1 => (PitchClass::D, -1),
            2 => (PitchClass::D, 0),
            3 => (PitchClass::E, -1),
            4 => (PitchClass::E, 0),
            5 => (PitchClass::F, 0),
            6 => (PitchClass::G, -1),
            7 => (PitchClass::G, 0),
            8 => (PitchClass::A, -1),
            9 => (PitchClass::A, 0),
            10 => (PitchClass::B, -1),
            _ => (PitchClass::B, 0),
        }
    } else {
        match chromatic {
            0 => (PitchClass::C, 0),
            1 => (PitchClass::C, 1),
            2 => (PitchClass::D, 0),
            3 => (PitchClass::D, 1),
            4 => (PitchClass::E, 0),
            5 => (PitchClass::F, 0),
            6 => (PitchClass::F, 1),
            7 => (PitchClass::G, 0),
            8 => (PitchClass::G, 1),
            9 => (PitchClass::A, 0),
            10 => (PitchClass::A, 1),
            _ => (PitchClass::B, 0),
        }
    }
}

/// Returns a pitch class at a signed diatonic distance from another class.
#[must_use]
pub const fn shift_class(class: PitchClass, steps: i8) -> PitchClass {
    let index = (class_number(class) + steps).rem_euclid(7);
    match index {
        0 => PitchClass::C,
        1 => PitchClass::D,
        2 => PitchClass::E,
        3 => PitchClass::F,
        4 => PitchClass::G,
        5 => PitchClass::A,
        _ => PitchClass::B,
    }
}

/// Returns the small signed diatonic number for a pitch class.
#[must_use]
pub const fn class_number(class: PitchClass) -> i8 {
    match class {
        PitchClass::C => 0,
        PitchClass::D => 1,
        PitchClass::E => 2,
        PitchClass::F => 3,
        PitchClass::G => 4,
        PitchClass::A => 5,
        PitchClass::B => 6,
    }
}

/// Returns the diatonic array index for a pitch class.
#[must_use]
pub const fn class_index(class: PitchClass) -> usize {
    match class {
        PitchClass::C => 0,
        PitchClass::D => 1,
        PitchClass::E => 2,
        PitchClass::F => 3,
        PitchClass::G => 4,
        PitchClass::A => 5,
        PitchClass::B => 6,
    }
}

/// Returns the natural chromatic semitone for a pitch class.
#[must_use]
pub const fn natural_semitone(class: PitchClass) -> i16 {
    match class {
        PitchClass::C => 0,
        PitchClass::D => 2,
        PitchClass::E => 4,
        PitchClass::F => 5,
        PitchClass::G => 7,
        PitchClass::A => 9,
        PitchClass::B => 11,
    }
}

/// Normalizes a chromatic difference to the nearest written accidental.
#[must_use]
pub const fn normalize_accidental(value: i16) -> i16 {
    let value = value.rem_euclid(12);
    if value > 6 { value - 12 } else { value }
}

/// Parses a single standalone ABC pitch token such as `^F`, `=c'`, `_3/2B,`,
/// `^/c`, or `d` using [`crate::pitch_parser`].
///
/// Rejects empty strings, whitespace, note lengths, and multi-token inputs.
#[must_use]
pub fn parse_single_pitch(text: &str) -> Option<Pitch> {
    crate::parse_pitch(text).ok()
}

/// Spells an absolute transposed pitch, including an explicit accidental.
pub fn spell_absolute_pitch(
    source: Pitch,
    source_offset: Offset,
    interval: i16,
    prefer_flats: bool,
) -> Result<Pitch, String> {
    let natural = i16::from(source.octave) * 12 + natural_semitone(source.class);
    let absolute = Offset::integer(natural)
        .add(source_offset)
        .add(Offset::integer(interval));
    let anchor = if prefer_flats {
        ceiling_fraction(absolute)
    } else {
        absolute.numerator.div_euclid(absolute.denominator)
    };
    let chromatic = u8::try_from(anchor.rem_euclid(12)).unwrap_or(0);
    let (class, integral_accidental) = chromatic_spelling(chromatic, prefer_flats);
    let octave = i8::try_from(anchor.div_euclid(12))
        .map_err(|_| "transposed pitch exceeds the supported octave range".to_owned())?;
    let class_absolute = i16::from(octave) * 12 + natural_semitone(class);
    let accidental = absolute.subtract(Offset::integer(class_absolute));
    debug_assert_eq!(
        accidental.numerator.signum(),
        i128::from(integral_accidental).signum()
    );
    Ok(Pitch {
        class,
        octave,
        accidental: Some(offset_accidental(accidental)?),
    })
}

#[cfg(test)]
mod tests {
    use super::Offset;
    use super::accidental_offset;
    use super::offset_accidental;
    use super::parse_single_pitch;
    use super::shift_class;
    use crate::ast::Accidental;
    use crate::ast::Fraction;
    use crate::ast::Pitch;
    use crate::ast::PitchClass;

    #[test]
    fn offset_reduction_preserves_value_equality() {
        assert_eq!(Offset::new(2, 4), Offset::new(1, 2));
        assert_eq!(Offset::new(1, -2), Offset::new(-1, 2));
        assert_eq!(Offset::new(0, 5), Offset::ZERO);
        assert_eq!(Offset::new(1, 0), Offset::ZERO);
        assert_eq!(Offset::new(1, 2).add(Offset::new(1, 2)), Offset::integer(1));
        assert_eq!(Offset::new(1, 2).subtract(Offset::new(1, 2)), Offset::ZERO);
    }

    #[test]
    fn single_pitch_parser_handles_accidentals_octaves_and_rejects_trailing_tokens() {
        assert_eq!(
            parse_single_pitch("^F"),
            Some(Pitch {
                class: PitchClass::F,
                octave: 0,
                accidental: Some(Accidental::Sharp(Fraction {
                    numerator: 1,
                    denominator: 1,
                })),
            })
        );
        assert_eq!(
            parse_single_pitch("=c'"),
            Some(Pitch {
                class: PitchClass::C,
                octave: 2,
                accidental: Some(Accidental::Natural),
            })
        );
        assert_eq!(
            parse_single_pitch("_3/2B,"),
            Some(Pitch {
                class: PitchClass::B,
                octave: -1,
                accidental: Some(Accidental::Flat(Fraction {
                    numerator: 3,
                    denominator: 2,
                })),
            })
        );
        assert_eq!(
            parse_single_pitch("^/c"),
            Some(Pitch {
                class: PitchClass::C,
                octave: 1,
                accidental: Some(Accidental::Sharp(Fraction {
                    numerator: 1,
                    denominator: 2,
                })),
            })
        );
        assert_eq!(
            parse_single_pitch("d"),
            Some(Pitch {
                class: PitchClass::D,
                octave: 1,
                accidental: None,
            })
        );

        assert_eq!(parse_single_pitch("C2"), None);
        assert_eq!(parse_single_pitch("C D"), None);
        assert_eq!(parse_single_pitch("^1/0C"), None);
        assert_eq!(parse_single_pitch(""), None);
    }

    #[test]
    fn accidental_round_trip() {
        let sharp = Accidental::Sharp(Fraction {
            numerator: 1,
            denominator: 1,
        });
        let offset = accidental_offset(sharp);
        assert_eq!(offset, Offset::integer(1));
        assert_eq!(offset_accidental(offset).unwrap(), sharp);
    }

    #[test]
    fn shift_class_wraps_diatonically() {
        assert_eq!(shift_class(PitchClass::C, 1), PitchClass::D);
        assert_eq!(shift_class(PitchClass::C, -1), PitchClass::B);
    }
}
