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
use chumsky::span::SimpleSpan;
use std::fmt;
use std::ops::Range;

mod combinators;
mod emit;
mod source;

pub use combinators::chord_parser;
pub use combinators::directive_parser;
pub use combinators::document_parser;
pub use combinators::field_parser;
pub use combinators::line_parser;
pub use combinators::music_element_parser;
pub use combinators::music_line_parser;
pub use combinators::parse_input;
pub use emit::ToAbc;
pub use source::IntoOwnedAst;
pub use source::PlaceholderResolver;
pub use source::ResolveError;
pub use source::SOURCE_REFERENCE_PREFIX;
pub use source::SOURCE_REFERENCE_SUFFIX;
pub use source::SourceResolver;
pub use source::SourceText;
pub use source::is_source_reference_placeholder;

/// A half-open byte range in the original input.
pub type Span = Range<usize>;

/// A syntax value paired with its location in the source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spanned<T, S = Span> {
    /// Parsed syntax value.
    pub value: T,
    /// Half-open byte range in the source.
    pub span: S,
}

/// A parsed ABC file, including file header material and tunes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Document<S = Span, T = SourceText<S>> {
    /// Lines before the first `X:` reference field.
    pub header: Vec<Spanned<Line<S, T>, S>>,
    /// Tunes found in the file.
    pub tunes: Vec<Tune<S, T>>,
}

/// A parser output whose source-derived text is represented by spans.
pub type ParsedDocument<S> = Document<S, SourceText<S>>;

/// A standalone document whose textual values are owned.
pub type OwnedDocument<S> = Document<S, String>;

/// One tune beginning with an `X:` field.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tune<S = Span, T = SourceText<S>> {
    /// All lines belonging to the tune, including `X:`.
    pub lines: Vec<Spanned<Line<S, T>, S>>,
}

/// A physical ABC source line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Line<S = Span, T = SourceText<S>> {
    /// An empty or whitespace-only line.
    Blank,
    /// A `%` comment, excluding the leading marker.
    Comment(T),
    /// A `%%` instruction.
    Directive(Directive<T>),
    /// An information field such as `T:Title`.
    Field(Field<T>),
    /// Music code represented as parsed elements.
    Music(Vec<Spanned<MusicElement<T>, S>>),
}

/// An ABC information field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field<T = String> {
    /// Single ASCII letter identifying the field.
    pub key: char,
    /// Standard meaning of the field letter.
    pub kind: FieldKind,
    /// Parsed field payload.
    pub value: FieldValue<T>,
}

/// The payload of an information field.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FieldValue<T = String> {
    /// An inherently textual metadata value.
    Text(T),
    /// An `L:` unit note length.
    UnitLength(Fraction),
    /// An `M:` time signature.
    Meter(Meter),
    /// A `Q:` tempo specification.
    Tempo(Tempo<T>),
    /// A `K:` key signature and optional parameters.
    Key(KeySignature<T>),
    /// An `X:` tune reference number.
    Reference(u32),
    /// A `V:` voice identifier and properties.
    Voice(VoiceDefinition<T>),
    /// A `P:` part-order expression.
    Parts(PartSequence<T>),
    /// A `U:` redefinable symbol assignment.
    UserSymbol(SymbolDefinition<T>),
    /// An `m:` macro assignment.
    Macro(MacroDefinition<T>),
    /// A structured field that failed to parse during recovery.
    Unparsed(T),
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
pub struct Tempo<T = String> {
    /// Optional text before the metronome mark.
    pub prelude: Option<T>,
    /// Beat lengths on the left of `=`.
    pub beats: Vec<Fraction>,
    /// Beats per minute.
    pub bpm: u32,
    /// Optional text after the metronome mark.
    pub postlude: Option<T>,
}

/// A key signature from a `K:` field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeySignature<T = String> {
    /// Tonic, or `None` for `none`/percussion.
    pub tonic: Option<KeyTonic>,
    /// Mode spelling as written, or an empty synthesized value when omitted.
    pub mode: T,
    /// Remaining clef and transposition parameters.
    pub parameters: Vec<FieldParameter<T>>,
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
pub struct VoiceDefinition<T = String> {
    /// Voice identifier.
    pub id: T,
    /// Voice properties.
    pub properties: Vec<FieldParameter<T>>,
}

/// A key/value or positional field parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldParameter<T = String> {
    /// Parameter name, absent for positional text.
    pub name: Option<T>,
    /// Unquoted parameter value.
    pub value: T,
}

/// A parsed part-order expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartSequence<T = String> {
    /// Tokens in source order.
    pub tokens: Vec<PartToken<T>>,
}

/// A token in a part-order expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartToken<T = String> {
    /// A named part.
    Part(T),
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
pub struct SymbolDefinition<T = String> {
    /// Single symbol being defined.
    pub symbol: char,
    /// Replacement music code.
    pub replacement: T,
}

/// An `m:` macro definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MacroDefinition<T = String> {
    /// Macro pattern.
    pub pattern: T,
    /// Macro replacement.
    pub replacement: T,
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
pub struct Directive<T = String> {
    /// Directive name.
    pub name: T,
    /// Remaining directive arguments.
    pub arguments: T,
}

/// A recognized element on a music-code line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MusicElement<T = String> {
    /// A pitched note.
    Note(Note),
    /// A rest or invisible spacer.
    Rest(Rest),
    /// A multi-measure rest.
    MultiMeasureRest(MultiMeasureRest),
    /// A bracketed chord.
    Chord(Chord),
    /// A bar line or repeat marker.
    Bar(BarLine<T>),
    /// A numbered or ranged variant ending.
    Ending(VariantEnding),
    /// An inline information field.
    InlineField(Field<T>),
    /// A grace-note group including its braces.
    Grace(GraceGroup),
    /// A decoration including delimiters where present.
    Decoration(Decoration<T>),
    /// A chord symbol or annotation in double quotes.
    Annotation(Annotation<T>),
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
    BeamBreak(T),
    /// Ignorable backquotes inside a beam.
    BeamContinuation(usize),
    /// A source line-break or spacing control.
    LineBreak(LineBreak),
    /// Syntax accepted for forward-compatible extensions.
    Extension(T),
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
pub struct BarLine<T = String> {
    /// Semantic classification.
    pub kind: BarKind,
    /// Exact standard-compatible spelling.
    pub source: T,
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
pub struct Decoration<T = String> {
    /// Decoration name after expanding shorthand spelling only structurally.
    pub name: T,
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
pub struct Annotation<T = String> {
    /// Placement marker.
    pub placement: AnnotationPlacement,
    /// Text without quotes or placement marker.
    pub text: T,
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
pub fn parse_recovering(source: &str) -> ParseReport<OwnedDocument<SimpleSpan<usize>>> {
    let (output, faults) = parse_input(source).into_output_errors();
    let output = output
        .and_then(|document| document.into_owned(source).ok())
        .unwrap_or_default();
    let errors = faults.iter().map(chumsky_error).collect();
    ParseReport { output, errors }
}

/// Parses a complete ABC document, failing if any syntax error is found.
///
/// # Errors
/// Returns all syntax errors found while recovering through the document.
pub fn parse(source: &str) -> Result<OwnedDocument<SimpleSpan<usize>>, Vec<ParseError>> {
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
pub fn parse_line(source: &str) -> ParseReport<Line<SimpleSpan<usize>, String>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = line_parser().parse(source).into_output_errors();
    let output = output
        .and_then(|line| line.value.into_owned(source).ok())
        .unwrap_or(Line::Blank);
    let errors = faults.iter().map(chumsky_error).collect();
    ParseReport { output, errors }
}

/// Parses a `%%` directive line.
///
/// # Errors
/// Returns an error if the prefix or directive name is invalid.
pub fn parse_directive(source: &str) -> Result<Directive, ParseError> {
    let source = source.trim_end_matches(['\r', '\n']);
    directive_parser()
        .parse(source)
        .into_result()
        .map_err(|errors| {
            errors.into_iter().next().map_or_else(
                || {
                    error(
                        ErrorKind::InvalidDirective,
                        "invalid directive",
                        0,
                        source.len(),
                    )
                },
                |error| chumsky_error(&error),
            )
        })?
        .into_owned(source)
        .map_err(|fault| {
            error(
                ErrorKind::InvalidDirective,
                &fault.to_string(),
                0,
                source.len(),
            )
        })
}

/// Parses an information field such as `K:C major`.
///
/// # Errors
/// Returns an error unless the input starts with one ASCII letter and `:`.
pub fn parse_field(source: &str) -> Result<Field, ParseError> {
    let source = source.trim_end_matches(['\r', '\n']);
    field_parser()
        .parse(source)
        .into_result()
        .map_err(|errors| {
            errors.into_iter().next().map_or_else(
                || error(ErrorKind::InvalidField, "invalid field", 0, source.len()),
                |error| chumsky_error(&error),
            )
        })?
        .into_owned(source)
        .map_err(|fault| error(ErrorKind::InvalidField, &fault.to_string(), 0, source.len()))
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
    chord_parser()
        .parse(source)
        .into_result()
        .map_err(|errors| {
            errors.into_iter().next().map_or_else(
                || error(ErrorKind::InvalidMusic, "invalid chord", 0, source.len()),
                |error| chumsky_error(&error),
            )
        })
}

/// Parses a line as music code, recovering after malformed elements.
pub fn parse_music_line(
    source: &str,
) -> ParseReport<Vec<Spanned<MusicElement<String>, SimpleSpan<usize>>>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = music_line_parser().parse(source).into_output_errors();
    let output = output
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.into_owned(source).ok())
        .collect();
    let errors = faults.iter().map(chumsky_error).collect();
    ParseReport { output, errors }
}

fn chumsky_error(error: &Rich<'_, char, SimpleSpan<usize>>) -> ParseError {
    let span = error.span().into_range();
    ParseError {
        kind: ErrorKind::InvalidMusic,
        message: error.to_string(),
        span,
    }
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

fn error(kind: ErrorKind, message: &str, start: usize, end: usize) -> ParseError {
    ParseError {
        kind,
        message: message.into(),
        span: start..end,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_partial_entries() {
        assert_eq!(parse_field("K:G mixolydian").unwrap().key, 'K');
        assert_eq!(parse_directive("%%staves (1 2)").unwrap().name, "staves");
        assert!(parse_field("V:1 name=\"Soprano\" snm=\"S\" clef=treble").is_ok());
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
        assert!(parse_chord("[CEG").is_err());
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
