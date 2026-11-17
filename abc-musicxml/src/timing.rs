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

//! Exact rational duration arithmetic, `MusicXML` `<divisions>` LCM calculation,
//! broken rhythm and tuplet scaling, and graphic `<type>`/`<dot>` decomposition.

use std::cmp;
use std::fmt;
use std::fmt::Write as _;

use abc_parser::BrokenRhythm;
use abc_parser::Fraction;
use abc_parser::Meter;
use abc_parser::NoteLength;
use abc_parser::Tuplet;

use crate::writer::XmlWriter;

/// Maximum `<divisions>` value recommended by `MusicXML` (`attributes.mod:165`)
/// for Standard MIDI File compatibility.
pub const MIDI_MAX_DIVISIONS: u128 = 16_383;

/// Exact non-negative rational duration expressed as a reduced fraction of a whole note.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Duration {
    numerator: u128,
    denominator: u128,
}

impl Default for Duration {
    fn default() -> Self {
        Self::ZERO
    }
}

impl PartialOrd for Duration {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Duration {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        compare_fractions(
            self.numerator,
            self.denominator,
            other.numerator,
            other.denominator,
        )
    }
}

const fn compare_fractions(n1: u128, d1: u128, n2: u128, d2: u128) -> cmp::Ordering {
    // Reduce cross terms when direct u128 multiplication would overflow.
    if let (Some(lhs), Some(rhs)) = (n1.checked_mul(d2), n2.checked_mul(d1)) {
        if lhs < rhs {
            cmp::Ordering::Less
        } else if lhs > rhs {
            cmp::Ordering::Greater
        } else {
            cmp::Ordering::Equal
        }
    } else {
        let q1 = n1 / d1;
        let q2 = n2 / d2;
        if q1 < q2 {
            cmp::Ordering::Less
        } else if q1 > q2 {
            cmp::Ordering::Greater
        } else {
            let r1 = n1 % d1;
            let r2 = n2 % d2;
            if r1 == 0 && r2 == 0 {
                cmp::Ordering::Equal
            } else if r1 == 0 {
                cmp::Ordering::Less
            } else if r2 == 0 {
                cmp::Ordering::Greater
            } else {
                // Compare r1/d1 vs r2/d2 by comparing reciprocals d2/r2 vs d1/r1.
                compare_fractions(d2, r2, d1, r1)
            }
        }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

impl Duration {
    /// Zero whole-note duration (`0/1`).
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    /// One whole-note duration (`1/1`).
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    /// Quarter-note duration (`1/4`).
    pub const QUARTER: Self = Self {
        numerator: 1,
        denominator: 4,
    };

    /// Eighth-note duration (`1/8`).
    pub const EIGHTH: Self = Self {
        numerator: 1,
        denominator: 8,
    };

    /// Sixteenth-note duration (`1/16`).
    pub const SIXTEENTH: Self = Self {
        numerator: 1,
        denominator: 16,
    };

    /// Creates a reduced rational duration `numerator / denominator`.
    ///
    /// If `denominator == 0`, returns [`Self::ZERO`].
    #[must_use]
    pub const fn new(numerator: u128, denominator: u128) -> Self {
        if numerator == 0 || denominator == 0 {
            return Self::ZERO;
        }
        let divisor = Fraction::gcd(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    /// Converts an [`abc_parser::Fraction`] into a reduced [`Duration`].
    #[must_use]
    pub const fn from_fraction(fraction: Fraction) -> Self {
        Self::new(fraction.numerator as u128, fraction.denominator as u128)
    }

    /// Converts an [`abc_parser::NoteLength`] multiplier into a reduced [`Duration`].
    #[must_use]
    pub const fn from_note_length(length: NoteLength) -> Self {
        Self::new(length.numerator as u128, length.denominator as u128)
    }

    /// Returns the reduced numerator.
    #[must_use]
    pub const fn numerator(self) -> u128 {
        self.numerator
    }

    /// Returns the reduced denominator.
    #[must_use]
    pub const fn denominator(self) -> u128 {
        self.denominator
    }

    /// Returns `true` when this duration is zero (`0/1`).
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    /// Adds two rational durations in lowest terms, cross-reducing to avoid overflow.
    #[must_use]
    pub const fn add(self, other: Self) -> Self {
        if self.numerator == 0 {
            return other;
        }
        if other.numerator == 0 {
            return self;
        }
        let g = Fraction::gcd(self.denominator, other.denominator);
        let s_den = self.denominator / g;
        let o_den = other.denominator / g;
        let num = self
            .numerator
            .saturating_mul(o_den)
            .saturating_add(other.numerator.saturating_mul(s_den));
        let den = self.denominator.saturating_mul(o_den);
        Self::new(num, den)
    }

    /// Subtracts `other` from `self`, returning `None` if `self < other`.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        if matches!(
            compare_fractions(
                self.numerator,
                self.denominator,
                other.numerator,
                other.denominator,
            ),
            cmp::Ordering::Less
        ) {
            return None;
        }
        let g = Fraction::gcd(self.denominator, other.denominator);
        let s_den = self.denominator / g;
        let o_den = other.denominator / g;
        let lhs = self.numerator.saturating_mul(o_den);
        let rhs = other.numerator.saturating_mul(s_den);
        Some(Self::new(lhs - rhs, self.denominator.saturating_mul(o_den)))
    }

    /// Subtracts `other` from `self`, returning [`Self::ZERO`] if `self < other`.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        match self.checked_sub(other) {
            Some(res) => res,
            None => Self::ZERO,
        }
    }

    /// Multiplies two rational durations in lowest terms, cross-canceling common factors first.
    #[must_use]
    pub const fn multiply(self, other: Self) -> Self {
        if self.numerator == 0 || other.numerator == 0 {
            return Self::ZERO;
        }
        let g1 = Fraction::gcd(self.numerator, other.denominator);
        let g2 = Fraction::gcd(other.numerator, self.denominator);
        Self::new(
            (self.numerator / g1).saturating_mul(other.numerator / g2),
            (self.denominator / g2).saturating_mul(other.denominator / g1),
        )
    }

    /// Divides `self` by `other`, returning [`Self::ZERO`] if `other` is zero.
    #[must_use]
    pub const fn divide(self, other: Self) -> Self {
        if other.numerator == 0 {
            Self::ZERO
        } else {
            self.multiply(Self {
                numerator: other.denominator,
                denominator: other.numerator,
            })
        }
    }

    /// Returns `true` if `self` is strictly shorter than `other`.
    #[must_use]
    pub const fn is_shorter_than(self, other: Self) -> bool {
        matches!(
            compare_fractions(
                self.numerator,
                self.denominator,
                other.numerator,
                other.denominator,
            ),
            cmp::Ordering::Less
        )
    }

    /// Returns `true` if `self <= other`.
    #[must_use]
    pub const fn is_at_most(self, other: Self) -> bool {
        !matches!(
            compare_fractions(
                self.numerator,
                self.denominator,
                other.numerator,
                other.denominator,
            ),
            cmp::Ordering::Greater
        )
    }

    /// Returns the larger of `self` and `other`.
    #[must_use]
    pub const fn max(self, other: Self) -> Self {
        if self.is_shorter_than(other) {
            other
        } else {
            self
        }
    }

    /// Converts a whole-note fraction $D$ into a reduced quarter-note multiple $Q = 4D$.
    #[must_use]
    pub const fn quarter_note_multiple(self) -> Self {
        self.multiply(Self {
            numerator: 4,
            denominator: 1,
        })
    }

    /// Converts a whole-note duration into integer `MusicXML` `<duration>` ticks
    /// given `divisions` per quarter note.
    #[must_use]
    pub const fn to_divisions(self, divisions: u128) -> u128 {
        if self.numerator == 0 || divisions == 0 {
            return 0;
        }
        let quarter = self.quarter_note_multiple();
        quarter.numerator * (divisions / quarter.denominator)
    }
}

/// Computes the least common multiple of two positive 128-bit integers.
#[must_use]
pub const fn lcm(left: u128, right: u128) -> u128 {
    if left == 0 || right == 0 {
        return 0;
    }
    let divisor = Fraction::gcd(left, right);
    (left / divisor).saturating_mul(right)
}

/// Computes the `MusicXML` `<divisions>` per quarter note as the LCM of all
/// quarter-note denominators across `whole_note_durations`.
///
/// Returns `(divisions, exceeds_midi_limit)` where `exceeds_midi_limit` is
/// `true` when `divisions > 16383`.
#[must_use]
pub fn compute_divisions<I>(whole_note_durations: I) -> (u128, bool)
where
    I: IntoIterator<Item = Duration>,
{
    let mut divisions = 1_u128;
    for duration in whole_note_durations {
        if !duration.is_zero() {
            let quarter = duration.quarter_note_multiple();
            divisions = lcm(divisions, quarter.denominator);
        }
    }
    (divisions, divisions > MIDI_MAX_DIVISIONS)
}

/// Computes the `(first_note_factor, second_note_factor)` scaling pair for an
/// ABC broken-rhythm operator (`>`, `>>`, `<`, `<<`).
///
/// Per ABC 2.1 §4.4, `n` markers scale the dotted note by $(2^{n+1} - 1) / 2^n$
/// and the shortened note by $1 / 2^n$.
#[must_use]
pub const fn broken_rhythm_factors(rhythm: BrokenRhythm) -> Option<(Duration, Duration)> {
    if rhythm.count == 0 {
        return Some((Duration::ONE, Duration::ONE));
    }
    let Some(power) = 1_u128.checked_shl(rhythm.count as u32) else {
        return None;
    };
    let Some(doubled) = power.checked_mul(2) else {
        return None;
    };
    let long = Duration::new(doubled - 1, power);
    let short = Duration::new(1, power);
    if rhythm.greater {
        Some((long, short))
    } else {
        Some((short, long))
    }
}

/// Resolved timing parameters for an ABC `(p:q:r` tuplet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedTuplet {
    /// Number of tuplet notes in the ratio (`p`).
    pub actual: u8,
    /// Number of normal notes whose time they occupy (`q`).
    pub normal: u8,
    /// Number of following timed groups governed by this tuplet (`r`).
    pub affected: u8,
    /// Duration multiplier `q / p` applied to each affected timed group.
    pub factor: Duration,
}

/// Resolves an [`abc_parser::Tuplet`] against whether the active meter is compound.
#[must_use]
pub const fn resolve_tuplet(tuplet: Tuplet, compound_meter: bool) -> Option<ResolvedTuplet> {
    let Some(normal) = tuplet.normal_note_count(compound_meter) else {
        return None;
    };
    let affected = tuplet.affected_note_count();
    if tuplet.actual == 0 || normal == 0 || affected == 0 {
        return None;
    }
    Some(ResolvedTuplet {
        actual: tuplet.actual,
        normal,
        affected,
        factor: Duration::new(normal as u128, tuplet.actual as u128),
    })
}

/// Returns `true` if `meter` is a compound meter (numerator $> 3$ and divisible by $3$).
#[must_use]
pub fn is_compound_meter(meter: &Meter) -> bool {
    match meter {
        Meter::Common | Meter::Cut | Meter::None => false,
        Meter::Simple(fraction) => fraction.numerator > 3 && fraction.numerator.is_multiple_of(3),
        Meter::Compound { groups, .. } => {
            let sum: u32 = groups.iter().copied().sum();
            sum > 3 && sum.is_multiple_of(3)
        }
    }
}

/// Returns the expected whole-note duration of one full measure under `meter`,
/// or `None` for free meter (`M:none`).
#[must_use]
pub fn meter_bar_duration(meter: &Meter) -> Option<Duration> {
    match meter {
        Meter::Common | Meter::Cut => Some(Duration::ONE),
        Meter::None => None,
        Meter::Simple(fraction) => Some(Duration::from_fraction(*fraction)),
        Meter::Compound {
            groups,
            denominator,
        } => {
            let numerator: u128 = groups.iter().map(|&g| u128::from(g)).sum();
            Some(Duration::new(numerator, u128::from(*denominator)))
        }
    }
}

/// Writes a `MusicXML` 4.0 `<time>` element for `meter` into `writer`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `writer` fails.
pub fn write_time_signature<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    meter: &Meter,
) -> fmt::Result {
    match meter {
        Meter::Common => {
            writer.open_tag_with_attrs("time", &[("symbol", "common")])?;
            writer.text_element("beats", "4")?;
            writer.text_element("beat-type", "4")?;
        }
        Meter::Cut => {
            writer.open_tag_with_attrs("time", &[("symbol", "cut")])?;
            writer.text_element("beats", "2")?;
            writer.text_element("beat-type", "2")?;
        }
        Meter::None => {
            writer.open_tag("time")?;
            writer.empty_tag("senza-misura")?;
        }
        Meter::Simple(fraction) => {
            writer.open_tag("time")?;
            writer.display_element("beats", fraction.numerator)?;
            writer.display_element("beat-type", fraction.denominator)?;
        }
        Meter::Compound {
            groups,
            denominator,
        } => {
            let mut beats = String::new();
            for (idx, group) in groups.iter().enumerate() {
                if idx > 0 {
                    beats.push('+');
                }
                let _ = write!(beats, "{group}");
            }
            writer.open_tag("time")?;
            writer.text_element("beats", &beats)?;
            writer.display_element("beat-type", *denominator)?;
        }
    }
    writer.close_tag("time")
}

/// Graphic notehead value for the `MusicXML` `<type>` element (`note-type-value`).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NoteType {
    /// `"1024th"` (`1/1024` whole note).
    OneThousandTwentyFourth,
    /// `"512th"` (`1/512` whole note).
    FiveHundredTwelfth,
    /// `"256th"` (`1/256` whole note).
    TwoHundredFiftySixth,
    /// `"128th"` (`1/128` whole note).
    OneHundredTwentyEighth,
    /// `"64th"` (`1/64` whole note).
    SixtyFourth,
    /// `"32nd"` (`1/32` whole note).
    ThirtySecond,
    /// `"16th"` (`1/16` whole note).
    Sixteenth,
    /// `"eighth"` (`1/8` whole note).
    Eighth,
    /// `"quarter"` (`1/4` whole note).
    Quarter,
    /// `"half"` (`1/2` whole note).
    Half,
    /// `"whole"` (`1/1` whole note).
    Whole,
    /// `"breve"` (`2/1` whole notes).
    Breve,
    /// `"long"` (`4/1` whole notes).
    Long,
    /// `"maxima"` (`8/1` whole notes).
    Maxima,
}

impl NoteType {
    /// All standard `MusicXML` graphic note types from longest (`Maxima`) to shortest (`1024th`).
    pub const ALL_DESCENDING: [Self; 14] = [
        Self::Maxima,
        Self::Long,
        Self::Breve,
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
        Self::SixtyFourth,
        Self::OneHundredTwentyEighth,
        Self::TwoHundredFiftySixth,
        Self::FiveHundredTwelfth,
        Self::OneThousandTwentyFourth,
    ];

    /// Returns the `MusicXML` `note-type-value` string for this note type.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OneThousandTwentyFourth => "1024th",
            Self::FiveHundredTwelfth => "512th",
            Self::TwoHundredFiftySixth => "256th",
            Self::OneHundredTwentyEighth => "128th",
            Self::SixtyFourth => "64th",
            Self::ThirtySecond => "32nd",
            Self::Sixteenth => "16th",
            Self::Eighth => "eighth",
            Self::Quarter => "quarter",
            Self::Half => "half",
            Self::Whole => "whole",
            Self::Breve => "breve",
            Self::Long => "long",
            Self::Maxima => "maxima",
        }
    }

    /// Returns the undotted whole-note [`Duration`] of this note type.
    #[must_use]
    pub const fn whole_note_duration(self) -> Duration {
        match self {
            Self::OneThousandTwentyFourth => Duration {
                numerator: 1,
                denominator: 1024,
            },
            Self::FiveHundredTwelfth => Duration {
                numerator: 1,
                denominator: 512,
            },
            Self::TwoHundredFiftySixth => Duration {
                numerator: 1,
                denominator: 256,
            },
            Self::OneHundredTwentyEighth => Duration {
                numerator: 1,
                denominator: 128,
            },
            Self::SixtyFourth => Duration {
                numerator: 1,
                denominator: 64,
            },
            Self::ThirtySecond => Duration {
                numerator: 1,
                denominator: 32,
            },
            Self::Sixteenth => Duration::SIXTEENTH,
            Self::Eighth => Duration::EIGHTH,
            Self::Quarter => Duration::QUARTER,
            Self::Half => Duration {
                numerator: 1,
                denominator: 2,
            },
            Self::Whole => Duration::ONE,
            Self::Breve => Duration {
                numerator: 2,
                denominator: 1,
            },
            Self::Long => Duration {
                numerator: 4,
                denominator: 1,
            },
            Self::Maxima => Duration {
                numerator: 8,
                denominator: 1,
            },
        }
    }

    /// Returns the whole-note [`Duration`] of this note type with `dots` augmentation dots ($0 \le \text{dots} \le 4$).
    #[must_use]
    pub const fn dotted_duration(self, dots: u8) -> Duration {
        let power = 1_u128 << (dots as u32);
        let factor = Duration::new(power * 2 - 1, power);
        self.whole_note_duration().multiply(factor)
    }

    /// Returns the number of beams (`1..=8`) required for this note type, or `0` for quarter and longer.
    #[must_use]
    pub const fn beam_count(self) -> u8 {
        match self {
            Self::Eighth => 1,
            Self::Sixteenth => 2,
            Self::ThirtySecond => 3,
            Self::SixtyFourth => 4,
            Self::OneHundredTwentyEighth => 5,
            Self::TwoHundredFiftySixth => 6,
            Self::FiveHundredTwelfth => 7,
            Self::OneThousandTwentyFourth => 8,
            Self::Quarter | Self::Half | Self::Whole | Self::Breve | Self::Long | Self::Maxima => 0,
        }
    }
}

/// One graphic notehead segment (`<type>` + `<dot>*`) with its untupleted graphic
/// duration and tuplet-scaled sounding duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphicSegment {
    /// Graphic `<type>` value.
    pub note_type: NoteType,
    /// Number of `<dot/>` elements (`0..=4`).
    pub dots: u8,
    /// Untupleted graphic whole-note duration of this segment.
    pub graphic_duration: Duration,
    /// Sounding whole-note duration of this segment (after tuplet scaling).
    pub sounding_duration: Duration,
}

/// Decomposes an untupleted `graphic` duration (and corresponding `sounding` duration)
/// into one or more representable [`GraphicSegment`]s.
///
/// Standard dotted or undotted durations (`1/4`, `3/8`, `7/16`, etc.) produce a single
/// segment. Compound durations that cannot be represented by a single notehead with
/// $\le 4$ dots (such as `5/8 = 1/2 + 1/8` for `C5` in `L:1/8`) are greedily split into
/// largest-first segments so callers can tie pitched segments or emit sequential rests.
#[must_use]
pub fn decompose_graphic_duration(graphic: Duration, sounding: Duration) -> Vec<GraphicSegment> {
    if graphic.is_zero() {
        return vec![GraphicSegment {
            note_type: NoteType::Quarter,
            dots: 0,
            graphic_duration: Duration::ZERO,
            sounding_duration: sounding,
        }];
    }

    // Fast path: exact match with a single (NoteType, dots) pair.
    if let Some((note_type, dots)) = match_single_graphic(graphic) {
        return vec![GraphicSegment {
            note_type,
            dots,
            graphic_duration: graphic,
            sounding_duration: sounding,
        }];
    }

    // Greedy decomposition for compound durations (e.g., 5/8 = 1/2 + 1/8).
    let mut segments = Vec::new();
    let mut remaining = graphic;
    let mut remaining_sounding = sounding;

    for _ in 0..16 {
        if remaining.is_zero() {
            break;
        }
        if let Some((note_type, dots)) = match_single_graphic(remaining) {
            segments.push(GraphicSegment {
                note_type,
                dots,
                graphic_duration: remaining,
                sounding_duration: remaining_sounding,
            });
            remaining = Duration::ZERO;
            break;
        }

        let Some((note_type, dots, seg_graphic)) = largest_graphic_at_most(remaining) else {
            break;
        };
        let seg_sounding = seg_graphic.multiply(sounding).divide(graphic);
        segments.push(GraphicSegment {
            note_type,
            dots,
            graphic_duration: seg_graphic,
            sounding_duration: seg_sounding,
        });
        remaining = remaining.checked_sub(seg_graphic).unwrap_or(Duration::ZERO);
        remaining_sounding = remaining_sounding
            .checked_sub(seg_sounding)
            .unwrap_or(Duration::ZERO);
    }

    if !remaining.is_zero() {
        if let Some(last) = segments.last_mut() {
            last.graphic_duration = last.graphic_duration.add(remaining);
            last.sounding_duration = last.sounding_duration.add(remaining_sounding);
        } else {
            segments.push(GraphicSegment {
                note_type: NoteType::OneThousandTwentyFourth,
                dots: 0,
                graphic_duration: graphic,
                sounding_duration: sounding,
            });
        }
    }

    segments
}

const fn match_single_graphic(duration: Duration) -> Option<(NoteType, u8)> {
    let mut idx = 0;
    while idx < NoteType::ALL_DESCENDING.len() {
        let note_type = NoteType::ALL_DESCENDING[idx];
        let mut dots = 0_u8;
        while dots <= 4 {
            let candidate = note_type.dotted_duration(dots);
            if candidate.numerator == duration.numerator
                && candidate.denominator == duration.denominator
            {
                return Some((note_type, dots));
            }
            dots += 1;
        }
        idx += 1;
    }
    None
}

const fn largest_graphic_at_most(limit: Duration) -> Option<(NoteType, u8, Duration)> {
    let mut idx = 0;
    while idx < NoteType::ALL_DESCENDING.len() {
        let note_type = NoteType::ALL_DESCENDING[idx];
        let mut dots = 4_u8;
        loop {
            let candidate = note_type.dotted_duration(dots);
            if candidate.is_at_most(limit) {
                return Some((note_type, dots, candidate));
            }
            if dots == 0 {
                break;
            }
            dots -= 1;
        }
        idx += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use abc_parser::BrokenRhythm;
    use abc_parser::Fraction;
    use abc_parser::Meter;
    use abc_parser::Tuplet;

    use super::Duration;
    use super::GraphicSegment;
    use super::NoteType;
    use super::broken_rhythm_factors;
    use super::compute_divisions;
    use super::decompose_graphic_duration;
    use super::is_compound_meter;
    use super::meter_bar_duration;
    use super::resolve_tuplet;
    use super::write_time_signature;
    use crate::writer::XmlWriter;

    #[test]
    fn computes_lcm_divisions_across_dotted_broken_and_tuplet_durations() {
        // 1/8 note (Q = 1/2), dotted 1/8 (Q = 3/4), triplet 1/8 (1/8 * 2/3 = 1/12, Q = 1/3)
        // Denominators in quarter notes: 2, 4, 3 -> LCM = 12.
        let eighth = Duration::EIGHTH;
        let dotted_eighth = Duration::new(3, 16);
        let triplet_eighth = eighth.multiply(Duration::new(2, 3));

        let (divisions, exceeds_midi) = compute_divisions([eighth, dotted_eighth, triplet_eighth]);
        assert_eq!(divisions, 12);
        assert!(!exceeds_midi);
        assert_eq!(eighth.to_divisions(divisions), 6);
        assert_eq!(dotted_eighth.to_divisions(divisions), 9);
        assert_eq!(triplet_eighth.to_divisions(divisions), 4);
    }

    #[test]
    fn resolves_broken_rhythms_and_meter_dependent_tuplets() {
        let single_dot = broken_rhythm_factors(BrokenRhythm {
            greater: true,
            count: 1,
        })
        .unwrap();
        assert_eq!(single_dot, (Duration::new(3, 2), Duration::new(1, 2)));

        let reverse_double_dot = broken_rhythm_factors(BrokenRhythm {
            greater: false,
            count: 2,
        })
        .unwrap();
        assert_eq!(
            reverse_double_dot,
            (Duration::new(1, 4), Duration::new(7, 4))
        );

        // (5 in simple meter occupies 2 normal notes; in compound 6/8 meter it occupies 3.
        let five_tuplet = Tuplet {
            actual: 5,
            normal: None,
            affected: None,
        };
        let simple = resolve_tuplet(five_tuplet, false).unwrap();
        assert_eq!((simple.actual, simple.normal, simple.affected), (5, 2, 5));
        assert_eq!(simple.factor, Duration::new(2, 5));

        let compound = resolve_tuplet(five_tuplet, true).unwrap();
        assert_eq!(
            (compound.actual, compound.normal, compound.affected),
            (5, 3, 5)
        );
        assert_eq!(compound.factor, Duration::new(3, 5));
    }

    #[test]
    fn decomposes_single_dotted_and_compound_tied_durations() {
        // C6 in L:1/8 -> 6/8 = 3/4 -> single dotted half note.
        let dotted_half = Duration::new(6, 8);
        assert_eq!(
            decompose_graphic_duration(dotted_half, dotted_half),
            vec![GraphicSegment {
                note_type: NoteType::Half,
                dots: 1,
                graphic_duration: Duration::new(3, 4),
                sounding_duration: Duration::new(3, 4),
            }]
        );

        // C5 in L:1/8 -> 5/8 = 1/2 + 1/8 -> half note tied to eighth note.
        let five_eighths = Duration::new(5, 8);
        assert_eq!(
            decompose_graphic_duration(five_eighths, five_eighths),
            vec![
                GraphicSegment {
                    note_type: NoteType::Half,
                    dots: 0,
                    graphic_duration: Duration::new(1, 2),
                    sounding_duration: Duration::new(1, 2),
                },
                GraphicSegment {
                    note_type: NoteType::Eighth,
                    dots: 0,
                    graphic_duration: Duration::new(1, 8),
                    sounding_duration: Duration::new(1, 8),
                },
            ]
        );
    }

    #[test]
    fn formats_time_signatures_and_meter_properties() {
        let compound = Meter::Compound {
            groups: vec![2, 3, 2],
            denominator: 8,
        };
        assert!(!is_compound_meter(&compound));
        assert_eq!(meter_bar_duration(&compound), Some(Duration::new(7, 8)));

        let six_eight = Meter::Simple(Fraction {
            numerator: 6,
            denominator: 8,
        });
        assert!(is_compound_meter(&six_eight));
        assert_eq!(meter_bar_duration(&Meter::None), None);

        let mut writer = XmlWriter::new(String::new(), 0);
        write_time_signature(&mut writer, &Meter::Common).unwrap();
        write_time_signature(&mut writer, &Meter::Cut).unwrap();
        write_time_signature(&mut writer, &Meter::None).unwrap();
        write_time_signature(&mut writer, &compound).unwrap();

        assert_eq!(
            writer.into_inner(),
            concat!(
                "<time symbol=\"common\">\n<beats>4</beats>\n<beat-type>4</beat-type>\n</time>\n",
                "<time symbol=\"cut\">\n<beats>2</beats>\n<beat-type>2</beat-type>\n</time>\n",
                "<time>\n<senza-misura/>\n</time>\n",
                "<time>\n<beats>2+3+2</beats>\n<beat-type>8</beat-type>\n</time>\n",
            )
        );
    }
}
