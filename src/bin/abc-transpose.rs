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

//! Transposes every tune in an ABC file.
//!
//! The command accepts either a destination key for each tune or a signed
//! chromatic interval. Transposed notes use explicit accidentals, making the
//! emitted result independent of accidental propagation in the destination.

use abc_parser::Accidental;
use abc_parser::BarLine;
use abc_parser::ChordMember;
use abc_parser::Field;
use abc_parser::FieldValue;
use abc_parser::Fraction;
use abc_parser::KeyAccidental;
use abc_parser::KeySignature;
use abc_parser::KeyTonic;
use abc_parser::Line;
use abc_parser::MusicElement;
use abc_parser::Pitch;
use abc_parser::PitchClass;
use abc_parser::ToAbc;
use abc_parser::Tune;
use abc_parser::parse_field;
use abc_parser::parse_music_line;
use abc_parser::parse_recovering;
use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "Usage: abc-transpose <FILE|-> (--key <KEY> | --semitones <N> | --steps <N>)\n\
\n\
Transpose every tune in an ABC file and write canonical ABC to standard output.\n\
Use - as FILE to read standard input. KEY accepts ABC K: syntax with or without K:.\n\
Semitones are signed: 0 is unchanged, 1 is one semitone higher, and -1 is one lower.\n\
Steps are signed whole-tone steps and must be multiples of 0.5; 0.5 is one semitone.\n";

/// A requested transposition operation.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Request {
    /// Move every tune independently to this destination key.
    Key(KeySignature<String>),
    /// Move all pitches by this signed chromatic interval.
    Semitones(i16),
}

/// Fully validated command-line arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Arguments {
    input: PathBuf,
    request: Request,
}

/// A rational accidental displacement measured in semitones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Offset {
    numerator: i128,
    denominator: i128,
}

impl Offset {
    const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    /// Creates an integral semitone displacement.
    const fn integer(value: i16) -> Self {
        Self {
            numerator: value as i128,
            denominator: 1,
        }
    }

    /// Adds two rational semitone displacements without losing precision.
    const fn add(self, other: Self) -> Self {
        Self {
            numerator: self.numerator * other.denominator + other.numerator * self.denominator,
            denominator: self.denominator * other.denominator,
        }
    }

    /// Returns the same displacement relative to another displacement.
    const fn subtract(self, other: Self) -> Self {
        Self {
            numerator: self.numerator * other.denominator - other.numerator * self.denominator,
            denominator: self.denominator * other.denominator,
        }
    }

    /// Returns whether this displacement is exactly zero.
    const fn is_zero(self) -> bool {
        self.numerator == 0
    }
}

/// Accidental state for one written pitch and octave within the current bar.
type MeasureAccidentals = Vec<((PitchClass, i8), Offset)>;

/// Mutable musical state while one tune is traversed in source order.
struct TranspositionState {
    interval: i16,
    source_signature: [Offset; 7],
    measure_accidentals: MeasureAccidentals,
    prefer_flats: bool,
    first_key_pending: bool,
    destination: Option<KeySignature<String>>,
}

impl TranspositionState {
    /// Creates state using a tune's initial source key and requested interval.
    fn new(
        interval: i16,
        source_key: &KeySignature<String>,
        destination: Option<KeySignature<String>>,
    ) -> Result<Self, String> {
        Ok(Self {
            interval,
            source_signature: signature_offsets(source_key)?,
            measure_accidentals: Vec::new(),
            prefer_flats: destination
                .as_ref()
                .and_then(|key| key.tonic)
                .or(source_key.tonic)
                .is_some_and(|tonic| tonic.accidental == Some(KeyAccidental::Flat)),
            first_key_pending: true,
            destination,
        })
    }

    /// Resolves and then replaces one pitch, updating source accidental state.
    fn transpose_pitch(&mut self, pitch: &mut Pitch) -> Result<(), String> {
        let source = *pitch;
        let offset = match source.accidental {
            Some(accidental) => {
                let offset = accidental_offset(accidental);
                set_measure_accidental(
                    &mut self.measure_accidentals,
                    source.class,
                    source.octave,
                    offset,
                );
                offset
            }
            None => find_measure_accidental(&self.measure_accidentals, source.class, source.octave)
                .unwrap_or(self.source_signature[class_index(source.class)]),
        };
        *pitch = spell_absolute_pitch(source, offset, self.interval, self.prefer_flats)?;
        Ok(())
    }

    /// Applies a source key change and transposes its emitted key field.
    fn transpose_key(&mut self, key: &mut KeySignature<String>) -> Result<(), String> {
        let source_key = key.clone();
        self.source_signature = signature_offsets(&source_key)?;
        self.measure_accidentals.clear();

        if self.first_key_pending {
            self.first_key_pending = false;
            if let Some(destination) = &self.destination {
                *key = destination.clone();
                return Ok(());
            }
        }
        transpose_key_tonic(key, self.interval, self.prefer_flats)
    }
}

/// Parses arguments, transforms the input, and writes the resulting ABC.
fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() == 1 && matches!(arguments[0].as_str(), "-h" | "--help") {
        return match write_output(USAGE) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("abc-transpose: {message}");
                ExitCode::FAILURE
            }
        };
    }
    match run(arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("abc-transpose: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command for an arbitrary argument iterator.
fn run(arguments: impl IntoIterator<Item = String>) -> Result<(), String> {
    let arguments = parse_arguments(arguments)?;
    let source = read_source(&arguments.input)?;
    let parsed = parse_recovering(&source);
    if !parsed.is_valid() {
        let diagnostics = parsed
            .errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("input is not valid ABC:\n{diagnostics}"));
    }

    if arguments.request == Request::Semitones(0) {
        return write_output(&source);
    }

    let mut document = parsed.output;
    for tune in document.tunes_mut() {
        transpose_tune(tune, &arguments.request)?;
    }
    write_output(&document.to_abc())
}

/// Parses the file path and exactly one mutually exclusive operation.
fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut arguments = arguments.into_iter();
    let Some(first) = arguments.next() else {
        return Err(USAGE.into());
    };
    if matches!(first.as_str(), "-h" | "--help") {
        return Err(USAGE.into());
    }
    let input = PathBuf::from(first);
    let Some(option) = arguments.next() else {
        return Err(USAGE.into());
    };
    let Some(value) = arguments.next() else {
        return Err(format!("{option} requires a value\n\n{USAGE}"));
    };
    if arguments.next().is_some() {
        return Err(format!("unexpected additional arguments\n\n{USAGE}"));
    }

    let request = match option.as_str() {
        "--key" => Request::Key(parse_key(&value)?),
        "--semitones" => Request::Semitones(
            value
                .parse()
                .map_err(|_| format!("invalid semitone count: {value}"))?,
        ),
        "--steps" => Request::Semitones(parse_steps(&value)?),
        _ => return Err(format!("unknown option: {option}\n\n{USAGE}")),
    };
    Ok(Arguments { input, request })
}

/// Converts exact half-step notation into its integral semitone equivalent.
fn parse_steps(value: &str) -> Result<i16, String> {
    let (negative, magnitude) = if let Some(magnitude) = value.strip_prefix('-') {
        (true, magnitude)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    let (whole, fraction) = magnitude.split_once('.').unwrap_or((magnitude, ""));
    if (whole.is_empty() && fraction.is_empty())
        || !whole.chars().all(|character| character.is_ascii_digit())
    {
        return Err(format!("invalid step count: {value}"));
    }
    let half = match fraction {
        "" => 0_i16,
        digits if digits.chars().all(|digit| digit == '0') => 0,
        digits if digits.starts_with('5') && digits[1..].chars().all(|digit| digit == '0') => 1,
        _ => {
            return Err(format!(
                "step count must be an exact multiple of 0.5: {value}"
            ));
        }
    };
    let whole: i16 = if whole.is_empty() {
        0
    } else {
        whole
            .parse()
            .map_err(|_| format!("step count is out of range: {value}"))?
    };
    let semitones = whole
        .checked_mul(2)
        .and_then(|semitones| semitones.checked_add(half))
        .ok_or_else(|| format!("step count is out of range: {value}"))?;
    Ok(if negative { -semitones } else { semitones })
}

/// Parses a destination key using the library's structured `K:` parser.
fn parse_key(value: &str) -> Result<KeySignature<String>, String> {
    let value = value.strip_prefix("K:").unwrap_or(value);
    let field = parse_field(&format!("K:{value}"))
        .map_err(|error| format!("invalid destination key: {error}"))?;
    let FieldValue::Key(key) = field.value else {
        return Err("destination did not parse as a key signature".into());
    };
    if key.tonic.is_none() {
        return Err("destination key must have a pitched tonic".into());
    }
    Ok(key)
}

/// Reads the named file, or standard input when the path is `-`.
fn read_source(path: &PathBuf) -> Result<String, String> {
    if path.as_os_str() == "-" {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("could not read standard input: {error}"))?;
        Ok(source)
    } else {
        fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
    }
}

/// Writes all output bytes and reports broken pipes or other I/O errors.
fn write_output(source: &str) -> Result<(), String> {
    io::stdout()
        .write_all(source.as_bytes())
        .map_err(|error| format!("could not write standard output: {error}"))
}

/// Transposes one tune using its first structured key field as the source key.
fn transpose_tune<S>(tune: &mut Tune<S, String>, request: &Request) -> Result<(), String> {
    let source_key = tune
        .lines
        .iter()
        .find_map(|line| match &line.value {
            Line::Field(Field {
                value: FieldValue::Key(key),
                ..
            }) => Some(key.clone()),
            _ => None,
        })
        .ok_or_else(|| "a tune has no K: field".to_owned())?;
    let source_tonic = source_key
        .tonic
        .ok_or_else(|| "cannot transpose a tune whose key has no pitched tonic".to_owned())?;
    let (interval, destination) = match request {
        Request::Semitones(interval) => (*interval, None),
        Request::Key(destination) => {
            let destination_tonic = destination
                .tonic
                .expect("parse_key rejects destination keys without a tonic");
            (
                signed_tonic_interval(source_tonic, destination_tonic),
                Some(destination.clone()),
            )
        }
    };
    let mut state = TranspositionState::new(interval, &source_key, destination)?;
    for line in &mut tune.lines {
        transpose_line(&mut line.value, &mut state)?;
    }
    Ok(())
}

/// Transposes fields and pitch-bearing elements on one physical line.
fn transpose_line<S>(
    line: &mut Line<S, String>,
    state: &mut TranspositionState,
) -> Result<(), String> {
    match line {
        Line::Field(field) => transpose_field(field, state),
        Line::Music(elements) => {
            for element in elements {
                transpose_music_element(&mut element.value, state)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Applies a key change while leaving unrelated information fields untouched.
fn transpose_field(
    field: &mut Field<String>,
    state: &mut TranspositionState,
) -> Result<(), String> {
    if let FieldValue::Key(key) = &mut field.value {
        state.transpose_key(key)?;
    }
    Ok(())
}

/// Transposes notes, chords, grace notes, inline keys, and bar state.
fn transpose_music_element(
    element: &mut MusicElement<String>,
    state: &mut TranspositionState,
) -> Result<(), String> {
    match element {
        MusicElement::Note(note) => state.transpose_pitch(&mut note.pitch),
        MusicElement::Chord(chord) => {
            for member in &mut chord.members {
                if let ChordMember::Note(note) = member {
                    state.transpose_pitch(&mut note.pitch)?;
                }
            }
            Ok(())
        }
        MusicElement::Grace(group) => {
            for note in &mut group.notes {
                state.transpose_pitch(&mut note.pitch)?;
            }
            Ok(())
        }
        MusicElement::InlineField(field) => transpose_field(field, state),
        MusicElement::Bar(BarLine { .. }) => {
            state.measure_accidentals.clear();
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Converts a parsed accidental to its signed rational semitone offset.
fn accidental_offset(accidental: Accidental) -> Offset {
    match accidental {
        Accidental::Natural => Offset::ZERO,
        Accidental::Sharp(value) => Offset {
            numerator: i128::from(value.numerator),
            denominator: i128::from(value.denominator),
        },
        Accidental::Flat(value) => Offset {
            numerator: -i128::from(value.numerator),
            denominator: i128::from(value.denominator),
        },
    }
}

/// Produces an explicit accidental from a rational semitone offset.
fn offset_accidental(offset: Offset) -> Result<Accidental, String> {
    if offset.is_zero() {
        return Ok(Accidental::Natural);
    }
    let divisor = greatest_common_divisor(
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

/// Calculates a greatest common divisor for normalizing accidental fractions.
const fn greatest_common_divisor(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

/// Spells an absolute transposed pitch, including an explicit accidental.
fn spell_absolute_pitch(
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
    let chromatic =
        u8::try_from(anchor.rem_euclid(12)).expect("a value modulo twelve always fits in u8");
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

/// Returns the mathematical ceiling of a rational value.
const fn ceiling_fraction(value: Offset) -> i128 {
    -(-value.numerator).div_euclid(value.denominator)
}

/// Selects a conventional sharp- or flat-oriented chromatic spelling.
const fn chromatic_spelling(chromatic: u8, prefer_flats: bool) -> (PitchClass, i16) {
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

/// Moves a key tonic chromatically while preserving its mode and parameters.
fn transpose_key_tonic(
    key: &mut KeySignature<String>,
    interval: i16,
    prefer_flats: bool,
) -> Result<(), String> {
    let tonic = key
        .tonic
        .ok_or_else(|| "cannot transpose a key without a pitched tonic".to_owned())?;
    let chromatic = (tonic_semitone(tonic) + interval).rem_euclid(12);
    let (class, accidental) = chromatic_spelling(
        u8::try_from(chromatic).expect("a value modulo twelve always fits in u8"),
        prefer_flats,
    );
    key.tonic = Some(KeyTonic {
        class,
        accidental: match accidental {
            -1 => Some(KeyAccidental::Flat),
            1 => Some(KeyAccidental::Sharp),
            _ => None,
        },
    });
    Ok(())
}

/// Finds the shortest signed tonic interval, preferring upward tritones.
const fn signed_tonic_interval(source: KeyTonic, destination: KeyTonic) -> i16 {
    let upward = (tonic_semitone(destination) - tonic_semitone(source)).rem_euclid(12);
    if upward > 6 { upward - 12 } else { upward }
}

/// Returns the chromatic semitone number of a key tonic.
const fn tonic_semitone(tonic: KeyTonic) -> i16 {
    natural_semitone(tonic.class)
        + match tonic.accidental {
            Some(KeyAccidental::Sharp) => 1,
            Some(KeyAccidental::Flat) => -1,
            None => 0,
        }
}

/// Builds the seven implicit accidental offsets of a conventional key.
fn signature_offsets(key: &KeySignature<String>) -> Result<[Offset; 7], String> {
    let tonic = key
        .tonic
        .ok_or_else(|| "cannot resolve a key signature without a pitched tonic".to_owned())?;
    let degree = mode_degree(&key.mode);
    let major_class = shift_class(tonic.class, -degree);
    let major_pitch = (tonic_semitone(tonic) - mode_semitones(degree)).rem_euclid(12);
    let accidental = normalize_accidental(major_pitch - natural_semitone(major_class));
    let fifths = major_key_fifths(major_class, accidental).ok_or_else(|| {
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

/// Parses a positional key parameter such as `^F`, `_B`, or `^1/2c`.
fn explicit_key_accidental(value: &str) -> Option<(PitchClass, Offset)> {
    if !matches!(value.as_bytes().first(), Some(b'^' | b'_' | b'=')) {
        return None;
    }
    let report = parse_music_line(value);
    if !report.is_valid() || report.output.len() != 1 {
        return None;
    }
    let MusicElement::Note(note) = &report.output[0].value else {
        return None;
    };
    if note.length.numerator != 1 || note.length.denominator != 1 {
        return None;
    }
    note.pitch
        .accidental
        .map(|accidental| (note.pitch.class, accidental_offset(accidental)))
}

/// Maps common ABC mode spellings to their scale degree above relative major.
fn mode_degree(mode: &str) -> i8 {
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

/// Returns the major-scale semitone distance for a diatonic degree.
const fn mode_semitones(degree: i8) -> i16 {
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

/// Normalizes a chromatic difference to the nearest written accidental.
const fn normalize_accidental(value: i16) -> i16 {
    let value = value.rem_euclid(12);
    if value > 6 { value - 12 } else { value }
}

/// Maps a conventional written major tonic to its circle-of-fifths count.
const fn major_key_fifths(class: PitchClass, accidental: i16) -> Option<i8> {
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

/// Returns a pitch class at a signed diatonic distance from another class.
const fn shift_class(class: PitchClass, steps: i8) -> PitchClass {
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
const fn class_number(class: PitchClass) -> i8 {
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
const fn class_index(class: PitchClass) -> usize {
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
const fn natural_semitone(class: PitchClass) -> i16 {
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

/// Records an explicitly written source accidental for subsequent notes.
fn set_measure_accidental(
    accidentals: &mut MeasureAccidentals,
    class: PitchClass,
    octave: i8,
    offset: Offset,
) {
    if let Some((_, stored)) = accidentals
        .iter_mut()
        .find(|((stored_class, stored_octave), _)| {
            *stored_class == class && *stored_octave == octave
        })
    {
        *stored = offset;
    } else {
        accidentals.push(((class, octave), offset));
    }
}

/// Finds a source accidental carried earlier in the current measure.
fn find_measure_accidental(
    accidentals: &MeasureAccidentals,
    class: PitchClass,
    octave: i8,
) -> Option<Offset> {
    accidentals
        .iter()
        .find(|((stored_class, stored_octave), _)| {
            *stored_class == class && *stored_octave == octave
        })
        .map(|(_, offset)| *offset)
}

#[cfg(test)]
mod tests {
    use super::Request;
    use super::parse_key;
    use super::parse_steps;
    use super::transpose_tune;
    use abc_parser::ToAbc;
    use abc_parser::parse_recovering;

    /// Parses and transposes the only tune in a compact fixture.
    fn transpose(source: &str, request: &Request) -> String {
        let mut document = parse_recovering(source).output;
        {
            let mut tunes = document.tunes_mut();
            let tune = tunes.next().unwrap();
            assert!(tunes.next().is_none());
            transpose_tune(tune, request).unwrap();
        }
        document.to_abc()
    }

    #[test]
    fn signed_semitones_transpose_keys_notes_chords_and_graces() {
        let output = transpose("X:1\nK:C\nC ^C C | [EG] {B} |\n", &Request::Semitones(1));
        assert!(output.contains("K:C#"));
        assert!(output.contains("^C =D =D | [=F^G] {=c} |"));
    }

    #[test]
    fn destination_key_is_applied_to_each_tune() {
        let output = transpose(
            "X:1\nK:C\nC E G |\n",
            &Request::Key(parse_key("D").unwrap()),
        );
        assert!(output.contains("K:D"));
        assert!(output.contains("=D ^F =A |"));
    }

    #[test]
    fn source_key_and_measure_accidentals_affect_sounding_pitch() {
        let output = transpose("X:1\nK:G\nF =F F | F |\n", &Request::Semitones(1));
        assert!(output.contains("K:G#"));
        assert!(output.contains("=G ^F ^F | =G |"), "{output}");
    }

    #[test]
    fn explicit_key_signature_accidentals_are_resolved() {
        let output = transpose("X:1\nK:C exp ^F\nF |\n", &Request::Semitones(1));
        assert!(output.contains("=G |"), "{output}");
    }

    #[test]
    fn steps_accept_only_exact_half_increments() {
        assert_eq!(parse_steps("0").unwrap(), 0);
        assert_eq!(parse_steps("0.5").unwrap(), 1);
        assert_eq!(parse_steps(".5").unwrap(), 1);
        assert_eq!(parse_steps("1").unwrap(), 2);
        assert_eq!(parse_steps("-1.5").unwrap(), -3);
        assert!(parse_steps("0.25").is_err());
        assert!(parse_steps("-+0.5").is_err());
        assert!(parse_steps("half").is_err());
    }
}
