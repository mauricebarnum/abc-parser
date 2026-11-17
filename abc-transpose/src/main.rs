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
//! accidental propagation in the destination. Chord symbols are transposed
//! to match the destination key or interval.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use abc_parser::AnnotationPlacement;
use abc_parser::BarLine;
use abc_parser::ChordMember;
use abc_parser::DiagnosticRenderer;
use abc_parser::Document;
use abc_parser::EmitOptions;
use abc_parser::Field;
use abc_parser::FieldValue;
use abc_parser::IntoOwnedAst;
use abc_parser::KeySignature;
use abc_parser::KeyTonic;
use abc_parser::Line;
use abc_parser::MeasureAccidentals;
use abc_parser::MusicElement;
use abc_parser::NoteLengthStyle;
use abc_parser::Offset;
use abc_parser::Pitch;
use abc_parser::PitchClass;
use abc_parser::ToAbc;
use abc_parser::Tune;
use abc_parser::accidental_offset;
use abc_parser::build_info::BuildInfo;
use abc_parser::build_info::check_version_and_exit_for;
use abc_parser::build_info::input_name;
use abc_parser::build_info::read_source;
use abc_parser::build_info_from_env;
use abc_parser::chromatic_spelling;
use abc_parser::chromatic_tonic;
use abc_parser::class_index;
use abc_parser::conventional_key_fifths;
use abc_parser::is_chord_boundary;
use abc_parser::is_valid_chord_type;
use abc_parser::key_prefers_flats;
use abc_parser::natural_semitone;
use abc_parser::normalize_key_mode;
use abc_parser::parse;
use abc_parser::parse_accidental;
use abc_parser::parse_bass_note;
use abc_parser::parse_field;
use abc_parser::signature_offsets;
use abc_parser::signed_tonic_interval;
use abc_parser::spell_absolute_pitch;
use abc_parser::starts_with_chord_root;
use abc_parser::tonic_semitone;
use clap::ArgGroup;
use clap::Parser;
use clap::ValueEnum;

const BUILD_INFO: BuildInfo = build_info_from_env!();

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
    /// Write shortened note lengths with explicit power-of-two denominators.
    #[arg(long)]
    explicit_note_lengths: bool,
    /// Write output to this file instead of standard output.
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// Print version information (combine with --verbose for detailed build info).
    #[arg(short = 'V', long)]
    version: bool,
    /// Emit verbose output (shows detailed build info when combined with --version).
    #[arg(short = 'v', long)]
    verbose: bool,
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

#[derive(Clone, Copy)]
struct VoiceState {
    source_signature: [Offset; 7],
    source_measure_accidentals: MeasureAccidentals,
    destination_signature: [Offset; 7],
    destination_measure_accidentals: MeasureAccidentals,
    prefer_flats: bool,
}

/// Mutable musical state while one tune is traversed in source order.
struct TranspositionState {
    interval: i16,
    pitch_interval: i16,
    spelling: SpellingPreference,
    defaults: VoiceState,
    active_voice: VoiceState,
    current_voice: String,
    voices: BTreeMap<String, VoiceState>,
    body_started: bool,
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
        let source_signature = signature_offsets(source_key)?;
        let (destination_signature, prefer_flats) = if let Some(dest) = &destination {
            let prefer_flats = spelling
                .forced_flats()
                .unwrap_or_else(|| key_prefers_flats(dest));
            (signature_offsets(dest)?, prefer_flats)
        } else {
            let mut initial_dest = source_key.clone();
            let prefer_flats = transpose_key_tonic(&mut initial_dest, interval, spelling)?;
            normalize_key_mode(&mut initial_dest);
            (signature_offsets(&initial_dest)?, prefer_flats)
        };
        let defaults = VoiceState {
            source_signature,
            source_measure_accidentals: MeasureAccidentals::default(),
            destination_signature,
            destination_measure_accidentals: MeasureAccidentals::default(),
            prefer_flats,
        };
        Ok(Self {
            interval,
            pitch_interval,
            spelling,
            defaults,
            active_voice: defaults,
            current_voice: String::new(),
            voices: BTreeMap::new(),
            body_started: false,
            first_key_pending: true,
            destination,
        })
    }

    const fn active_voice_mut(&mut self) -> &mut VoiceState {
        &mut self.active_voice
    }

    const fn active_prefer_flats(&self) -> bool {
        self.active_voice.prefer_flats
    }

    fn switch_voice(&mut self, id: &str) {
        if self.current_voice == id {
            return;
        }
        self.voices
            .insert(self.current_voice.clone(), self.active_voice);
        self.current_voice.clear();
        self.current_voice.push_str(id);
        self.active_voice = self.voices.get(id).copied().unwrap_or(self.defaults);
    }

    fn clear_active_measure_accidentals(&mut self) {
        let voice = self.active_voice_mut();
        voice.source_measure_accidentals.clear();
        voice.destination_measure_accidentals.clear();
    }

    /// Resolves and then replaces one pitch, updating source accidental state.
    fn transpose_pitch(&mut self, pitch: &mut Pitch) -> Result<(), String> {
        let pitch_interval = self.pitch_interval;
        let voice = self.active_voice_mut();
        let source = *pitch;
        let offset = match source.accidental {
            Some(accidental) => {
                let offset = accidental_offset(accidental);
                voice.source_measure_accidentals.set(source.class, offset);
                offset
            }
            None => voice
                .source_measure_accidentals
                .get(source.class)
                .unwrap_or_else(|| voice.source_signature[class_index(source.class)]),
        };
        let mut destination =
            spell_absolute_pitch(source, offset, pitch_interval, voice.prefer_flats)?;
        let destination_offset = destination
            .accidental
            .map(accidental_offset)
            .expect("spell_absolute_pitch always emits an accidental");
        let active_offset = voice
            .destination_measure_accidentals
            .get(destination.class)
            .unwrap_or_else(|| voice.destination_signature[class_index(destination.class)]);
        if destination_offset == active_offset {
            destination.accidental = None;
        } else {
            voice
                .destination_measure_accidentals
                .set(destination.class, destination_offset);
        }
        *pitch = destination;
        Ok(())
    }

    /// Applies a source key change and transposes its emitted key field.
    fn transpose_key(&mut self, key: &mut KeySignature<String>) -> Result<(), String> {
        let source_signature = signature_offsets(key)?;
        let (destination_signature, prefer_flats) = if self.first_key_pending {
            self.first_key_pending = false;
            if let Some(destination) = self.destination.take() {
                let prefer_flats = self
                    .spelling
                    .forced_flats()
                    .unwrap_or_else(|| key_prefers_flats(&destination));
                *key = destination;
                normalize_key_mode(key);
                (signature_offsets(key)?, prefer_flats)
            } else {
                let prefer_flats = transpose_key_tonic(key, self.interval, self.spelling)?;
                normalize_key_mode(key);
                (signature_offsets(key)?, prefer_flats)
            }
        } else {
            let prefer_flats = transpose_key_tonic(key, self.interval, self.spelling)?;
            normalize_key_mode(key);
            (signature_offsets(key)?, prefer_flats)
        };

        let next_state = VoiceState {
            source_signature,
            source_measure_accidentals: MeasureAccidentals::default(),
            destination_signature,
            destination_measure_accidentals: MeasureAccidentals::default(),
            prefer_flats,
        };
        if self.body_started {
            self.active_voice = next_state;
        } else {
            self.defaults = next_state;
            self.active_voice = next_state;
            for voice in self.voices.values_mut() {
                *voice = next_state;
            }
            self.body_started = true;
        }
        Ok(())
    }
}

/// Parses arguments, transforms the input, and writes the resulting ABC.
fn main() -> ExitCode {
    if let Some(exit_code) = check_version_and_exit_for(&BUILD_INFO, "abc-transpose") {
        return exit_code;
    }
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
    let parsed = parse(source.as_str());
    let input_name = input_name(&arguments.input);
    let mut diagnostic_renderer = DiagnosticRenderer::new(source.as_str());
    for warning in &parsed.warnings {
        eprintln!(
            "abc-transpose: warning: {input_name}:{}",
            diagnostic_renderer.render_warning(warning)
        );
    }
    if !parsed.is_valid() {
        let diagnostics = parsed
            .errors
            .iter()
            .map(|error| format!("{input_name}:{}", diagnostic_renderer.render_error(error)))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("input is not valid ABC:\n{diagnostics}"));
    }

    if request == Request::Semitones(0)
        && arguments.octave == 0
        && arguments.spelling == SpellingPreference::Auto
        && !arguments.explicit_note_lengths
    {
        return write_output(source, arguments.out.as_ref());
    }

    let parsed_document = parsed
        .output
        .ok_or_else(|| "parser did not produce a document".to_owned())?;
    let mut document = parsed_document
        .into_owned(source.as_str())
        .map_err(|error| format!("could not resolve parsed source text: {error}"))?;
    transpose_document(
        &mut document,
        &request,
        arguments.spelling,
        arguments.octave,
    )?;
    let note_length_style = if arguments.explicit_note_lengths {
        NoteLengthStyle::ExplicitDenominator
    } else {
        NoteLengthStyle::Shorthand
    };
    let options = EmitOptions::new().with_note_length_style(note_length_style);
    write_output(
        document.to_abc_with_options(options),
        arguments.out.as_ref(),
    )
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
    signature_offsets(&key)?;
    Ok(key)
}

/// Writes all output bytes to the selected file or standard output.
fn write_output(mut source: String, path: Option<&PathBuf>) -> Result<(), String> {
    if !source.ends_with('\n') {
        source.push('\n');
    }
    if let Some(path) = path {
        fs::write(path, &source)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    } else {
        io::stdout()
            .write_all(source.as_bytes())
            .map_err(|error| format!("could not write standard output: {error}"))
    }
}

/// Transposes file-header keys and all tunes in a document.
fn transpose_document<S>(
    document: &mut Document<S, String>,
    request: &Request,
    spelling: SpellingPreference,
    octave: i16,
) -> Result<(), String> {
    let mut header_key = None;
    for line in &mut document.header {
        if let Line::Field(Field {
            value: FieldValue::Key(key),
            ..
        }) = &mut line.value
        {
            let key_clone = key.clone();
            let tonic = key_clone
                .tonic
                .ok_or_else(|| "cannot transpose a key without a pitched tonic".to_owned())?;
            let (interval, destination) = match request {
                Request::Semitones(interval) => (*interval, None),
                Request::Key(destination) => {
                    let destination_tonic = destination
                        .tonic
                        .expect("parse_key rejects destination keys without a tonic");
                    (
                        signed_tonic_interval(tonic, destination_tonic),
                        Some(destination.clone()),
                    )
                }
            };
            let mut header_state =
                TranspositionState::new(interval, &key_clone, destination, spelling, octave)?;
            header_state.transpose_key(key)?;
            header_key = Some(key_clone);
        }
    }
    for tune in document.tunes_mut() {
        transpose_tune(tune, header_key.as_ref(), request, spelling, octave)?;
    }
    Ok(())
}

/// Transposes one tune using its first structured key field as the source key.
fn transpose_tune<S>(
    tune: &mut Tune<S, String>,
    header_key: Option<&KeySignature<String>>,
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
            }) => Some(key),
            _ => None,
        })
        .or(header_key)
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
    let mut state = TranspositionState::new(interval, source_key, destination, spelling, octave)?;
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
            state.body_started = true;
            for element in elements {
                transpose_music_element(&mut element.value, state)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Applies a key or voice change while leaving unrelated information fields untouched.
fn transpose_field(
    field: &mut Field<String>,
    state: &mut TranspositionState,
) -> Result<(), String> {
    match &mut field.value {
        FieldValue::Key(key) => state.transpose_key(key)?,
        FieldValue::Voice(voice) => state.switch_voice(&voice.id),
        _ => {}
    }
    Ok(())
}

/// Transposes notes, chords, grace notes, chord symbols, inline keys, and bar state.
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
            for element in &mut group.elements {
                if let abc_parser::GraceElement::Note(note) = element {
                    state.transpose_pitch(&mut note.pitch)?;
                }
            }
            Ok(())
        }
        MusicElement::InlineField(field) => transpose_field(field, state),
        MusicElement::Annotation(annotation) => {
            if annotation.placement == AnnotationPlacement::ChordSymbol {
                annotation.text = transpose_chord_annotation(
                    &annotation.text,
                    state.interval,
                    state.active_prefer_flats(),
                );
            }
            Ok(())
        }
        MusicElement::Bar(BarLine { .. }) => {
            state.clear_active_measure_accidentals();
            Ok(())
        }
        _ => Ok(()),
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

/// Transposes chord symbols inside an annotation while preserving non-chord text.
fn transpose_chord_annotation(text: &str, interval: i16, prefer_flats: bool) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remainder = text;
    while !remainder.is_empty() {
        if let Some((len, transposed)) = match_chord(remainder, interval, prefer_flats) {
            result.push_str(&transposed);
            remainder = &remainder[len..];
        } else {
            let next_char = remainder.chars().next().expect("remainder is not empty");
            result.push(next_char);
            remainder = &remainder[next_char.len_utf8()..];
        }
    }
    result
}

/// Attempts to parse a leading chord symbol or standalone bass note.
fn match_chord(s: &str, interval: i16, prefer_flats: bool) -> Option<(usize, String)> {
    if let Some((bass_len, (bass_char, bass_acc))) = s.strip_prefix('/').and_then(parse_bass_note) {
        let (new_bass, new_bass_acc) = transpose_note(bass_char, bass_acc, interval, prefer_flats);
        return Some((1 + bass_len, format!("/{new_bass}{new_bass_acc}")));
    }

    let root_char = s.chars().next()?;
    if !matches!(root_char, 'A'..='G') {
        return None;
    }
    let after_root = &s[root_char.len_utf8()..];
    let (root_acc_len, root_acc) = parse_accidental(after_root);
    let after_acc = &after_root[root_acc_len..];

    let mut type_end = 0;
    let mut bass_info = None;

    let mut char_indices = after_acc.char_indices().peekable();
    while let Some((offset, ch)) = char_indices.next() {
        let tail = &after_acc[offset..];
        if ch == '/' {
            if let Some((bass_len, (bass_char, bass_acc))) = parse_bass_note(&tail[1..]) {
                type_end = offset;
                bass_info = Some((1 + bass_len, bass_char, bass_acc));
                break;
            }
        } else if ch == '(' {
            let inside = &tail[1..];
            if let Some(close_idx) = inside.find(')') {
                let inner = &inside[..close_idx];
                if starts_with_chord_root(inner) {
                    type_end = offset;
                    break;
                }
                let end_paren_offset = offset + 1 + close_idx;
                while let Some(&(next_offset, _)) = char_indices.peek() {
                    if next_offset <= end_paren_offset {
                        char_indices.next();
                    } else {
                        break;
                    }
                }
                type_end = end_paren_offset + 1;
                continue;
            }
        } else if is_chord_boundary(tail) {
            type_end = offset;
            break;
        }
        type_end = offset + ch.len_utf8();
    }

    let chord_type = &after_acc[..type_end];
    if !is_valid_chord_type(chord_type) {
        return None;
    }

    let total_chord_len = root_char.len_utf8() + root_acc_len + type_end;
    let (slash_len, (new_bass, new_bass_acc)) = if let Some((len, bass_char, bass_acc)) = bass_info
    {
        (
            len,
            transpose_note(bass_char, bass_acc, interval, prefer_flats),
        )
    } else if is_chord_boundary(&after_acc[type_end..]) {
        (0, ('\0', ""))
    } else {
        return None;
    };

    let (new_root, new_root_acc) = transpose_note(root_char, root_acc, interval, prefer_flats);
    let mut out = format!("{new_root}{new_root_acc}{chord_type}");
    if slash_len > 0 {
        out.push('/');
        out.push(new_bass);
        out.push_str(new_bass_acc);
    }
    Some((total_chord_len + slash_len, out))
}

/// Transposes a single note letter and accidental by a signed chromatic interval.
fn transpose_note(
    letter: char,
    accidental: Option<&str>,
    interval: i16,
    prefer_flats: bool,
) -> (char, &'static str) {
    let is_lowercase = letter.is_ascii_lowercase();
    let class = match letter.to_ascii_uppercase() {
        'C' => PitchClass::C,
        'D' => PitchClass::D,
        'E' => PitchClass::E,
        'F' => PitchClass::F,
        'G' => PitchClass::G,
        'A' => PitchClass::A,
        'B' => PitchClass::B,
        _ => return (letter, ""),
    };
    let offset: i16 = match accidental {
        Some("#" | "♯") => 1,
        Some("##") => 2,
        Some("b" | "♭") => -1,
        Some("bb") => -2,
        _ => 0,
    };
    let semitone = natural_semitone(class) + offset;
    let transposed_semitone = (semitone + interval).rem_euclid(12);
    let transposed_semitone =
        u8::try_from(transposed_semitone).expect("a value modulo twelve fits in u8");
    let (new_class, new_acc) = chromatic_spelling(transposed_semitone, prefer_flats);
    let new_letter = match new_class {
        PitchClass::C => 'C',
        PitchClass::D => 'D',
        PitchClass::E => 'E',
        PitchClass::F => 'F',
        PitchClass::G => 'G',
        PitchClass::A => 'A',
        PitchClass::B => 'B',
    };
    let new_letter = if is_lowercase {
        new_letter.to_ascii_lowercase()
    } else {
        new_letter
    };
    let is_unicode = accidental.is_some_and(|a| a.contains(['♯', '♭', '♮']));
    let new_acc_str = match new_acc {
        1 => {
            if is_unicode {
                "♯"
            } else {
                "#"
            }
        }
        -1 => {
            if is_unicode {
                "♭"
            } else {
                "b"
            }
        }
        _ => "",
    };
    (new_letter, new_acc_str)
}

#[cfg(test)]
mod tests {
    use abc_parser::IntoOwnedAst;
    use abc_parser::MeasureAccidentals;
    use abc_parser::Offset;
    use abc_parser::ToAbc;
    use abc_parser::parse;
    use clap::Parser;

    use super::Arguments;
    use super::Request;
    use super::SpellingPreference;
    use super::parse_key;
    use super::parse_steps;
    use super::transpose_document;

    #[test]
    fn measure_accidental_slots_cover_every_pitch_class() {
        let classes = [
            abc_parser::PitchClass::C,
            abc_parser::PitchClass::D,
            abc_parser::PitchClass::E,
            abc_parser::PitchClass::F,
            abc_parser::PitchClass::G,
            abc_parser::PitchClass::A,
            abc_parser::PitchClass::B,
        ];
        let mut accidentals = MeasureAccidentals::default();
        for (index, class) in classes.into_iter().enumerate() {
            let offset = Offset::integer(i16::try_from(index).unwrap());
            accidentals.set(class, offset);
            assert_eq!(accidentals.get(class), Some(offset));
        }

        accidentals.clear();
        assert!(
            classes
                .into_iter()
                .all(|class| accidentals.get(class).is_none())
        );
    }

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

    /// Parses and transposes the document using all pitch options.
    fn transpose_with_options(
        source: &str,
        request: &Request,
        spelling: SpellingPreference,
        octave: i16,
    ) -> String {
        let mut document = parse(source).output.unwrap().into_owned(source).unwrap();
        transpose_document(&mut document, request, spelling, octave).unwrap();
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
    fn destination_and_interval_transposition_normalize_key_modes() {
        let source = "X:1\nK:Dmin\nD E F |\n";
        let destination = transpose(source, &Request::Key(parse_key("Em").unwrap()));
        let interval = transpose(source, &Request::Semitones(2));

        assert_eq!(destination, interval);
        assert!(destination.contains("K:Em"), "{destination}");
    }

    #[test]
    fn unconventional_destination_key_is_rejected() {
        assert!(parse_key("D#").is_err());
        assert!(parse_key("G#").is_err());
        assert!(parse_key("Fb").is_err());
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

    #[test]
    fn chord_symbols_transpose_with_semitone_interval() {
        let output = transpose(
            "X:1\nK:C\n\"C\"C \"Am\"A \"G7\"G | \"F#m\"F |\n",
            &Request::Semitones(2),
        );
        assert!(
            output.contains("\"D\"D \"Bm\"B \"A7\"A | \"G#m\"G |"),
            "{output}"
        );
    }

    #[test]
    fn slash_chords_transpose_root_and_bass() {
        let output = transpose(
            "X:1\nK:C\n\"C/E\"C \"Am/G\"A \"F/c\"F | \"/G\"G |\n",
            &Request::Semitones(2),
        );
        assert!(
            output.contains("\"D/F#\"D \"Bm/A\"B \"G/d\"G | \"/A\"A |"),
            "{output}"
        );
    }

    #[test]
    fn alternate_chords_transpose_both_chords() {
        let output = transpose(
            "X:1\nK:C\n\"G(Em)\"G \"C (Am)\"C | \"(Em)\"E \"C/E(G/B)\"C |\n",
            &Request::Semitones(2),
        );
        assert!(
            output.contains("\"A(F#m)\"A \"D (Bm)\"D | \"(F#m)\"F \"D/F#(A/C#)\"D |"),
            "{output}"
        );
    }

    #[test]
    fn complex_chords_preserve_tensions_and_alterations() {
        let output = transpose(
            "X:1\nK:C\n\"Cmaj7\"C \"Cm7b5\"C \"C7#9\"C \"C7(b9)\"C | \"C6/9\"C \"Caug\"C \"C+\"C \"C-7\"C \"C°7\"C \"Cø\"C \"CΔ7\"C \"Cadd9\"C |\n",
            &Request::Semitones(2),
        );
        assert!(
            output.contains("\"Dmaj7\"D \"Dm7b5\"D \"D7#9\"D \"D7(b9)\"D |"),
            "{output}"
        );
        assert!(
            output.contains(
                "\"D6/9\"D \"Daug\"D \"D+\"D \"D-7\"D \"D°7\"D \"Dø\"D \"DΔ7\"D \"Dadd9\"D |"
            ),
            "{output}"
        );
    }

    #[test]
    fn multiple_chords_and_non_chord_annotations() {
        let output = transpose(
            "X:1\nK:C\n\"C G\"C \"C - G\"C \"C | G\"C | \"Fine\"C \"^C\"C \"_G\"G \"<D\"D |\n",
            &Request::Semitones(2),
        );
        assert!(
            output.contains("\"D A\"D \"D - A\"D \"D | A\"D | \"Fine\"D \"^C\"D \"_G\"A \"<D\"E |"),
            "{output}"
        );
    }

    #[test]
    fn chord_symbols_follow_enharmonic_spelling_preferences() {
        let source = "X:1\nK:C\n\"C\"C |\n";
        let sharps =
            transpose_with_spelling(source, &Request::Semitones(1), SpellingPreference::Sharps);
        assert!(sharps.contains("\"C#\""), "{sharps}");

        let flats =
            transpose_with_spelling(source, &Request::Semitones(1), SpellingPreference::Flats);
        assert!(flats.contains("\"Db\""), "{flats}");
    }

    #[test]
    fn octave_transposition_does_not_affect_chord_symbols() {
        let output = transpose_with_options(
            "X:1\nK:C\n\"C\"C \"Am\"A |\n",
            &Request::Semitones(0),
            SpellingPreference::Auto,
            1,
        );
        assert!(output.contains("\"C\"c \"Am\"a |"), "{output}");
    }

    #[test]
    fn chord_symbols_transpose_with_destination_key() {
        let output = transpose(
            "X:1\nK:C\n\"C\"C \"F\"F \"G\"G |\n",
            &Request::Key(parse_key("D").unwrap()),
        );
        assert!(output.contains("K:D\n\"D\"D \"G\"G \"A\"A |"), "{output}");
    }

    #[test]
    fn unicode_accidentals_in_chord_symbols() {
        let output = transpose("X:1\nK:C\n\"C♯\"C \"B♭m\"A |\n", &Request::Semitones(2));
        assert!(output.contains("\"D♯\"D \"Cm\"B |"), "{output}");
    }

    #[test]
    fn multi_voice_accidentals_and_keys_are_scoped_per_voice() {
        let output = transpose(
            "X:1\nK:C\nV:1\n^C\nV:2\nC |\nV:1\nC |\n",
            &Request::Semitones(1),
        );
        assert!(output.contains("V:1\n=D\nV:2\nD |\nV:1\nD |"), "{output}");

        let key_output = transpose(
            "X:1\nK:G\nV:1\nK:F\nF |\nV:2\nF |\n",
            &Request::Semitones(1),
        );
        assert!(
            key_output.contains("V:1\nK:F#\nF |\nV:2\nG |"),
            "{key_output}"
        );
    }

    #[test]
    fn file_header_key_is_transposed_and_inherited_by_tunes() {
        let mut document = parse("M:4/4\n\nX:1\nF |\n")
            .output
            .unwrap()
            .into_owned("M:4/4\n\nX:1\nF |\n")
            .unwrap();
        document.header.push(abc_parser::Spanned {
            value: abc_parser::Line::Field(abc_parser::parse_field("K:G").unwrap()),
            span: (0..0).into(),
        });
        transpose_document(
            &mut document,
            &Request::Semitones(2),
            SpellingPreference::Auto,
            0,
        )
        .unwrap();
        let output = document.to_abc();
        assert!(output.contains("M:4/4\nK:A\n\nX:1\nG |"), "{output}");
    }
}
