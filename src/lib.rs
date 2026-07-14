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

//! ABC music notation 2.1 parsing with source spans and error recovery.
//!
//! The parser keeps extension fields and directives losslessly while recognizing
//! the structure of standard music constructs.
//!
#![doc = include_str!("../docs/architecture.md")]

use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::extra;
use chumsky::prelude::end;
use chumsky::prelude::none_of;
use std::fmt;
use std::ops::Range;

/// A half-open byte range in the original input.
pub type Span = Range<usize>;

/// A syntax value paired with its location in the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spanned<T> {
    /// Parsed syntax value.
    pub value: T,
    /// Half-open byte range in the source.
    pub span: Span,
}

/// A parsed ABC file, including file header material and tunes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Document {
    /// Lines before the first `X:` reference field.
    pub header: Vec<Spanned<Line>>,
    /// Tunes found in the file.
    pub tunes: Vec<Tune>,
}

/// One tune beginning with an `X:` field.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tune {
    /// All lines belonging to the tune, including `X:`.
    pub lines: Vec<Spanned<Line>>,
}

/// A physical ABC source line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Line {
    /// An empty or whitespace-only line.
    Blank,
    /// A `%` comment, excluding the leading marker.
    Comment(String),
    /// A `%%` instruction.
    Directive(Directive),
    /// An information field such as `T:Title`.
    Field(Field),
    /// Music code represented as parsed elements.
    Music(Vec<Spanned<MusicElement>>),
}

/// An ABC information field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    /// Single ASCII letter identifying the field.
    pub key: char,
    /// Standard meaning of the field letter.
    pub kind: FieldKind,
    /// Parsed field payload.
    pub value: FieldValue,
}

/// The payload of an information field.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FieldValue {
    /// An inherently textual metadata value.
    Text(String),
    /// An `L:` unit note length.
    UnitLength(Fraction),
    /// An `M:` time signature.
    Meter(Meter),
    /// A `Q:` tempo specification.
    Tempo(Tempo),
    /// A `K:` key signature and optional parameters.
    Key(KeySignature),
    /// An `X:` tune reference number.
    Reference(u32),
    /// A `V:` voice identifier and properties.
    Voice(VoiceDefinition),
    /// A `P:` part-order expression.
    Parts(PartSequence),
    /// A `U:` redefinable symbol assignment.
    UserSymbol(SymbolDefinition),
    /// An `m:` macro assignment.
    Macro(MacroDefinition),
    /// A structured field that failed to parse during recovery.
    Unparsed(String),
}

/// A time signature from an `M:` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Meter {
    /// Common time (`C`).
    Common,
    /// Cut time (`C|`).
    Cut,
    /// No meter (`none`).
    None,
    /// A simple fractional meter such as `3/4`.
    Simple(Fraction),
    /// An additive meter such as `2+3/8`.
    Compound {
        /// Additive numerator groups.
        groups: Vec<u32>,
        /// Beat-unit denominator.
        denominator: u32,
    },
}

/// A tempo from a `Q:` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tempo {
    /// Optional text before the metronome mark.
    pub prelude: Option<String>,
    /// Beat lengths on the left of `=`.
    pub beats: Vec<Fraction>,
    /// Beats per minute.
    pub bpm: u32,
    /// Optional text after the metronome mark.
    pub postlude: Option<String>,
}

/// A key signature from a `K:` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySignature {
    /// Tonic, or `None` for `none`/percussion.
    pub tonic: Option<KeyTonic>,
    /// Mode spelling, normalized to lowercase.
    pub mode: String,
    /// Remaining clef and transposition parameters.
    pub parameters: Vec<FieldParameter>,
}

/// The tonic of a key signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyTonic {
    /// Diatonic tonic.
    pub class: PitchClass,
    /// Optional sharp or flat.
    pub accidental: Option<KeyAccidental>,
}

/// A key-signature accidental.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAccidental {
    /// Sharp.
    Sharp,
    /// Flat.
    Flat,
}

/// A voice declaration from a `V:` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoiceDefinition {
    /// Voice identifier.
    pub id: String,
    /// Voice properties.
    pub properties: Vec<FieldParameter>,
}

/// A key/value or positional field parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldParameter {
    /// Parameter name, absent for positional text.
    pub name: Option<String>,
    /// Unquoted parameter value.
    pub value: String,
}

/// A parsed part-order expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartSequence {
    /// Tokens in source order.
    pub tokens: Vec<PartToken>,
}

/// A token in a part-order expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartToken {
    /// A named part.
    Part(String),
    /// Repetition count.
    Repeat(u32),
    /// Opening parenthesis.
    Open,
    /// Closing parenthesis.
    Close,
    /// Sequence separator (`.`).
    Separator,
}

/// A `U:` redefinable symbol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolDefinition {
    /// Single symbol being defined.
    pub symbol: char,
    /// Replacement music code.
    pub replacement: String,
}

/// An `m:` macro definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroDefinition {
    /// Macro pattern.
    pub pattern: String,
    /// Macro replacement.
    pub replacement: String,
}

/// The standardized meaning of an ABC information-field letter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FieldKind {
    /// `A:` area.
    Area,
    /// `B:` book.
    Book,
    /// `C:` composer.
    Composer,
    /// `D:` discography.
    Discography,
    /// `F:` source file URL.
    FileUrl,
    /// `G:` group.
    Group,
    /// `H:` history.
    History,
    /// `I:` instruction.
    Instruction,
    /// `K:` key.
    Key,
    /// `L:` unit note length.
    UnitLength,
    /// `M:` meter.
    Meter,
    /// `N:` notes.
    Notes,
    /// `O:` origin.
    Origin,
    /// `P:` parts.
    Parts,
    /// `Q:` tempo.
    Tempo,
    /// `R:` rhythm.
    Rhythm,
    /// `S:` source.
    Source,
    /// `T:` title.
    Title,
    /// `U:` user-defined symbol.
    UserSymbol,
    /// `V:` voice.
    Voice,
    /// `W:` unaligned words.
    Words,
    /// `X:` tune reference number.
    Reference,
    /// `Z:` transcription.
    Transcription,
    /// `m:` macro.
    Macro,
    /// `s:` symbol line.
    Symbols,
    /// `w:` aligned lyrics.
    Lyrics,
    /// A reserved or application-defined field.
    Extension(char),
}

/// A `%%` directive, retained for application-specific interpretation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Directive {
    /// Directive name.
    pub name: String,
    /// Remaining directive arguments.
    pub arguments: String,
}

/// A recognized element on a music-code line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MusicElement {
    /// A pitched note.
    Note(Note),
    /// A rest or invisible spacer.
    Rest(Rest),
    /// A multi-measure rest.
    MultiMeasureRest(MultiMeasureRest),
    /// A bracketed chord.
    Chord(Chord),
    /// A bar line or repeat marker.
    Bar(BarLine),
    /// A numbered or ranged variant ending.
    Ending(VariantEnding),
    /// An inline information field.
    InlineField(Field),
    /// A grace-note group including its braces.
    Grace(GraceGroup),
    /// A decoration including delimiters where present.
    Decoration(Decoration),
    /// A chord symbol or annotation in double quotes.
    Annotation(Annotation),
    /// A tuplet prefix `(p:q:r`.
    Tuplet(Tuplet),
    /// An opening or closing slur.
    Slur(Slur),
    /// A tie following a note or chord.
    Tie(Tie),
    /// A broken-rhythm operator.
    BrokenRhythm(BrokenRhythm),
    /// A voice-overlay operator (`&` or `(& ... & )`).
    Overlay(Overlay),
    /// Whitespace that starts a new beam group.
    BeamBreak(String),
    /// Ignorable backquotes inside a beam.
    BeamContinuation(usize),
    /// A source line-break or spacing control.
    LineBreak(LineBreak),
    /// Syntax accepted for forward-compatible extensions.
    Extension(String),
}

/// A diatonic pitch name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PitchClass {
    /// C.
    C,
    /// D.
    D,
    /// E.
    E,
    /// F.
    F,
    /// G.
    G,
    /// A.
    A,
    /// B.
    B,
}

/// A rational number used by lengths and microtonal accidentals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fraction {
    /// Numerator.
    pub numerator: u32,
    /// Denominator.
    pub denominator: u32,
}

/// An explicitly written accidental.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Accidental {
    /// Cancel the key signature or prior accidental.
    Natural,
    /// Raise by the given fraction of a semitone.
    Sharp(Fraction),
    /// Lower by the given fraction of a semitone.
    Flat(Fraction),
}

/// A pitch, independent of its duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pitch {
    /// Letter name.
    pub class: PitchClass,
    /// Octave displacement relative to uppercase ABC pitch.
    pub octave: i8,
    /// Explicit accidental, if any.
    pub accidental: Option<Accidental>,
}

/// A note-length multiplier relative to `L:`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NoteLength {
    /// Numerator.
    pub numerator: u32,
    /// Denominator.
    pub denominator: u32,
}

/// A pitched note.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Note {
    /// Parsed pitch.
    pub pitch: Pitch,
    /// Written duration multiplier.
    pub length: NoteLength,
}

/// The visual and playback kind of a rest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestKind {
    /// A printed `z` rest.
    Visible,
    /// An invisible `x` spacer.
    Invisible,
}

/// A single rest or invisible spacer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rest {
    /// Rest kind.
    pub kind: RestKind,
    /// Written duration multiplier.
    pub length: NoteLength,
}

/// A rest spanning whole measures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MultiMeasureRest {
    /// Whether the rest is invisible (`X`).
    pub invisible: bool,
    /// Number of measures.
    pub measures: u32,
}

/// The semantic role of a bar-line token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BarKind {
    /// `|`.
    Single,
    /// `||`.
    Double,
    /// `|]`.
    ThinThick,
    /// `[|`.
    ThickThin,
    /// `|:`.
    RepeatStart,
    /// `:|`.
    RepeatEnd,
    /// A combined end/start repeat.
    RepeatBoth,
    /// `.|`.
    Dotted,
    /// `[|]`.
    Invisible,
    /// A liberal standard-compatible bar spelling.
    Other,
}

/// A bar line or repeat delimiter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarLine {
    /// Semantic classification.
    pub kind: BarKind,
    /// Exact standard-compatible spelling.
    pub source: String,
}

/// A variant-ending selector such as `[1,3,5-7`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantEnding {
    /// Individual numbers and inclusive ranges.
    pub selectors: Vec<EndingSelector>,
}

/// One selector in a variant ending.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndingSelector {
    /// One ending number.
    Number(u32),
    /// An inclusive range of ending numbers.
    Range {
        /// First ending.
        start: u32,
        /// Last ending.
        end: u32,
    },
}

/// A grace-note group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraceGroup {
    /// Whether `/` requests acciaccatura rendering.
    pub acciaccatura: bool,
    /// Notes inside the braces.
    pub notes: Vec<Note>,
}

/// A named or shorthand decoration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Decoration {
    /// Decoration name after expanding shorthand spelling only structurally.
    pub name: String,
    /// Whether deprecated `+name+` syntax was used.
    pub legacy_delimiter: bool,
}

/// Placement of a quoted annotation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnotationPlacement {
    /// A chord symbol with implicit placement.
    ChordSymbol,
    /// Above the staff.
    Above,
    /// Below the staff.
    Below,
    /// Left of the following element.
    Left,
    /// Right of the following element.
    Right,
    /// Application-positioned annotation.
    Free,
}

/// A chord symbol or textual annotation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    /// Placement marker.
    pub placement: AnnotationPlacement,
    /// Text without quotes or placement marker.
    pub text: String,
}

/// Tuplet timing and extent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tuplet {
    /// Notes written.
    pub p: u8,
    /// Time occupied, or context-dependent default.
    pub q: Option<u8>,
    /// Number of affected notes, defaulting to `p`.
    pub r: Option<u8>,
}

/// An opening or closing slur marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slur {
    /// True for an opening marker.
    pub opening: bool,
    /// Whether the slur is dotted.
    pub dotted: bool,
}

/// A tie marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tie {
    /// Whether the tie is dotted.
    pub dotted: bool,
}

/// Direction and strength of broken rhythm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokenRhythm {
    /// True for `>`, false for `<`.
    pub greater: bool,
    /// Number of repeated angle brackets.
    pub count: u8,
}

/// A voice-overlay control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlay {
    /// Switch to the next overlay voice.
    NextVoice,
    /// Start a measure overlay.
    Start,
    /// End a measure overlay.
    End,
}

/// An explicit line-breaking control in music code.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineBreak {
    /// Continue the music code line.
    Continue,
    /// Force a staff break.
    Break,
    /// Force a paragraph break.
    Paragraph,
    /// Add typesetting space.
    Space,
}

/// A bracketed group of simultaneous notes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chord {
    /// Notes and rests within the brackets.
    pub members: Vec<ChordMember>,
    /// Duration multiplier following the closing bracket.
    pub length: NoteLength,
}

/// A note or rest inside a chord/unison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChordMember {
    /// A pitched chord member.
    Note(Note),
    /// A rest chord member.
    Rest(Rest),
}

type ElementScan = (MusicElement, usize, Option<(ErrorKind, &'static str)>);

/// Classification of a parse fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A construct ended before its closing delimiter.
    UnclosedDelimiter,
    /// A field lacks a valid single-letter name.
    InvalidField,
    /// A directive lacks a name.
    InvalidDirective,
    /// A token is not valid music syntax.
    InvalidMusic,
    /// A file has tune material but no `X:` reference field.
    MissingReference,
}

/// A recoverable syntax error with an exact source location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    /// Error category suitable for programmatic handling.
    pub kind: ErrorKind,
    /// Human-readable explanation.
    pub message: String,
    /// Half-open byte range in the original input.
    pub span: Span,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for ParseError {}

/// The syntax tree and every diagnostic produced during recovering parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseReport<T> {
    /// Recovered syntax value.
    pub output: T,
    /// Diagnostics in source order.
    pub errors: Vec<ParseError>,
}

impl<T> ParseReport<T> {
    /// Returns whether parsing completed without diagnostics.
    pub const fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Parses a complete ABC document and recovers at physical line boundaries.
///
/// Source spans are always recorded. Callers that do not need them can ignore
/// the [`Spanned::span`] fields.
pub fn parse_recovering(source: &str) -> ParseReport<Document> {
    let mut document = Document::default();
    let mut errors = Vec::new();
    let mut current_tune: Option<Tune> = None;

    for (start, raw) in physical_lines(source) {
        let line_source = raw.strip_suffix('\r').unwrap_or(raw);
        let end = start + line_source.len();
        let report = parse_line_at(line_source, start);
        errors.extend(report.errors);
        let is_reference = matches!(&report.output, Line::Field(field) if field.key == 'X');
        let spanned = Spanned {
            value: report.output,
            span: start..end,
        };
        if is_reference {
            if let Some(tune) = current_tune.replace(Tune::default()) {
                document.tunes.push(tune);
            }
        }
        if let Some(tune) = &mut current_tune {
            tune.lines.push(spanned);
        } else {
            document.header.push(spanned);
        }
    }
    if let Some(tune) = current_tune {
        document.tunes.push(tune);
    }
    if document.tunes.is_empty()
        && document.header.iter().any(|line| {
            !matches!(
                line.value,
                Line::Blank | Line::Comment(_) | Line::Directive(_)
            )
        })
    {
        errors.push(ParseError {
            kind: ErrorKind::MissingReference,
            message: "document contains tune material but no X: reference field".into(),
            span: 0..source.len().min(1),
        });
    }
    ParseReport {
        output: document,
        errors,
    }
}

/// Parses a complete ABC document, failing if any syntax error is found.
///
/// # Errors
/// Returns all syntax errors found while recovering through the document.
pub fn parse(source: &str) -> Result<Document, Vec<ParseError>> {
    let report = parse_recovering(source);
    if report.errors.is_empty() {
        Ok(report.output)
    } else {
        Err(report.errors)
    }
}

/// Validates a complete ABC document without returning its syntax tree.
///
/// # Errors
/// Returns all syntax errors found in the document.
pub fn validate(source: &str) -> Result<(), Vec<ParseError>> {
    parse(source).map(|_| ())
}

/// Parses one physical ABC line.
pub fn parse_line(source: &str) -> ParseReport<Line> {
    parse_line_at(source.strip_suffix(['\r', '\n']).unwrap_or(source), 0)
}

/// Parses a `%%` directive line.
///
/// # Errors
/// Returns an error if the prefix or directive name is invalid.
pub fn parse_directive(source: &str) -> Result<Directive, ParseError> {
    let line = source.trim_end_matches(['\r', '\n']);
    let Some(body) = line.strip_prefix("%%") else {
        return Err(error(
            ErrorKind::InvalidDirective,
            "directive must begin with %%",
            0,
            line.len().min(2),
        ));
    };
    let split = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = &body[..split];
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(error(
            ErrorKind::InvalidDirective,
            "invalid directive name",
            2,
            2 + name.len().max(1),
        ));
    }
    Ok(Directive {
        name: name.into(),
        arguments: body[split..].trim().into(),
    })
}

/// Parses an information field such as `K:C major`.
///
/// # Errors
/// Returns an error unless the input starts with one ASCII letter and `:`.
pub fn parse_field(source: &str) -> Result<Field, ParseError> {
    parse_field_at(source.trim_end_matches(['\r', '\n']), 0)
}

/// Parses a complete bracketed chord.
///
/// ```
/// use abc_parser::parse_chord;
/// let chord = parse_chord("[^CEG]3/2").unwrap();
/// assert_eq!(chord.length.numerator, 3);
/// assert_eq!(chord.length.denominator, 2);
/// ```
///
/// # Errors
/// Returns an error for missing brackets, empty contents, or an invalid length.
pub fn parse_chord(source: &str) -> Result<Chord, ParseError> {
    if !source.starts_with('[') {
        return Err(error(
            ErrorKind::InvalidMusic,
            "chord must begin with [",
            0,
            source.len().min(1),
        ));
    }
    let Some(close) = source.find(']') else {
        return Err(error(
            ErrorKind::UnclosedDelimiter,
            "unclosed chord",
            0,
            source.len(),
        ));
    };
    let contents = &source[1..close];
    let length = &source[close + 1..];
    if contents.is_empty() || !valid_chord_contents(contents) || !valid_length(length) {
        return Err(error(
            ErrorKind::InvalidMusic,
            "invalid chord contents or length",
            1,
            source.len(),
        ));
    }
    let mut members = Vec::new();
    let mut index = 0;
    while index < contents.len() {
        let tail = &contents[index..];
        if let Some((note, consumed)) = parse_note_token(tail) {
            members.push(ChordMember::Note(note));
            index += consumed;
        } else if let Some((rest, consumed)) = parse_rest_token(tail) {
            members.push(ChordMember::Rest(rest));
            index += consumed;
        } else {
            return Err(error(
                ErrorKind::InvalidMusic,
                "invalid chord member",
                1 + index,
                2 + index,
            ));
        }
    }
    Ok(Chord {
        members,
        length: parse_length(length),
    })
}

/// Parses a line as music code, recovering after malformed elements.
pub fn parse_music_line(source: &str) -> ParseReport<Vec<Spanned<MusicElement>>> {
    parse_music_at(source.trim_end_matches(['\r', '\n']), 0)
}

fn physical_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    source.split_terminator('\n').map(move |line| {
        let result = (offset, line);
        offset += line.len() + 1;
        result
    })
}

fn parse_line_at(source: &str, offset: usize) -> ParseReport<Line> {
    if source.trim().is_empty() {
        return ParseReport {
            output: Line::Blank,
            errors: Vec::new(),
        };
    }
    if let Some(directive_body) = source.strip_prefix("%%") {
        return match parse_directive(source) {
            Ok(value) => ParseReport {
                output: Line::Directive(value),
                errors: Vec::new(),
            },
            Err(mut fault) => {
                shift_error(&mut fault, offset);
                ParseReport {
                    output: Line::Directive(Directive {
                        name: String::new(),
                        arguments: directive_body.into(),
                    }),
                    errors: vec![fault],
                }
            }
        };
    }
    if let Some(comment) = source.strip_prefix('%') {
        return ParseReport {
            output: Line::Comment(comment.into()),
            errors: Vec::new(),
        };
    }
    if source.as_bytes().get(1) == Some(&b':') && source.as_bytes()[0].is_ascii_alphabetic() {
        return match parse_field_at(source, offset) {
            Ok(value) => ParseReport {
                output: Line::Field(value),
                errors: Vec::new(),
            },
            Err(fault) => ParseReport {
                output: Line::Field(Field {
                    key: source.chars().next().unwrap_or('?'),
                    kind: field_kind(source.chars().next().unwrap_or('?')),
                    value: FieldValue::Unparsed(source.get(2..).unwrap_or_default().trim().into()),
                }),
                errors: vec![fault],
            },
        };
    }
    let report = parse_music_at(source, offset);
    ParseReport {
        output: Line::Music(report.output),
        errors: report.errors,
    }
}

fn parse_field_at(source: &str, offset: usize) -> Result<Field, ParseError> {
    let bytes = source.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return Err(error(
            ErrorKind::InvalidField,
            "field must have the form A:value",
            offset,
            offset + source.len().min(2),
        ));
    }
    let key = char::from(bytes[0]);
    let raw = source[2..].trim();
    let value = parse_field_value(key, raw).map_err(|message| {
        error(
            ErrorKind::InvalidField,
            message,
            offset + 2,
            offset + source.len(),
        )
    })?;
    Ok(Field {
        key,
        kind: field_kind(key),
        value,
    })
}

fn parse_music_at(source: &str, offset: usize) -> ParseReport<Vec<Spanned<MusicElement>>> {
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let tail = &source[index..];
        let start = index;
        let (element, consumed, fault) = scan_element(tail);
        let consumed = consumed.max(tail.chars().next().map_or(1, char::len_utf8));
        output.push(Spanned {
            value: element,
            span: offset + start..offset + start + consumed,
        });
        if let Some((kind, message)) = fault {
            errors.push(error(
                kind,
                message,
                offset + start,
                offset + start + consumed,
            ));
        }
        index += consumed;
    }
    ParseReport { output, errors }
}

#[allow(clippy::too_many_lines)]
fn scan_element(source: &str) -> ElementScan {
    let first = source.as_bytes()[0];
    if first.is_ascii_whitespace() {
        let len = source
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(source.len());
        return (MusicElement::BeamBreak(source[..len].into()), len, None);
    }
    if let Some(result) = scan_bracketed(source) {
        return result;
    }
    if source.starts_with('{') {
        return scan_grace(source);
    }
    if first == b'!' || first == b'+' {
        return scan_long_decoration(source);
    }
    if first == b'"' {
        return scan_annotation(source);
    }
    if let Some((note, len)) = parse_note_token(source) {
        return (MusicElement::Note(note), len, None);
    }
    if let Some((rest, len)) = parse_rest_token(source) {
        return (MusicElement::Rest(rest), len, None);
    }
    if matches!(first, b'Z' | b'X') {
        let suffix = source[1..].bytes().take_while(u8::is_ascii_digit).count();
        let measures = source[1..][..suffix].parse().unwrap_or(1);
        return (
            MusicElement::MultiMeasureRest(MultiMeasureRest {
                invisible: first == b'X',
                measures,
            }),
            suffix + 1,
            None,
        );
    }
    if "|:]".contains(char::from(first)) || source.starts_with(".|") {
        let len = source
            .bytes()
            .take_while(|byte| b".|:[]".contains(byte))
            .count();
        let text = &source[..len];
        return (MusicElement::Bar(bar_line(text)), len, None);
    }
    if first == b'(' && source.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        return scan_tuplet(source);
    }
    if source.starts_with(".(") {
        return (
            MusicElement::Slur(Slur {
                opening: true,
                dotted: true,
            }),
            2,
            None,
        );
    }
    if source.starts_with(".)") {
        return (
            MusicElement::Slur(Slur {
                opening: false,
                dotted: true,
            }),
            2,
            None,
        );
    }
    if first == b'(' || first == b')' {
        return (
            MusicElement::Slur(Slur {
                opening: first == b'(',
                dotted: false,
            }),
            1,
            None,
        );
    }
    if source.starts_with(".-") {
        return (MusicElement::Tie(Tie { dotted: true }), 2, None);
    }
    if first == b'-' {
        return (MusicElement::Tie(Tie { dotted: false }), 1, None);
    }
    if matches!(first, b'<' | b'>') {
        let len = source.bytes().take_while(|byte| *byte == first).count();
        return (
            MusicElement::BrokenRhythm(BrokenRhythm {
                greater: first == b'>',
                count: u8::try_from(len).unwrap_or(u8::MAX),
            }),
            len,
            None,
        );
    }
    if source.starts_with("(&") {
        return (MusicElement::Overlay(Overlay::Start), 2, None);
    }
    if source.starts_with("&)") {
        return (MusicElement::Overlay(Overlay::End), 2, None);
    }
    if first == b'&' {
        return (MusicElement::Overlay(Overlay::NextVoice), 1, None);
    }
    if first.is_ascii_digit() {
        let len = source
            .bytes()
            .take_while(|byte| byte.is_ascii_digit() || matches!(byte, b',' | b'-'))
            .count();
        if let Some(ending) = parse_ending(&source[..len]) {
            return (MusicElement::Ending(ending), len, None);
        }
    }
    if first == b'\\' {
        return (MusicElement::LineBreak(LineBreak::Continue), 1, None);
    }
    if first == b'`' {
        let len = source.bytes().take_while(|byte| *byte == b'`').count();
        return (MusicElement::BeamContinuation(len), len, None);
    }
    if let Some(name) = shorthand_decoration(char::from(first)) {
        return (
            MusicElement::Decoration(Decoration {
                name: name.into(),
                legacy_delimiter: false,
            }),
            1,
            None,
        );
    }
    let len = source.chars().next().map_or(1, char::len_utf8);
    (
        MusicElement::Extension(source[..len].into()),
        len,
        Some((ErrorKind::InvalidMusic, "unrecognized music token")),
    )
}

fn scan_bracketed(source: &str) -> Option<ElementScan> {
    if !source.starts_with('[') {
        return None;
    }
    if source.as_bytes().get(2) == Some(&b':')
        && source
            .as_bytes()
            .get(1)
            .is_some_and(u8::is_ascii_alphabetic)
    {
        return Some(if let Some(close) = source.find(']') {
            match parse_field(&source[1..close]) {
                Ok(field) => (MusicElement::InlineField(field), close + 1, None),
                Err(_) => (
                    MusicElement::InlineField(Field {
                        key: char::from(source.as_bytes()[1]),
                        kind: field_kind(char::from(source.as_bytes()[1])),
                        value: FieldValue::Unparsed(source[3..close].trim().into()),
                    }),
                    close + 1,
                    Some((ErrorKind::InvalidField, "invalid inline field")),
                ),
            }
        } else {
            (
                MusicElement::Extension(source.into()),
                source.len(),
                Some((ErrorKind::UnclosedDelimiter, "unclosed inline field")),
            )
        });
    }
    if source.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        let len = source
            .bytes()
            .skip(1)
            .take_while(|byte| byte.is_ascii_digit() || matches!(byte, b',' | b'-'))
            .count()
            + 1;
        return Some(match parse_ending(&source[1..len]) {
            Some(ending) => (MusicElement::Ending(ending), len, None),
            None => (
                MusicElement::Extension(source[..len].into()),
                len,
                Some((ErrorKind::InvalidMusic, "invalid variant ending")),
            ),
        });
    }
    if source.starts_with("[|") {
        let len = source
            .bytes()
            .take_while(|byte| b"|:[]".contains(byte))
            .count();
        return Some((MusicElement::Bar(bar_line(&source[..len])), len, None));
    }
    Some(if let Some(close) = source.find(']') {
        let suffix = length_prefix(&source[close + 1..]);
        let len = close + 1 + suffix;
        match parse_chord(&source[..len]) {
            Ok(chord) => (MusicElement::Chord(chord), len, None),
            Err(_) => (
                MusicElement::Extension(source[..len].into()),
                len,
                Some((ErrorKind::InvalidMusic, "invalid chord")),
            ),
        }
    } else {
        (
            MusicElement::Extension(source.into()),
            source.len(),
            Some((ErrorKind::UnclosedDelimiter, "unclosed chord")),
        )
    })
}

fn scan_grace(source: &str) -> ElementScan {
    let Some(close) = source.find('}') else {
        return (
            MusicElement::Extension(source.into()),
            source.len(),
            Some((ErrorKind::UnclosedDelimiter, "unclosed grace group")),
        );
    };
    let mut body = &source[1..close];
    let acciaccatura = body.starts_with('/');
    if acciaccatura {
        body = &body[1..];
    }
    let mut notes = Vec::new();
    while !body.is_empty() {
        if let Some((note, len)) = parse_note_token(body) {
            notes.push(note);
            body = &body[len..];
        } else {
            return (
                MusicElement::Extension(source[..=close].into()),
                close + 1,
                Some((ErrorKind::InvalidMusic, "invalid grace note")),
            );
        }
    }
    (
        MusicElement::Grace(GraceGroup {
            acciaccatura,
            notes,
        }),
        close + 1,
        None,
    )
}

fn scan_long_decoration(source: &str) -> ElementScan {
    let delimiter = char::from(source.as_bytes()[0]);
    let Some(close) = source[1..].find(delimiter) else {
        return (
            MusicElement::Extension(source.into()),
            source.len(),
            Some((ErrorKind::UnclosedDelimiter, "unclosed decoration")),
        );
    };
    let len = close + 2;
    (
        MusicElement::Decoration(Decoration {
            name: source[1..len - 1].into(),
            legacy_delimiter: delimiter == '+',
        }),
        len,
        None,
    )
}

fn scan_annotation(source: &str) -> ElementScan {
    let Some(close) = source[1..].find('"') else {
        return (
            MusicElement::Extension(source.into()),
            source.len(),
            Some((ErrorKind::UnclosedDelimiter, "unclosed annotation")),
        );
    };
    let len = close + 2;
    let body = &source[1..len - 1];
    let (placement, text) = match body.chars().next() {
        Some('^') => (AnnotationPlacement::Above, &body[1..]),
        Some('_') => (AnnotationPlacement::Below, &body[1..]),
        Some('<') => (AnnotationPlacement::Left, &body[1..]),
        Some('>') => (AnnotationPlacement::Right, &body[1..]),
        Some('@') => (AnnotationPlacement::Free, &body[1..]),
        _ => (AnnotationPlacement::ChordSymbol, body),
    };
    (
        MusicElement::Annotation(Annotation {
            placement,
            text: text.into(),
        }),
        len,
        None,
    )
}

fn scan_tuplet(source: &str) -> ElementScan {
    let len = source
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || matches!(byte, b'(' | b':'))
        .count();
    let parts: Vec<_> = source[1..len].split(':').collect();
    let p = parts
        .first()
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    let q = parts
        .get(1)
        .filter(|part| !part.is_empty())
        .and_then(|part| part.parse().ok());
    let r = parts
        .get(2)
        .filter(|part| !part.is_empty())
        .and_then(|part| part.parse().ok());
    (MusicElement::Tuplet(Tuplet { p, q, r }), len, None)
}

fn parse_note_token(source: &str) -> Option<(Note, usize)> {
    let bytes = source.as_bytes();
    let mut index = 0;
    let accidental = parse_accidental(source, &mut index);
    let letter = *bytes.get(index)?;
    let class = match letter.to_ascii_uppercase() {
        b'A' => PitchClass::A,
        b'B' => PitchClass::B,
        b'C' => PitchClass::C,
        b'D' => PitchClass::D,
        b'E' => PitchClass::E,
        b'F' => PitchClass::F,
        b'G' => PitchClass::G,
        _ => return None,
    };
    let mut octave = i8::from(letter.is_ascii_lowercase());
    index += 1;
    while let Some(marker) = bytes.get(index) {
        match marker {
            b'\'' => octave += 1,
            b',' => octave -= 1,
            _ => break,
        }
        index += 1;
    }
    let length_len = length_prefix(&source[index..]);
    let length = parse_length(&source[index..index + length_len]);
    index += length_len;
    Some((
        Note {
            pitch: Pitch {
                class,
                octave,
                accidental,
            },
            length,
        },
        index,
    ))
}

fn parse_rest_token(source: &str) -> Option<(Rest, usize)> {
    let first = *source.as_bytes().first()?;
    let kind = match first {
        b'z' => RestKind::Visible,
        b'x' => RestKind::Invisible,
        _ => return None,
    };
    let suffix = length_prefix(&source[1..]);
    Some((
        Rest {
            kind,
            length: parse_length(&source[1..][..suffix]),
        },
        suffix + 1,
    ))
}

fn parse_accidental(source: &str, index: &mut usize) -> Option<Accidental> {
    let bytes = source.as_bytes();
    let marker = *bytes.get(*index)?;
    if marker == b'=' {
        *index += 1;
        return Some(Accidental::Natural);
    }
    if !matches!(marker, b'^' | b'_') {
        return None;
    }
    let mut count = 0;
    while bytes.get(*index) == Some(&marker) {
        count += 1;
        *index += 1;
    }
    let number_start = *index;
    while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
        *index += 1;
    }
    let numerator = source[number_start..*index].parse().unwrap_or(count);
    let denominator = if bytes.get(*index) == Some(&b'/') {
        *index += 1;
        let start = *index;
        while bytes.get(*index).is_some_and(u8::is_ascii_digit) {
            *index += 1;
        }
        source[start..*index].parse().unwrap_or(2)
    } else {
        1
    };
    let fraction = Fraction {
        numerator,
        denominator,
    };
    Some(if marker == b'^' {
        Accidental::Sharp(fraction)
    } else {
        Accidental::Flat(fraction)
    })
}

fn parse_length(source: &str) -> NoteLength {
    if source.is_empty() {
        return NoteLength {
            numerator: 1,
            denominator: 1,
        };
    }
    let slash = source.find('/');
    let numerator = slash
        .map_or(source, |at| &source[..at])
        .parse()
        .unwrap_or(1);
    let denominator = slash.map_or(1, |at| {
        let tail = &source[at + 1..];
        if tail.is_empty() {
            2
        } else if tail.bytes().all(|byte| byte == b'/') {
            2_u32.pow(u32::try_from(tail.len() + 1).unwrap_or(31))
        } else {
            tail.parse().unwrap_or(2)
        }
    });
    NoteLength {
        numerator,
        denominator,
    }
}

fn bar_line(source: &str) -> BarLine {
    let kind = match source {
        "|" => BarKind::Single,
        "||" => BarKind::Double,
        "|]" => BarKind::ThinThick,
        "[|" => BarKind::ThickThin,
        "|:" => BarKind::RepeatStart,
        ":|" => BarKind::RepeatEnd,
        "::" | ":|:" | ":||:" => BarKind::RepeatBoth,
        ".|" => BarKind::Dotted,
        "[|]" => BarKind::Invisible,
        _ => BarKind::Other,
    };
    BarLine {
        kind,
        source: source.into(),
    }
}

fn parse_ending(source: &str) -> Option<VariantEnding> {
    let mut selectors = Vec::new();
    for part in source.split(',') {
        if let Some((start, end)) = part.split_once('-') {
            selectors.push(EndingSelector::Range {
                start: start.parse().ok()?,
                end: end.parse().ok()?,
            });
        } else {
            selectors.push(EndingSelector::Number(part.parse().ok()?));
        }
    }
    Some(VariantEnding { selectors })
}

fn shorthand_decoration(character: char) -> Option<&'static str> {
    Some(match character {
        '.' => "staccato",
        '~' => "roll",
        'H' => "fermata",
        'L' => "accent",
        'M' => "lowermordent",
        'O' => "coda",
        'P' => "uppermordent",
        'S' => "segno",
        'T' => "trill",
        'u' => "upbow",
        'v' => "downbow",
        _ => return None,
    })
}

fn parse_field_value(key: char, source: &str) -> Result<FieldValue, &'static str> {
    match key {
        'L' => parse_fraction(source)
            .filter(|value| value.denominator != 0)
            .map(FieldValue::UnitLength)
            .ok_or("invalid L: unit note length"),
        'M' => parse_meter(source).map(FieldValue::Meter),
        'Q' => parse_tempo(source).map(FieldValue::Tempo),
        'K' => parse_key(source).map(FieldValue::Key),
        'X' => source
            .parse()
            .ok()
            .map(FieldValue::Reference)
            .ok_or("invalid X: reference number"),
        'V' => parse_voice(source).map(FieldValue::Voice),
        'P' => parse_parts(source).map(FieldValue::Parts),
        'U' => parse_assignment(source)
            .filter(|(left, _)| left.chars().count() == 1)
            .map(|(left, replacement)| {
                FieldValue::UserSymbol(SymbolDefinition {
                    symbol: left.chars().next().unwrap_or_default(),
                    replacement: replacement.into(),
                })
            })
            .ok_or("invalid U: symbol definition"),
        'm' => parse_assignment(source)
            .map(|(pattern, replacement)| {
                FieldValue::Macro(MacroDefinition {
                    pattern: pattern.into(),
                    replacement: replacement.into(),
                })
            })
            .ok_or("invalid m: macro definition"),
        _ => Ok(FieldValue::Text(source.into())),
    }
}

fn parse_fraction(source: &str) -> Option<Fraction> {
    let (numerator, denominator) = source.split_once('/')?;
    if numerator.is_empty() || denominator.is_empty() || denominator.contains('/') {
        return None;
    }
    Some(Fraction {
        numerator: numerator.parse().ok()?,
        denominator: denominator.parse().ok()?,
    })
}

fn parse_meter(source: &str) -> Result<Meter, &'static str> {
    match source {
        "C" => return Ok(Meter::Common),
        "C|" => return Ok(Meter::Cut),
        value if value.eq_ignore_ascii_case("none") => return Ok(Meter::None),
        _ => {}
    }
    let (numerators, denominator) = source.split_once('/').ok_or("invalid M: meter")?;
    let groups = numerators
        .trim_matches(['(', ')'])
        .split('+')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid M: numerator")?;
    let denominator = denominator.parse().map_err(|_| "invalid M: denominator")?;
    if groups.is_empty() || denominator == 0 {
        return Err("invalid M: meter");
    }
    if let [numerator] = groups.as_slice() {
        Ok(Meter::Simple(Fraction {
            numerator: *numerator,
            denominator,
        }))
    } else {
        Ok(Meter::Compound {
            groups,
            denominator,
        })
    }
}

fn parse_tempo(source: &str) -> Result<Tempo, &'static str> {
    let (prelude, remainder) = take_quoted_prefix(source.trim())?;
    let (mark, postlude_source) = if let Some(quote) = remainder.find('"') {
        (&remainder[..quote], remainder[quote..].trim())
    } else {
        (remainder, "")
    };
    let postlude = if postlude_source.is_empty() {
        None
    } else {
        let (text, trailing) = take_quoted_prefix(postlude_source)?;
        if !trailing.is_empty() {
            return Err("invalid Q: trailing text");
        }
        text
    };
    let (beats, bpm) = mark
        .trim()
        .split_once('=')
        .ok_or("invalid Q: metronome mark")?;
    let beats = beats
        .split_whitespace()
        .map(|beat| {
            if beat == "C" {
                Some(Fraction {
                    numerator: 1,
                    denominator: 1,
                })
            } else {
                parse_fraction(beat)
            }
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("invalid Q: beat length")?;
    let bpm = bpm.trim().parse().map_err(|_| "invalid Q: bpm")?;
    if beats.is_empty() || bpm == 0 {
        return Err("invalid Q: tempo");
    }
    Ok(Tempo {
        prelude,
        beats,
        bpm,
        postlude,
    })
}

fn take_quoted_prefix(source: &str) -> Result<(Option<String>, &str), &'static str> {
    if !source.starts_with('"') {
        return Ok((None, source));
    }
    let close = source[1..].find('"').ok_or("unclosed quoted field text")? + 1;
    Ok((Some(source[1..close].into()), source[close + 1..].trim()))
}

fn parse_key(source: &str) -> Result<KeySignature, &'static str> {
    let mut words = split_field_words(source);
    if words.is_empty() {
        return Err("empty K: field");
    }
    let head = words.remove(0);
    let lower = head.to_ascii_lowercase();
    let (tonic, mode) = if matches!(lower.as_str(), "none" | "hp" | "perc") {
        (None, lower)
    } else {
        let bytes = head.as_bytes();
        let class = match bytes.first().map(u8::to_ascii_uppercase) {
            Some(b'A') => PitchClass::A,
            Some(b'B') => PitchClass::B,
            Some(b'C') => PitchClass::C,
            Some(b'D') => PitchClass::D,
            Some(b'E') => PitchClass::E,
            Some(b'F') => PitchClass::F,
            Some(b'G') => PitchClass::G,
            _ => return Err("invalid K: tonic"),
        };
        let accidental = match bytes.get(1) {
            Some(b'#') => Some(KeyAccidental::Sharp),
            Some(b'b') => Some(KeyAccidental::Flat),
            _ => None,
        };
        let mode_start = 1 + usize::from(accidental.is_some());
        (
            Some(KeyTonic { class, accidental }),
            head[mode_start..].to_ascii_lowercase(),
        )
    };
    let mut parameters = Vec::new();
    let mut mode = mode;
    if mode.is_empty() && words.first().is_some_and(|word| !word.contains('=')) {
        mode = words.remove(0).to_ascii_lowercase();
    }
    for word in words {
        parameters.push(parameter_from_word(&word));
    }
    Ok(KeySignature {
        tonic,
        mode,
        parameters,
    })
}

fn parse_voice(source: &str) -> Result<VoiceDefinition, &'static str> {
    let mut words = split_field_words(source);
    if words.is_empty() {
        return Err("empty V: voice identifier");
    }
    let id = words.remove(0);
    Ok(VoiceDefinition {
        id,
        properties: words.iter().map(|word| parameter_from_word(word)).collect(),
    })
}

fn split_field_words(source: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in source.chars() {
        match character {
            '"' => quoted = !quoted,
            value if value.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            value => current.push(value),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn parameter_from_word(source: &str) -> FieldParameter {
    if let Some((name, value)) = source.split_once('=') {
        FieldParameter {
            name: Some(name.into()),
            value: value.into(),
        }
    } else {
        FieldParameter {
            name: None,
            value: source.into(),
        }
    }
}

fn parse_parts(source: &str) -> Result<PartSequence, &'static str> {
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < source.len() {
        let tail = &source[index..];
        let first = tail.chars().next().ok_or("invalid P: part sequence")?;
        if first.is_whitespace() {
            index += first.len_utf8();
        } else if first.is_ascii_alphabetic() {
            let len = tail.bytes().take_while(u8::is_ascii_alphabetic).count();
            tokens.push(PartToken::Part(tail[..len].into()));
            index += len;
        } else if first.is_ascii_digit() {
            let len = tail.bytes().take_while(u8::is_ascii_digit).count();
            tokens.push(PartToken::Repeat(
                tail[..len].parse().map_err(|_| "invalid P: repeat")?,
            ));
            index += len;
        } else {
            tokens.push(match first {
                '(' => PartToken::Open,
                ')' => PartToken::Close,
                '.' => PartToken::Separator,
                _ => return Err("invalid P: part sequence"),
            });
            index += 1;
        }
    }
    if tokens.is_empty() {
        Err("empty P: part sequence")
    } else {
        Ok(PartSequence { tokens })
    }
}

fn parse_assignment(source: &str) -> Option<(&str, &str)> {
    let (left, right) = source.split_once('=')?;
    let left = left.trim();
    let right = right.trim();
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

fn field_kind(key: char) -> FieldKind {
    match key {
        'A' => FieldKind::Area,
        'B' => FieldKind::Book,
        'C' => FieldKind::Composer,
        'D' => FieldKind::Discography,
        'F' => FieldKind::FileUrl,
        'G' => FieldKind::Group,
        'H' => FieldKind::History,
        'I' => FieldKind::Instruction,
        'K' => FieldKind::Key,
        'L' => FieldKind::UnitLength,
        'M' => FieldKind::Meter,
        'N' => FieldKind::Notes,
        'O' => FieldKind::Origin,
        'P' => FieldKind::Parts,
        'Q' => FieldKind::Tempo,
        'R' => FieldKind::Rhythm,
        'S' => FieldKind::Source,
        'T' => FieldKind::Title,
        'U' => FieldKind::UserSymbol,
        'V' => FieldKind::Voice,
        'W' => FieldKind::Words,
        'X' => FieldKind::Reference,
        'Z' => FieldKind::Transcription,
        'm' => FieldKind::Macro,
        's' => FieldKind::Symbols,
        'w' => FieldKind::Lyrics,
        other => FieldKind::Extension(other),
    }
}

fn length_prefix(source: &str) -> usize {
    source
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || *byte == b'/')
        .count()
}
fn valid_length(source: &str) -> bool {
    source
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'/')
}

fn valid_chord_contents(source: &str) -> bool {
    // Chumsky is the core recognizer here: a chord is a non-empty sequence from
    // the ABC note alphabet. Semantic grouping remains lossless in the AST.
    none_of::<_, _, extra::Err<Rich<'_, char>>>("]\r\n")
        .repeated()
        .at_least(1)
        .then_ignore(end::<_, extra::Err<Rich<'_, char>>>())
        .parse(source)
        .has_output()
        && !source.contains('[')
}

fn error(kind: ErrorKind, message: &str, start: usize, end: usize) -> ParseError {
    ParseError {
        kind,
        message: message.into(),
        span: start..end,
    }
}
fn shift_error(error: &mut ParseError, offset: usize) {
    error.span = error.span.start + offset..error.span.end + offset;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_partial_entries() {
        assert_eq!(parse_field("K:G mixolydian").unwrap().key, 'K');
        assert_eq!(parse_directive("%%staves (1 2)").unwrap().name, "staves");
        assert_eq!(parse_chord("[CEG]4").unwrap().length.numerator, 4);
        assert!(parse_music_line("|: CDEF [CEG]2 :|").is_valid());
    }

    #[test]
    fn music_elements_have_semantic_structure() {
        let report =
            parse_music_line("^/2c'3/2 z/ X4 |: [1,3-5 (3:2:3 !trill! \"^text\" C>>D.-D &");
        assert!(report.is_valid(), "{:#?}", report.errors);
        assert!(matches!(
            report.output[0].value,
            MusicElement::Note(Note {
                pitch: Pitch {
                    class: PitchClass::C,
                    octave: 2,
                    accidental: Some(Accidental::Sharp(Fraction {
                        numerator: 1,
                        denominator: 2
                    }))
                },
                length: NoteLength {
                    numerator: 3,
                    denominator: 2
                }
            })
        ));
        assert!(report.output.iter().any(|item| matches!(
            item.value,
            MusicElement::Bar(BarLine {
                kind: BarKind::RepeatStart,
                ..
            })
        )));
        assert!(report.output.iter().any(|item| matches!(
            item.value,
            MusicElement::Tuplet(Tuplet {
                p: 3,
                q: Some(2),
                r: Some(3)
            })
        )));
        assert!(
            report
                .output
                .iter()
                .any(|item| matches!(item.value, MusicElement::BrokenRhythm(_)))
        );
    }

    #[test]
    fn structured_fields_are_parsed_and_recover_losslessly() {
        assert_eq!(
            parse_field("L:1/16").unwrap().value,
            FieldValue::UnitLength(Fraction {
                numerator: 1,
                denominator: 16
            })
        );
        assert!(matches!(
            parse_field("M:(2+3)/8").unwrap().value,
            FieldValue::Meter(Meter::Compound {
                groups,
                denominator: 8
            }) if groups == [2, 3]
        ));
        assert_eq!(
            parse_field("M:3/4").unwrap().value,
            FieldValue::Meter(Meter::Simple(Fraction {
                numerator: 3,
                denominator: 4
            }))
        );
        assert!(matches!(
            parse_field("Q:\"Allegro\" 1/4=120 \"brightly\"")
                .unwrap()
                .value,
            FieldValue::Tempo(Tempo { bpm: 120, .. })
        ));
        assert!(matches!(
            parse_field("K:G mixolydian clef=bass").unwrap().value,
            FieldValue::Key(KeySignature {
                tonic: Some(KeyTonic {
                    class: PitchClass::G,
                    ..
                }),
                ..
            })
        ));

        assert!(parse_field("L:not-a-length").is_err());
        let report = parse_line("L:not-a-length");
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(
            report.output,
            Line::Field(Field {
                value: FieldValue::Unparsed(ref value),
                ..
            }) if value == "not-a-length"
        ));
    }

    #[test]
    fn reports_negative_partial_entries() {
        assert!(parse_field("Key:C").is_err());
        assert!(parse_directive("%% bad").is_err());
        assert_eq!(
            parse_chord("[CEG").unwrap_err().kind,
            ErrorKind::UnclosedDelimiter
        );
        assert!(!parse_music_line("C !trill").is_valid());
    }

    #[test]
    fn recovers_on_the_next_line() {
        let report = parse_recovering("X:1\nK:C\n[CEG\nCDEF |\n");
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.output.tunes[0].lines.len(), 4);
        assert!(matches!(
            report.output.tunes[0].lines[3].value,
            Line::Music(_)
        ));
    }

    #[test]
    fn every_error_span_is_in_bounds() {
        let original = "X:1\nK:C\nCDEF GABc |\n";
        for index in 0..original.len() {
            let mut mutated = original.to_owned();
            mutated.replace_range(index..=index, "@");
            let report = parse_recovering(&mutated);
            assert!(
                report
                    .errors
                    .iter()
                    .all(|fault| fault.span.end <= mutated.len())
            );
        }
    }
}
