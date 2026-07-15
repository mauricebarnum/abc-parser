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
//! chromatic interval, plus an optional octave displacement. Transposed notes
//! use explicit accidentals, making the emitted result independent of
//! accidental propagation in the destination.

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
use clap::ArgGroup;
use clap::Parser;
use clap::ValueEnum;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// A requested transposition operation.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Request {
    /// Move every tune independently to this destination key.
    Key(KeySignature<String>),
    /// Move all pitches by this signed chromatic interval.
    Semitones(i16),
}

/// Requested policy for choosing enharmonic spellings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum SpellingPreference {
    /// Select the conventional destination having the smallest key signature.
    #[value(name = "auto")]
    #[default]
    Auto,
    /// Prefer flat-oriented conventional destinations.
    #[value(name = "true")]
    Flats,
    /// Prefer sharp-oriented conventional destinations.
    #[value(name = "false")]
    Sharps,
}

impl SpellingPreference {
    /// Returns a forced orientation, or `None` when heuristics should decide.
    const fn forced_flats(self) -> Option<bool> {
        match self {
            Self::Auto => None,
            Self::Flats => Some(true),
            Self::Sharps => Some(false),
        }
    }
}

/// Command-line arguments for transposing an ABC file.
#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "abc-transpose",
    about = "Transpose every tune in an ABC file",
    long_about = "Transpose every tune in an ABC file and write canonical ABC to standard output or a selected file."
)]
#[command(group(
    ArgGroup::new("transposition")
        .required(true)
        .multiple(false)
        .args(["key", "semitones", "steps"])
))]
struct Arguments {
    /// ABC input file, or - to read standard input.
    input: PathBuf,
    /// Destination key, in ABC K: syntax with or without K:.
    #[arg(long, value_name = "KEY", value_parser = parse_key)]
    key: Option<KeySignature<String>>,
    /// Signed chromatic interval; 1 is one semitone higher.
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    semitones: Option<i16>,
    /// Signed whole-tone steps, in exact multiples of 0.5.
    #[arg(long, value_name = "N", allow_hyphen_values = true, value_parser = parse_steps)]
    steps: Option<i16>,
    /// Raise or lower every note by this many octaves.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    octave: i16,
    /// Override automatic enharmonic spelling selection.
    #[arg(
        long = "prefer-flats",
        value_enum,
        default_value = "auto",
        value_name = "BOOL"
    )]
    spelling: SpellingPreference,
    /// Write output to this file instead of standard output.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

impl Arguments {
    /// Returns the mutually exclusive transposition requested by the user.
    fn request(&self) -> Request {
        if let Some(key) = &self.key {
            Request::Key(key.clone())
        } else if let Some(interval) = self.semitones {
            Request::Semitones(interval)
        } else {
            Request::Semitones(
                self.steps
                    .expect("clap requires exactly one transposition operation"),
            )
        }
    }
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

/// Accidental state for each written pitch class within the current bar.
type MeasureAccidentals = Vec<(PitchClass, Offset)>;

/// Mutable musical state while one tune is traversed in source order.
struct TranspositionState {
    interval: i16,
    pitch_interval: i16,
    spelling: SpellingPreference,
    source_signature: [Offset; 7],
    source_measure_accidentals: MeasureAccidentals,
    destination_signature: [Offset; 7],
    destination_measure_accidentals: MeasureAccidentals,
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
        spelling: SpellingPreference,
        octave: i16,
    ) -> Result<Self, String> {
        let pitch_interval = octave
            .checked_mul(12)
            .and_then(|octave_interval| interval.checked_add(octave_interval))
            .ok_or_else(|| "combined transposition interval is out of range".to_owned())?;
        Ok(Self {
            interval,
            pitch_interval,
            spelling,
            source_signature: signature_offsets(source_key)?,
            source_measure_accidentals: Vec::new(),
            destination_signature: signature_offsets(source_key)?,
            destination_measure_accidentals: Vec::new(),
            prefer_flats: spelling.forced_flats().unwrap_or_else(|| {
                destination
                    .as_ref()
                    .map_or_else(|| key_prefers_flats(source_key), key_prefers_flats)
            }),
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
                set_measure_accidental(&mut self.source_measure_accidentals, source.class, offset);
                offset
            }
            None => find_measure_accidental(&self.source_measure_accidentals, source.class)
                .unwrap_or(self.source_signature[class_index(source.class)]),
        };
        let mut destination =
            spell_absolute_pitch(source, offset, self.pitch_interval, self.prefer_flats)?;
        let destination_offset = destination
            .accidental
            .map(accidental_offset)
            .expect("spell_absolute_pitch always emits an accidental");
        let active_offset =
            find_measure_accidental(&self.destination_measure_accidentals, destination.class)
                .unwrap_or(self.destination_signature[class_index(destination.class)]);
        if destination_offset == active_offset {
            destination.accidental = None;
        } else {
            set_measure_accidental(
                &mut self.destination_measure_accidentals,
                destination.class,
                destination_offset,
            );
        }
        *pitch = destination;
        Ok(())
    }

    /// Applies a source key change and transposes its emitted key field.
    fn transpose_key(&mut self, key: &mut KeySignature<String>) -> Result<(), String> {
        let source_key = key.clone();
        self.source_signature = signature_offsets(&source_key)?;
        self.source_measure_accidentals.clear();
        self.destination_measure_accidentals.clear();

        if self.first_key_pending {
            self.first_key_pending = false;
            if let Some(destination) = &self.destination {
                self.prefer_flats = self
                    .spelling
                    .forced_flats()
                    .unwrap_or_else(|| key_prefers_flats(destination));
                *key = destination.clone();
                self.destination_signature = signature_offsets(key).unwrap_or([Offset::ZERO; 7]);
                return Ok(());
            }
        }
        self.prefer_flats = transpose_key_tonic(key, self.interval, self.spelling)?;
        self.destination_signature = signature_offsets(key)?;
        Ok(())
    }
}

/// Parses arguments, transforms the input, and writes the resulting ABC.
fn main() -> ExitCode {
    match run(&Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("abc-transpose: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the command with fully parsed arguments.
fn run(arguments: &Arguments) -> Result<(), String> {
    let request = arguments.request();
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

    if request == Request::Semitones(0)
        && arguments.octave == 0
        && arguments.spelling == SpellingPreference::Auto
    {
        return write_output(&source, arguments.out.as_ref());
    }

    let mut document = parsed.output;
    for tune in document.tunes_mut() {
        transpose_tune(tune, &request, arguments.spelling, arguments.octave)?;
    }
    write_output(&document.to_abc(), arguments.out.as_ref())
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

/// Writes all output bytes to the selected file or standard output.
fn write_output(source: &str, path: Option<&PathBuf>) -> Result<(), String> {
    if let Some(path) = path {
        fs::write(path, source)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    } else {
        io::stdout()
            .write_all(source.as_bytes())
            .map_err(|error| format!("could not write standard output: {error}"))
    }
}

/// Transposes one tune using its first structured key field as the source key.
fn transpose_tune<S>(
    tune: &mut Tune<S, String>,
    request: &Request,
    spelling: SpellingPreference,
    octave: i16,
) -> Result<(), String> {
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
    let mut state = TranspositionState::new(interval, &source_key, destination, spelling, octave)?;
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
            state.source_measure_accidentals.clear();
            state.destination_measure_accidentals.clear();
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

/// Moves a key tonic chromatically and returns its conventional spelling orientation.
fn transpose_key_tonic(
    key: &mut KeySignature<String>,
    interval: i16,
    spelling: SpellingPreference,
) -> Result<bool, String> {
    let tonic = key
        .tonic
        .ok_or_else(|| "cannot transpose a key without a pitched tonic".to_owned())?;
    let chromatic = (tonic_semitone(tonic) + interval).rem_euclid(12);
    let chromatic = u8::try_from(chromatic).expect("a value modulo twelve always fits in u8");
    let sharp = chromatic_tonic(chromatic, false);
    let flat = chromatic_tonic(chromatic, true);
    let sharp_fifths = conventional_key_fifths(sharp, &key.mode);
    let flat_fifths = conventional_key_fifths(flat, &key.mode);
    let source_fifths = conventional_key_fifths(tonic, &key.mode);
    let selected = match spelling {
        SpellingPreference::Flats => flat_fifths
            .map(|count| (flat, count))
            .or_else(|| sharp_fifths.map(|count| (sharp, count))),
        SpellingPreference::Sharps => sharp_fifths
            .map(|count| (sharp, count))
            .or_else(|| flat_fifths.map(|count| (flat, count))),
        SpellingPreference::Auto => select_automatic_spelling(
            sharp,
            sharp_fifths,
            flat,
            flat_fifths,
            source_fifths,
            interval,
        ),
    }
    .ok_or_else(|| {
        format!(
            "transposed key {} has no conventional seven-accidental spelling",
            key.to_abc()
        )
    })?;
    key.tonic = Some(selected.0);
    Ok(match spelling {
        SpellingPreference::Flats if flat_fifths.is_some() => true,
        SpellingPreference::Sharps if sharp_fifths.is_some() => false,
        SpellingPreference::Auto | SpellingPreference::Flats | SpellingPreference::Sharps => {
            selected.1 < 0
        }
    })
}

/// Chooses the smallest signature, using direction and source orientation for ties.
fn select_automatic_spelling(
    sharp: KeyTonic,
    sharp_fifths: Option<i8>,
    flat: KeyTonic,
    flat_fifths: Option<i8>,
    source_fifths: Option<i8>,
    interval: i16,
) -> Option<(KeyTonic, i8)> {
    match (sharp_fifths, flat_fifths) {
        (Some(sharp_count), Some(flat_count))
            if sharp == flat || sharp_count.unsigned_abs() < flat_count.unsigned_abs() =>
        {
            Some((sharp, sharp_count))
        }
        (Some(sharp_count), Some(flat_count))
            if flat_count.unsigned_abs() < sharp_count.unsigned_abs() =>
        {
            Some((flat, flat_count))
        }
        (Some(_), Some(flat_count)) if interval < 0 => Some((flat, flat_count)),
        (Some(sharp_count), Some(_)) if interval > 0 => Some((sharp, sharp_count)),
        (Some(_), Some(flat_count)) if source_fifths.is_some_and(|count| count < 0) => {
            Some((flat, flat_count))
        }
        (Some(sharp_count), Some(_) | None) => Some((sharp, sharp_count)),
        (None, Some(flat_count)) => Some((flat, flat_count)),
        (None, None) => None,
    }
}

/// Creates a key tonic from one sharp- or flat-oriented chromatic spelling.
const fn chromatic_tonic(chromatic: u8, prefer_flats: bool) -> KeyTonic {
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
fn key_prefers_flats(key: &KeySignature<String>) -> bool {
    key.tonic.is_some_and(|tonic| {
        conventional_key_fifths(tonic, &key.mode).map_or_else(
            || tonic.accidental == Some(KeyAccidental::Flat),
            |fifths| fifths < 0,
        )
    })
}

/// Finds a modal key's relative-major position on the conventional circle of fifths.
fn conventional_key_fifths(tonic: KeyTonic, mode: &str) -> Option<i8> {
    let degree = mode_degree(mode);
    let major_class = shift_class(tonic.class, -degree);
    let major_pitch = (tonic_semitone(tonic) - mode_semitones(degree)).rem_euclid(12);
    let accidental = normalize_accidental(major_pitch - natural_semitone(major_class));
    major_key_fifths(major_class, accidental)
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
fn set_measure_accidental(accidentals: &mut MeasureAccidentals, class: PitchClass, offset: Offset) {
    if let Some((_, stored)) = accidentals
        .iter_mut()
        .find(|(stored_class, _)| *stored_class == class)
    {
        *stored = offset;
    } else {
        accidentals.push((class, offset));
    }
}

/// Finds a source accidental carried earlier in the current measure.
fn find_measure_accidental(accidentals: &MeasureAccidentals, class: PitchClass) -> Option<Offset> {
    accidentals
        .iter()
        .find(|(stored_class, _)| *stored_class == class)
        .map(|(_, offset)| *offset)
}

#[cfg(test)]
mod tests {
    use super::Arguments;
    use super::Request;
    use super::SpellingPreference;
    use super::parse_key;
    use super::parse_steps;
    use super::transpose_tune;
    use abc_parser::ToAbc;
    use abc_parser::parse_recovering;
    use clap::Parser;

    /// Parses and transposes the only tune in a compact fixture.
    fn transpose(source: &str, request: &Request) -> String {
        transpose_with_options(source, request, SpellingPreference::Auto, 0)
    }

    /// Parses and transposes the only tune using an explicit spelling policy.
    fn transpose_with_spelling(
        source: &str,
        request: &Request,
        spelling: SpellingPreference,
    ) -> String {
        transpose_with_options(source, request, spelling, 0)
    }

    /// Parses and transposes the only tune using all pitch options.
    fn transpose_with_options(
        source: &str,
        request: &Request,
        spelling: SpellingPreference,
        octave: i16,
    ) -> String {
        let mut document = parse_recovering(source).output;
        {
            let mut tunes = document.tunes_mut();
            let tune = tunes.next().unwrap();
            assert!(tunes.next().is_none());
            transpose_tune(tune, request, spelling, octave).unwrap();
        }
        document.to_abc()
    }

    #[test]
    fn signed_semitones_transpose_keys_notes_chords_and_graces() {
        let output = transpose("X:1\nK:C\nC ^C C | [EG] {B} |\n", &Request::Semitones(1));
        assert!(output.contains("K:Db"), "{output}");
        assert!(output.contains("D =D D | [FA] {c} |"), "{output}");
    }

    #[test]
    fn octave_transposition_changes_notes_without_changing_the_key() {
        let upward = transpose_with_options(
            "X:1\nK:C\nC [EG] {B} |\n",
            &Request::Semitones(0),
            SpellingPreference::Auto,
            1,
        );
        assert!(upward.contains("K:C\nc [eg] {b} |"), "{upward}");

        let downward = transpose_with_options(
            "X:1\nK:C\nC [EG] {B} |\n",
            &Request::Semitones(0),
            SpellingPreference::Auto,
            -1,
        );
        assert!(downward.contains("K:C\nC, [E,G,] {B,} |"), "{downward}");
    }

    #[test]
    fn destination_key_and_measure_state_suppress_redundant_accidentals() {
        let output = transpose_with_options(
            "X:1\nK:Bb\nBcde fgab | bbag f=e^fg |\n",
            &Request::Key(parse_key("Eb").unwrap()),
            SpellingPreference::Auto,
            -1,
        );
        assert!(
            output.contains("K:Eb\nEFGA Bcde | eedc B=A=Bc |"),
            "{output}"
        );
    }

    #[test]
    fn measure_accidentals_carry_across_octaves_until_the_bar_line() {
        let output = transpose("X:1\nK:C\n^C c =C c | C |\n", &Request::Semitones(0));
        assert!(output.contains("^C c =C c | C |"), "{output}");
    }

    #[test]
    fn destination_key_is_applied_to_each_tune() {
        let output = transpose(
            "X:1\nK:C\nC E G |\n",
            &Request::Key(parse_key("D").unwrap()),
        );
        assert!(output.contains("K:D"));
        assert!(output.contains("D F A |"));
    }

    #[test]
    fn source_key_and_measure_accidentals_affect_sounding_pitch() {
        let output = transpose("X:1\nK:G\nF =F F | F |\n", &Request::Semitones(1));
        assert!(output.contains("K:Ab"), "{output}");
        assert!(output.contains("G _G G | G |"), "{output}");
    }

    #[test]
    fn explicit_key_signature_accidentals_are_resolved() {
        let output = transpose("X:1\nK:C exp ^F\nF |\n", &Request::Semitones(1));
        assert!(output.contains("=G |"), "{output}");
    }

    #[test]
    fn semitone_transposition_chooses_the_smallest_destination_signature() {
        let upward = transpose("X:1\nK:F\nF ^F |\n", &Request::Semitones(2));
        assert!(upward.contains("K:G"), "{upward}");
        assert!(upward.contains("G ^G |"), "{upward}");

        let downward = transpose("X:1\nK:F\nF ^F |\n", &Request::Semitones(-2));
        assert!(downward.contains("K:Eb"), "{downward}");
        assert!(downward.contains("E =E |"), "{downward}");
    }

    #[test]
    fn natural_tonic_flat_keys_prefer_flat_note_spellings() {
        let modal = transpose("X:1\nK:Dm\n^D |\n", &Request::Semitones(0));
        assert!(modal.contains("K:Dm"), "{modal}");
        assert!(modal.contains("_E |"), "{modal}");

        let explicit = transpose("X:1\nK:C\n^C |\n", &Request::Key(parse_key("F").unwrap()));
        assert!(explicit.contains("K:F"), "{explicit}");
        assert!(explicit.contains("_G |"), "{explicit}");
    }

    #[test]
    fn explicit_unconventional_destination_spelling_is_preserved() {
        let output = transpose("X:1\nK:C\nC |\n", &Request::Key(parse_key("D#").unwrap()));
        assert!(output.contains("K:D#"), "{output}");
        assert!(output.contains("^D |"), "{output}");
    }

    #[test]
    fn equal_signature_sizes_follow_transposition_direction() {
        let upward = transpose("X:1\nK:C\nC |\n", &Request::Semitones(6));
        assert!(upward.contains("K:F#"), "{upward}");
        assert!(upward.contains("F |"), "{upward}");

        let downward = transpose("X:1\nK:C\nC |\n", &Request::Semitones(-6));
        assert!(downward.contains("K:Gb"), "{downward}");
        assert!(downward.contains("G, |"), "{downward}");
    }

    #[test]
    fn key_changes_recompute_the_enharmonic_orientation() {
        let output = transpose("X:1\nK:C\n^C |\nK:F\nF ^F |\n", &Request::Semitones(1));
        assert!(output.contains("K:Db\n=D |"), "{output}");
        assert!(output.contains("K:F#\nF =G |"), "{output}");
    }

    #[test]
    fn forced_spelling_applies_to_every_key_change() {
        let source = "X:1\nK:C\nC |\nK:F\nF ^F |\n";
        let sharps =
            transpose_with_spelling(source, &Request::Semitones(1), SpellingPreference::Sharps);
        assert!(sharps.contains("K:C#\nC |"), "{sharps}");
        assert!(sharps.contains("K:F#\nF =G |"), "{sharps}");

        let flats =
            transpose_with_spelling(source, &Request::Semitones(1), SpellingPreference::Flats);
        assert!(flats.contains("K:Db\nD |"), "{flats}");
        assert!(flats.contains("K:Gb\nG =G |"), "{flats}");
    }

    #[test]
    fn forced_spelling_falls_back_to_a_conventional_enharmonic() {
        let output = transpose_with_spelling(
            "X:1\nK:C\nC |\n",
            &Request::Semitones(3),
            SpellingPreference::Sharps,
        );
        assert!(output.contains("K:Eb"), "{output}");
        assert!(output.contains("E |"), "{output}");
    }

    #[test]
    fn prefer_flats_accepts_all_values_in_either_option_order() {
        let before = Arguments::try_parse_from([
            "abc-transpose".to_owned(),
            "input.abc".to_owned(),
            "--prefer-flats".to_owned(),
            "true".to_owned(),
            "--steps".to_owned(),
            "1".to_owned(),
        ])
        .unwrap();
        assert_eq!(before.spelling, SpellingPreference::Flats);

        let after = Arguments::try_parse_from([
            "abc-transpose".to_owned(),
            "input.abc".to_owned(),
            "--semitones".to_owned(),
            "-2".to_owned(),
            "--prefer-flats".to_owned(),
            "false".to_owned(),
        ])
        .unwrap();
        assert_eq!(after.spelling, SpellingPreference::Sharps);

        let automatic = Arguments::try_parse_from([
            "abc-transpose".to_owned(),
            "input.abc".to_owned(),
            "--key".to_owned(),
            "F".to_owned(),
            "--prefer-flats".to_owned(),
            "auto".to_owned(),
        ])
        .unwrap();
        assert_eq!(automatic.spelling, SpellingPreference::Auto);
    }

    #[test]
    fn prefer_flats_rejects_invalid_and_duplicate_values() {
        let invalid = Arguments::try_parse_from([
            "abc-transpose".to_owned(),
            "input.abc".to_owned(),
            "--steps".to_owned(),
            "1".to_owned(),
            "--prefer-flats".to_owned(),
            "yes".to_owned(),
        ]);
        assert!(invalid.is_err());

        let duplicate = Arguments::try_parse_from([
            "abc-transpose".to_owned(),
            "input.abc".to_owned(),
            "--semitones".to_owned(),
            "1".to_owned(),
            "--prefer-flats".to_owned(),
            "true".to_owned(),
            "--prefer-flats".to_owned(),
            "false".to_owned(),
        ]);
        assert!(duplicate.is_err());
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
