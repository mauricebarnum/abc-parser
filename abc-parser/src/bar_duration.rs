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

//! Semantic validation of bar durations.

use std::collections::BTreeMap;
use std::fmt;

use chumsky::span::SimpleSpan;

use crate::ChordMember;
use crate::Document;
use crate::ErrorKind;
use crate::Field;
use crate::FieldKind;
use crate::FieldValue;
use crate::Fraction;
use crate::Line;
use crate::Meter;
use crate::MusicElement;
use crate::NoteLength;
use crate::ParseWarning;
use crate::Tune;
use crate::Tuplet;

const EIGHTH_NOTE: Fraction = Fraction {
    numerator: 1,
    denominator: 8,
};
const SIXTEENTH_NOTE: Fraction = Fraction {
    numerator: 1,
    denominator: 16,
};

/// Heuristic used to avoid reporting legitimate pickup bars as incomplete.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum BarDurationPickupPolicy {
    /// Do not infer pickup bars.
    #[default]
    None,
    /// Allow an underfull first bar in each voice and metered section.
    OpeningBar,
    /// Pair complementary underfull bars only at section boundaries.
    FirstAndLast,
}

/// Choices controlling semantic bar-duration validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BarDurationOptions {
    pickup_policy: BarDurationPickupPolicy,
    check_trailing_bar: bool,
}

impl BarDurationOptions {
    /// Creates conservative options that check every non-empty bar.
    pub const fn new() -> Self {
        Self {
            pickup_policy: BarDurationPickupPolicy::None,
            check_trailing_bar: true,
        }
    }

    /// Selects how possible pickup bars are inferred.
    #[must_use]
    pub const fn pickup_policy(mut self, policy: BarDurationPickupPolicy) -> Self {
        self.pickup_policy = policy;
        self
    }

    /// Selects whether notes after the final bar line form a completed bar.
    #[must_use]
    pub const fn check_trailing_bar(mut self, check: bool) -> Self {
        self.check_trailing_bar = check;
        self
    }
}

impl Default for BarDurationOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Reports non-empty bars whose duration differs from their effective meter.
///
/// The validator follows body `M:`, `L:`, and `V:` changes and accounts for
/// chords, tuplets, and broken rhythm. Bars with free or indeterminate timing
/// are skipped.
///
/// # Examples
///
/// ```
/// use abc_parser::BarDurationOptions;
/// use abc_parser::BarDurationPickupPolicy;
/// use abc_parser::IntoOwnedAst;
/// use abc_parser::bar_duration_warnings;
/// use abc_parser::parse;
///
/// let source = "X:1\nM:4/4\nL:1/4\nK:C\nC | CDEF |\n";
/// let document = parse(source).output.unwrap().into_owned(source).unwrap();
/// let options = BarDurationOptions::new()
///     .pickup_policy(BarDurationPickupPolicy::OpeningBar);
/// assert!(bar_duration_warnings(&document, options).is_empty());
/// ```
pub fn bar_duration_warnings(
    document: &Document<SimpleSpan<usize>, String>,
    options: BarDurationOptions,
) -> Vec<ParseWarning<SimpleSpan<usize>>> {
    let mut file_defaults = TimingDefaults::default();
    for line in &document.header {
        if let Line::Field(field) = &line.value {
            apply_header_timing(field, &mut file_defaults);
        }
    }

    let mut warnings = Vec::new();
    for tune in document.tunes() {
        warnings.extend(tune_bar_duration_warnings(tune, file_defaults, options));
    }
    warnings
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Duration {
    numerator: u128,
    denominator: u128,
}

impl Duration {
    const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };
    const THREE_QUARTERS: Self = Self {
        numerator: 3,
        denominator: 4,
    };

    const fn new(numerator: u128, denominator: u128) -> Self {
        let divisor = greatest_common_divisor(numerator, denominator);
        Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        }
    }

    const fn add(self, other: Self) -> Self {
        Self::new(
            self.numerator * other.denominator + other.numerator * self.denominator,
            self.denominator * other.denominator,
        )
    }

    const fn multiply(self, other: Self) -> Self {
        Self::new(
            self.numerator * other.numerator,
            self.denominator * other.denominator,
        )
    }

    const fn is_shorter_than(self, other: Self) -> bool {
        self.numerator * other.denominator < other.numerator * self.denominator
    }
}

impl Default for Duration {
    fn default() -> Self {
        Self::new(0, 1)
    }
}

impl From<Fraction> for Duration {
    fn from(value: Fraction) -> Self {
        Self::new(u128::from(value.numerator), u128::from(value.denominator))
    }
}

impl From<NoteLength> for Duration {
    fn from(value: NoteLength) -> Self {
        Self::new(u128::from(value.numerator), u128::from(value.denominator))
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.denominator == 1 {
            write!(formatter, "{}", self.numerator)
        } else if self.numerator < self.denominator {
            write!(formatter, "{}/{}", self.numerator, self.denominator)
        } else {
            write!(
                formatter,
                "{} {}/{}",
                self.numerator / self.denominator,
                self.numerator % self.denominator,
                self.denominator
            )
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct EffectiveMeter {
    duration: Duration,
    beat_denominator: u128,
    compound: bool,
}

#[derive(Clone, Copy)]
struct TimingDefaults {
    meter: Option<EffectiveMeter>,
    unit_length: Fraction,
    explicit_unit_length: bool,
}

impl Default for TimingDefaults {
    fn default() -> Self {
        Self {
            meter: None,
            unit_length: EIGHTH_NOTE,
            explicit_unit_length: false,
        }
    }
}

#[derive(Clone, Copy)]
struct ActiveTuplet {
    factor: Duration,
    remaining: u8,
}

#[derive(Default)]
struct BarTiming {
    duration: Duration,
    expected: Option<Duration>,
    indeterminate: bool,
    seen_timed_group: bool,
    pending_timed_group: Option<Duration>,
    next_broken_rhythm_factor: Option<Duration>,
    last_span: Option<SimpleSpan<usize>>,
}

struct BarDuration {
    duration: Duration,
    expected: Duration,
    beat_denominator: u128,
    position: usize,
    span: SimpleSpan<usize>,
}

struct VoiceTiming {
    meter: Option<EffectiveMeter>,
    unit_length: Fraction,
    tuplets: Vec<ActiveTuplet>,
    bar: BarTiming,
    incomplete_bars: Vec<BarDuration>,
    completed_bars: usize,
    opening_bar_available: bool,
}

impl VoiceTiming {
    fn new(defaults: TimingDefaults) -> Self {
        Self {
            meter: defaults.meter,
            unit_length: defaults.unit_length,
            tuplets: Vec::new(),
            bar: BarTiming::default(),
            incomplete_bars: Vec::new(),
            completed_bars: 0,
            opening_bar_available: true,
        }
    }
}

fn tune_bar_duration_warnings(
    tune: &Tune<SimpleSpan<usize>, String>,
    file_defaults: TimingDefaults,
    options: BarDurationOptions,
) -> Vec<ParseWarning<SimpleSpan<usize>>> {
    let mut defaults = file_defaults;
    let mut body_started = false;
    let mut current_voice = String::new();
    let mut voices = BTreeMap::<String, VoiceTiming>::new();
    let mut warnings = Vec::new();

    for line in &tune.lines {
        if !body_started {
            match &line.value {
                Line::Field(field) => {
                    apply_header_timing(field, &mut defaults);
                    if field.kind == FieldKind::Key {
                        body_started = true;
                    }
                }
                Line::Music(_) => body_started = true,
                _ => {}
            }
        }

        match &line.value {
            Line::Field(field) if body_started && field.kind != FieldKind::Key => {
                apply_body_timing_field(
                    field,
                    &mut current_voice,
                    &mut voices,
                    defaults,
                    options,
                    &mut warnings,
                );
            }
            Line::Music(elements) => {
                ensure_voice_timing(&mut voices, &current_voice, defaults);
                for element in elements {
                    if let MusicElement::InlineField(field) = &element.value {
                        apply_body_timing_field(
                            field,
                            &mut current_voice,
                            &mut voices,
                            defaults,
                            options,
                            &mut warnings,
                        );
                    } else {
                        let voice = voices
                            .get_mut(current_voice.as_str())
                            .expect("the selected voice is initialized before processing music");
                        apply_music_timing(
                            &element.value,
                            element.span,
                            voice,
                            options,
                            &mut warnings,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    for voice in voices.values_mut() {
        if options.check_trailing_bar
            && let Some(span) = voice.bar.last_span
        {
            complete_bar(voice, span, options, &mut warnings);
        }
        finish_pickup_section(voice, options.pickup_policy, &mut warnings);
    }
    warnings.sort_by_key(|warning| (warning.span.start, warning.span.end));
    warnings
}

fn apply_header_timing(field: &Field<String>, defaults: &mut TimingDefaults) {
    match &field.value {
        FieldValue::Meter(meter) => {
            defaults.meter = effective_meter(meter);
            if !defaults.explicit_unit_length {
                defaults.unit_length = default_unit_length(meter);
            }
        }
        FieldValue::UnitLength(unit_length) => {
            defaults.unit_length = *unit_length;
            defaults.explicit_unit_length = true;
        }
        _ => {}
    }
}

fn apply_body_timing_field(
    field: &Field<String>,
    current_voice: &mut String,
    voices: &mut BTreeMap<String, VoiceTiming>,
    defaults: TimingDefaults,
    options: BarDurationOptions,
    warnings: &mut Vec<ParseWarning<SimpleSpan<usize>>>,
) {
    if let FieldValue::Voice(voice) = &field.value {
        current_voice.clone_from(&voice.id);
        ensure_voice_timing(voices, current_voice, defaults);
        return;
    }

    ensure_voice_timing(voices, current_voice, defaults);
    let voice = voices
        .get_mut(current_voice.as_str())
        .expect("the selected voice was initialized above");
    match &field.value {
        FieldValue::Meter(meter) => {
            // ABC 2.1 §3.1.7 specifies that a body M: change does not alter L:.
            let meter = effective_meter(meter);
            if voice.meter != meter {
                finish_pickup_section(voice, options.pickup_policy, warnings);
                voice.opening_bar_available = true;
            }
            if voice.bar.seen_timed_group {
                make_bar_indeterminate(voice);
            }
            voice.meter = meter;
        }
        FieldValue::UnitLength(unit_length) => voice.unit_length = *unit_length,
        _ => {}
    }
}

fn ensure_voice_timing(
    voices: &mut BTreeMap<String, VoiceTiming>,
    current_voice: &str,
    defaults: TimingDefaults,
) {
    voices
        .entry(current_voice.to_owned())
        .or_insert_with(|| VoiceTiming::new(defaults));
}

fn apply_music_timing(
    element: &MusicElement<String>,
    span: SimpleSpan<usize>,
    voice: &mut VoiceTiming,
    options: BarDurationOptions,
    warnings: &mut Vec<ParseWarning<SimpleSpan<usize>>>,
) {
    match element {
        MusicElement::Note(note) => add_timed_group(note.length, span, voice),
        MusicElement::Rest(rest) => add_timed_group(rest.length, span, voice),
        MusicElement::Chord(chord) => {
            // ABC 2.1 §4.17 uses the first member's duration when chord
            // members differ and multiplies inner and outer length modifiers.
            let first_length = match chord.members.first() {
                Some(ChordMember::Note(note)) => note.length,
                Some(ChordMember::Rest(rest)) => rest.length,
                None => return,
            };
            add_timed_duration(
                Duration::from(voice.unit_length)
                    .multiply(Duration::from(first_length))
                    .multiply(Duration::from(chord.length)),
                span,
                voice,
            );
        }
        MusicElement::MultiMeasureRest(_) => skip_multi_measure_rest(span, voice),
        MusicElement::Tuplet(tuplet) => start_tuplet(*tuplet, voice),
        MusicElement::BrokenRhythm(rhythm) => {
            apply_broken_rhythm(rhythm.greater, rhythm.count, voice);
        }
        MusicElement::Bar(_) => complete_bar(voice, span, options, warnings),
        MusicElement::Overlay(_) => {
            flush_pending_timed_group(voice);
            make_bar_indeterminate(voice);
        }
        _ => {}
    }
}

fn add_timed_group(length: NoteLength, span: SimpleSpan<usize>, voice: &mut VoiceTiming) {
    // ABC 2.1 §4.3 defines written note lengths as multipliers of L:.
    let duration = Duration::from(voice.unit_length).multiply(Duration::from(length));
    add_timed_duration(duration, span, voice);
}

fn add_timed_duration(mut duration: Duration, span: SimpleSpan<usize>, voice: &mut VoiceTiming) {
    flush_pending_timed_group(voice);
    begin_timed_group(span, voice);

    for tuplet in &mut voice.tuplets {
        duration = duration.multiply(tuplet.factor);
        tuplet.remaining = tuplet.remaining.saturating_sub(1);
    }
    voice.tuplets.retain(|tuplet| tuplet.remaining != 0);
    if let Some(factor) = voice.bar.next_broken_rhythm_factor.take() {
        duration = duration.multiply(factor);
    }
    voice.bar.pending_timed_group = Some(duration);
}

fn skip_multi_measure_rest(span: SimpleSpan<usize>, voice: &mut VoiceTiming) {
    // ABC 2.1 §4.5 defines Z and X in whole measures. Skip their containing
    // bar instead of expanding the encoded measure count.
    flush_pending_timed_group(voice);
    begin_timed_group(span, voice);
    make_bar_indeterminate(voice);
}

fn begin_timed_group(span: SimpleSpan<usize>, voice: &mut VoiceTiming) {
    if !voice.bar.seen_timed_group {
        voice.bar.expected = if voice.bar.indeterminate {
            None
        } else {
            voice.meter.map(|meter| meter.duration)
        };
    }
    voice.bar.seen_timed_group = true;
    voice.bar.last_span = Some(span);
}

fn start_tuplet(tuplet: Tuplet, voice: &mut VoiceTiming) {
    // ABC 2.1 §4.13 defines (p:q:r) as p notes in the time of q.
    let compound = voice.meter.is_some_and(|meter| meter.compound);
    let Some(normal) = tuplet.normal_note_count(compound) else {
        make_bar_indeterminate(voice);
        return;
    };
    if tuplet.actual == 0 || tuplet.affected_note_count() == 0 {
        make_bar_indeterminate(voice);
        return;
    }
    voice.tuplets.push(ActiveTuplet {
        factor: Duration::new(u128::from(normal), u128::from(tuplet.actual)),
        remaining: tuplet.affected_note_count(),
    });
}

fn apply_broken_rhythm(greater: bool, count: u8, voice: &mut VoiceTiming) {
    // ABC 2.1 §4.4 makes n markers scale the long side by
    // (2^(n + 1) - 1) / 2^n and the short side by 1 / 2^n.
    let Some(power) = 1_u128.checked_shl(u32::from(count)) else {
        make_bar_indeterminate(voice);
        return;
    };
    let Some(long_numerator) = power.checked_mul(2).and_then(|value| value.checked_sub(1)) else {
        make_bar_indeterminate(voice);
        return;
    };
    let long = Duration::new(long_numerator, power);
    let short = Duration::new(1, power);
    let Some(previous) = voice.bar.pending_timed_group.take() else {
        make_bar_indeterminate(voice);
        return;
    };
    voice.bar.duration =
        voice
            .bar
            .duration
            .add(previous.multiply(if greater { long } else { short }));
    voice.bar.next_broken_rhythm_factor = Some(if greater { short } else { long });
}

const fn flush_pending_timed_group(voice: &mut VoiceTiming) {
    if let Some(duration) = voice.bar.pending_timed_group.take() {
        voice.bar.duration = voice.bar.duration.add(duration);
    }
}

const fn make_bar_indeterminate(voice: &mut VoiceTiming) {
    voice.bar.indeterminate = true;
    voice.bar.expected = None;
}

fn complete_bar(
    voice: &mut VoiceTiming,
    span: SimpleSpan<usize>,
    options: BarDurationOptions,
    warnings: &mut Vec<ParseWarning<SimpleSpan<usize>>>,
) {
    flush_pending_timed_group(voice);
    if voice.bar.seen_timed_group {
        let duration = voice.bar.duration;
        let expected = voice.bar.expected;
        process_completed_bar(voice, duration, expected, span, options, warnings);
    }
    voice.bar = BarTiming::default();
}

fn process_completed_bar(
    voice: &mut VoiceTiming,
    duration: Duration,
    expected: Option<Duration>,
    span: SimpleSpan<usize>,
    options: BarDurationOptions,
    warnings: &mut Vec<ParseWarning<SimpleSpan<usize>>>,
) {
    let Some(expected) = expected else {
        return;
    };
    let position = voice.completed_bars;
    voice.completed_bars += 1;
    let opening_bar = voice.opening_bar_available;
    voice.opening_bar_available = false;
    if duration == expected {
        return;
    }
    let bar = BarDuration {
        duration,
        expected,
        beat_denominator: voice
            .meter
            .expect("a determinate expected duration has an effective meter")
            .beat_denominator,
        position,
        span,
    };
    if duration.is_shorter_than(expected) {
        if options.pickup_policy == BarDurationPickupPolicy::OpeningBar && opening_bar {
            return;
        }
        if options.pickup_policy == BarDurationPickupPolicy::FirstAndLast {
            voice.incomplete_bars.push(bar);
            return;
        }
    }
    warnings.push(bar_duration_warning(&bar));
}

fn finish_pickup_section(
    voice: &mut VoiceTiming,
    pickup_policy: BarDurationPickupPolicy,
    warnings: &mut Vec<ParseWarning<SimpleSpan<usize>>>,
) {
    let bars = std::mem::take(&mut voice.incomplete_bars);
    let completed_bars = std::mem::take(&mut voice.completed_bars);
    if bars.is_empty() {
        return;
    }
    // ABC 2.1 does not define pickup inference. Lint accepts only a
    // complementary pair at the boundaries of one metered section.
    debug_assert_eq!(pickup_policy, BarDurationPickupPolicy::FirstAndLast);

    let boundary_pair = bars.len() >= 2
        && bars.first().is_some_and(|bar| bar.position == 0)
        && bars
            .last()
            .is_some_and(|bar| bar.position + 1 == completed_bars)
        && bars_are_complementary(
            bars.first().expect("a boundary pair has a first bar"),
            bars.last().expect("a boundary pair has a last bar"),
        );
    for (index, bar) in bars.iter().enumerate() {
        if !boundary_pair || (index != 0 && index + 1 != bars.len()) {
            warnings.push(bar_duration_warning(bar));
        }
    }
}

fn bars_are_complementary(first: &BarDuration, last: &BarDuration) -> bool {
    first.expected == last.expected && first.duration.add(last.duration) == first.expected
}

fn bar_duration_warning(bar: &BarDuration) -> ParseWarning<SimpleSpan<usize>> {
    let beat_scale = Duration::new(bar.beat_denominator, 1);
    let duration = bar.duration.multiply(beat_scale);
    let expected = bar.expected.multiply(beat_scale);
    ParseWarning {
        kind: ErrorKind::InvalidMusic,
        message: format!(
            "bar duration is {duration} {}, expected {expected} {} under the effective meter",
            beat_unit(duration),
            beat_unit(expected),
        ),
        span: bar.span,
    }
}

const fn beat_unit(duration: Duration) -> &'static str {
    if duration.numerator == duration.denominator {
        "beat"
    } else {
        "beats"
    }
}

fn effective_meter(meter: &Meter) -> Option<EffectiveMeter> {
    // ABC 2.1 §3.1.6 defines C as 4/4, C| as 2/2, none as free meter,
    // and an additive numerator as the sum of its groups.
    match meter {
        Meter::Common | Meter::Cut => Some(EffectiveMeter {
            duration: Duration::ONE,
            beat_denominator: if matches!(meter, Meter::Cut) { 2 } else { 4 },
            compound: false,
        }),
        Meter::None => None,
        Meter::Simple(fraction) => Some(EffectiveMeter {
            duration: Duration::from(*fraction),
            beat_denominator: u128::from(fraction.denominator),
            compound: fraction.numerator > 3 && fraction.numerator % 3 == 0,
        }),
        Meter::Compound {
            groups,
            denominator,
        } => {
            let numerator = groups.iter().map(|group| u128::from(*group)).sum();
            Some(EffectiveMeter {
                duration: Duration::new(numerator, u128::from(*denominator)),
                beat_denominator: u128::from(*denominator),
                compound: numerator > 3 && numerator % 3 == 0,
            })
        }
    }
}

fn default_unit_length(meter: &Meter) -> Fraction {
    let short_meter = effective_meter(meter)
        .is_some_and(|meter| meter.duration.is_shorter_than(Duration::THREE_QUARTERS));
    if short_meter {
        SIXTEENTH_NOTE
    } else {
        EIGHTH_NOTE
    }
}

const fn greatest_common_divisor(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    if left == 0 { 1 } else { left }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IntoOwnedAst;
    use crate::parse;

    fn warnings(source: &str, options: BarDurationOptions) -> Vec<ParseWarning<SimpleSpan<usize>>> {
        let document = parse(source)
            .output
            .expect("test input should parse")
            .into_owned(source)
            .expect("test source should resolve");
        bar_duration_warnings(&document, options)
    }

    #[test]
    fn opening_pickup_is_local_and_trailing_bar_is_optional() {
        let source = "X:1\nM:4/4\nL:1/4\nK:C\nC | C | CDEF | C\n";
        let options = BarDurationOptions::new()
            .pickup_policy(BarDurationPickupPolicy::OpeningBar)
            .check_trailing_bar(false);
        let found = warnings(source, options);
        assert_eq!(found.len(), 1);
        assert!(found[0].message.starts_with("bar duration is 1 beat"));
    }

    #[test]
    fn opening_pickup_resets_for_each_meter_section() {
        let source = "X:1\nM:4/4\nL:1/4\nK:C\nC | CDEF | [M:3/4] C | CDE |\n";
        let options = BarDurationOptions::new().pickup_policy(BarDurationPickupPolicy::OpeningBar);
        assert!(warnings(source, options).is_empty());
    }

    #[test]
    fn opening_pickup_does_not_hide_an_overfull_bar() {
        let source = "X:1\nM:4/4\nL:1/4\nK:C\nCDEFG |\n";
        let options = BarDurationOptions::new().pickup_policy(BarDurationPickupPolicy::OpeningBar);
        let found = warnings(source, options);
        assert_eq!(found.len(), 1);
        assert!(found[0].message.starts_with("bar duration is 5 beats"));
    }

    #[test]
    fn warning_formats_fractional_meter_denominator_beats() {
        let source = "X:1\nM:4/4\nL:1/8\nK:C\nA8 | ABCDEFG |\n";
        let found = warnings(source, BarDurationOptions::new());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].message,
            "bar duration is 3 1/2 beats, expected 4 beats under the effective meter"
        );
    }

    #[test]
    fn durations_display_as_reduced_mixed_numbers() {
        assert_eq!(Duration::new(8, 2).to_string(), "4");
        assert_eq!(Duration::new(2, 4).to_string(), "1/2");
        assert_eq!(Duration::new(14, 4).to_string(), "3 1/2");
    }
}
