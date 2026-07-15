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
//! The companion [`abc-transpose` command](../abc_transpose/index.html)
//! transposes complete ABC files by destination key or chromatic interval.
//!
//! See [`architecture`] for parser flows and an overview of the public AST.

use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::input::ValueInput;
use chumsky::span::SimpleSpan;
use chumsky::span::Span as ChumskySpan;
use std::fmt;
use std::fmt::Write as _;
use std::ops::Range;

mod combinators;
mod emit;
mod source;

include!(concat!(env!("OUT_DIR"), "/architecture.rs"));

pub use combinators::chord_parser;
pub use combinators::directive_parser;
pub use combinators::field_parser;
pub use combinators::line_parser;
pub use combinators::music_element_parser;
pub use combinators::music_line_parser;
pub use emit::AbcEmitter;
pub use emit::EmitOptions;
pub use emit::NoteLengthStyle;
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
    /// Optional initial file-header lines, excluding its terminating blank.
    pub header: Vec<Spanned<Line<S, T>, S>>,
    /// Tunes and text annotations in source order after the file header.
    pub items: Vec<Spanned<DocumentItem<S, T>, S>>,
}

impl<S, T> Document<S, T> {
    /// Iterates over tunes in file order.
    pub fn tunes(&self) -> impl Iterator<Item = &Tune<S, T>> {
        self.items.iter().filter_map(|item| match &item.value {
            DocumentItem::Tune(tune) => Some(tune),
            _ => None,
        })
    }

    /// Iterates mutably over tunes in file order.
    pub fn tunes_mut(&mut self) -> impl Iterator<Item = &mut Tune<S, T>> {
        self.items
            .iter_mut()
            .filter_map(|item| match &mut item.value {
                DocumentItem::Tune(tune) => Some(tune),
                _ => None,
            })
    }
}

/// One ordered, file-level section after the optional file header.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DocumentItem<S = Span, T = SourceText<S>> {
    /// An ABC tune terminated by an empty line or end of file.
    Tune(Tune<S, T>),
    /// One or more lines of non-typeset annotation text.
    FreeText(FreeText<T>),
    /// Text introduced by standard typesetting directives.
    TypesetText(TypesetText<T>),
    /// A file-level comment outside the file header and tunes.
    Comment(T),
    /// A file-level stylesheet directive outside a tune.
    Directive(Directive<T>),
}

/// A contiguous free-text block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FreeText<T = String> {
    /// Physical text lines in source order.
    pub lines: Vec<T>,
}

/// Typeset text introduced by ABC text directives.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TypesetText<T = String> {
    /// A `%%text` text string.
    Text(T),
    /// A centered `%%center` text string.
    Centered(T),
    /// A `%%begintext` through `%%endtext` block.
    Block(Vec<T>),
}

/// A parser output whose source-derived text is represented by spans.
pub type ParsedDocument<S> = Document<S, SourceText<S>>;

/// A standalone document whose textual values are owned.
pub type OwnedDocument<S> = Document<S, String>;

/// One field-led ABC tune, with or without an `X:` field.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tune<S = Span, T = SourceText<S>> {
    /// All lines belonging to the tune, including its opening field.
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
    /// Tune-local typeset text.
    TypesetText(TypesetText<T>),
    /// Raw text following `%%` when it is not a valid directive.
    DirectiveText(T),
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
    /// Standard semantic category when this is a text directive.
    pub kind: DirectiveKind,
    /// Exact directive body following the leading `%%`.
    pub body: T,
}

/// Semantic category of a stylesheet directive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectiveKind {
    /// `%%text`.
    Text,
    /// `%%center`.
    Center,
    /// `%%begintext`.
    BeginText,
    /// `%%endtext`.
    EndText,
    /// Any other stylesheet directive.
    Other,
}

/// Controls optional document text retained in parser output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParserOptions {
    retain_free_text: bool,
    retain_typeset_text: bool,
    strict: bool,
}

impl ParserOptions {
    /// Creates options retaining both free and typeset text.
    pub const fn new() -> Self {
        Self {
            retain_free_text: true,
            retain_typeset_text: true,
            strict: false,
        }
    }

    /// Selects whether free-text blocks are retained in the AST.
    #[must_use]
    pub const fn retain_free_text(mut self, retain: bool) -> Self {
        self.retain_free_text = retain;
        self
    }

    /// Selects whether file- and tune-level typeset text is retained.
    #[must_use]
    pub const fn retain_typeset_text(mut self, retain: bool) -> Self {
        self.retain_typeset_text = retain;
        self
    }

    /// Selects strict validation for complete ABC documents.
    ///
    /// Strict validation requires every tune to contain an `X:` reference
    /// field. It also warns when `X:` is not the first information field or a
    /// header-level `K:` is not the last. Parsing remains recovering when
    /// these requirements are not met.
    #[must_use]
    pub const fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Returns whether free text is retained.
    pub const fn keeps_free_text(self) -> bool {
        self.retain_free_text
    }

    /// Returns whether typeset text is retained.
    pub const fn keeps_typeset_text(self) -> bool {
        self.retain_typeset_text
    }

    /// Returns whether strict document validation is enabled.
    pub const fn is_strict(self) -> bool {
        self.strict
    }
}

impl Default for ParserOptions {
    fn default() -> Self {
        Self::new()
    }
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

/// Classification of an error or advisory parser diagnostic.
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
    /// A free-text block looks like tune material without an opening field.
    MissingReference,
    /// A tune-header field does not appear in its recommended position.
    InvalidFieldOrder,
}

/// A recoverable syntax error with an exact source location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError<S = Span> {
    /// Error category suitable for programmatic handling.
    pub kind: ErrorKind,
    /// Human-readable explanation.
    pub message: String,
    /// Half-open native span in the original input.
    pub span: S,
}

impl<S> fmt::Display for ParseError<S>
where
    S: ChumskySpan,
    S::Offset: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.message,
            self.span.start(),
            self.span.end()
        )
    }
}

impl<S> std::error::Error for ParseError<S>
where
    S: ChumskySpan + fmt::Debug,
    S::Offset: fmt::Display,
{
}

impl<S> ParseError<S>
where
    S: ChumskySpan,
    S::Offset: fmt::Display,
{
    /// Renders this error with line, column, and source context when available.
    ///
    /// ```
    /// use abc_parser::ErrorKind;
    /// use abc_parser::ParseError;
    ///
    /// let error = ParseError {
    ///     kind: ErrorKind::InvalidField,
    ///     message: "invalid M: field value".to_owned(),
    ///     span: 4..10,
    /// };
    /// let diagnostic = error.diagnostic("X:1\nM:nope\n");
    /// assert!(diagnostic.starts_with("2:1: invalid M: field value"));
    /// assert!(diagnostic.contains("2 | M:nope"));
    /// ```
    ///
    /// Falls back to [`Display`](fmt::Display) when `resolver` no longer has the
    /// complete source or the stored native span is invalid for that source.
    pub fn diagnostic<R>(&self, resolver: &R) -> String
    where
        R: SourceResolver<S> + ?Sized,
    {
        resolver
            .full_source()
            .zip(resolver.diagnostic_range(&self.span))
            .map_or_else(
                || self.to_string(),
                |(source, range)| {
                    render_diagnostic(&self.message, &range, &source)
                        .unwrap_or_else(|| self.to_string())
                },
            )
    }
}

/// A non-fatal parser advisory with an exact source location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseWarning<S = Span> {
    /// Diagnostic category suitable for programmatic handling.
    pub kind: ErrorKind,
    /// Human-readable explanation.
    pub message: String,
    /// Half-open native span in the original input.
    pub span: S,
}

impl<S> fmt::Display for ParseWarning<S>
where
    S: ChumskySpan,
    S::Offset: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}..{}",
            self.message,
            self.span.start(),
            self.span.end()
        )
    }
}

impl<S> ParseWarning<S>
where
    S: ChumskySpan,
    S::Offset: fmt::Display,
{
    /// Renders this warning with line, column, and source context when available.
    pub fn diagnostic<R>(&self, resolver: &R) -> String
    where
        R: SourceResolver<S> + ?Sized,
    {
        resolver
            .full_source()
            .zip(resolver.diagnostic_range(&self.span))
            .map_or_else(
                || self.to_string(),
                |(source, range)| {
                    render_diagnostic(&self.message, &range, &source)
                        .unwrap_or_else(|| self.to_string())
                },
            )
    }
}

/// Renders one byte-spanned diagnostic against its complete source.
fn render_diagnostic(message: &str, span: &Span, source: &str) -> Option<String> {
    let start = span.start;
    let end = span.end;
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return None;
    }

    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| start + offset);
    let line_number = source[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let column = source[line_start..start].chars().count() + 1;
    let prefix = source[line_start..start]
        .chars()
        .map(|character| if character == '\t' { '\t' } else { ' ' })
        .collect::<String>();
    let highlight_end = end.min(line_end);
    let highlight_width = source[start..highlight_end].chars().count().max(1);
    let marker = "^".repeat(highlight_width);
    let gutter_width = line_number.to_string().len();
    Some(format!(
        "{line_number}:{column}: {}\n{empty:>gutter_width$} |\n{line_number:>gutter_width$} | {}\n{empty:>gutter_width$} | {prefix}{marker}",
        message,
        &source[line_start..line_end],
        empty = "",
    ))
}

/// The syntax tree and every diagnostic produced during recovering parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseReport<T, S = Span> {
    /// Parsed or recovered syntax value.
    pub output: Option<T>,
    /// Diagnostics in source order.
    pub errors: Vec<ParseError<S>>,
    /// Non-fatal advisories in source order.
    pub warnings: Vec<ParseWarning<S>>,
}

impl<T, S> ParseReport<T, S> {
    /// Returns whether parsing completed without diagnostics.
    pub const fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns whether parsing produced any non-fatal advisories.
    pub const fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }
}

/// Parses a complete ABC document with default options and physical-line error recovery.
///
/// The returned AST uses the input's native spans and retains source-derived
/// text as [`SourceText`] values. Use [`IntoOwnedAst::into_owned`] with the
/// original input to create a standalone document.
///
/// # Examples
///
/// ```
/// use abc_parser::IntoOwnedAst;
/// use abc_parser::parse;
///
/// let source = "X:1\nT:Example\nK:C\nCDEF |\n";
/// let report = parse(source);
/// assert!(report.is_valid());
/// let document = report.output.unwrap().into_owned(source).unwrap();
/// assert_eq!(document.tunes().count(), 1);
/// ```
pub fn parse<'src, I>(input: I) -> ParseReport<ParsedDocument<I::Span>, I::Span>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord + fmt::Display,
{
    parse_with_options(input, ParserOptions::default())
}

/// Parses a complete ABC document using the supplied options.
///
/// Like [`parse`], this parser uses physical-line error recovery and returns
/// source-backed text with the input's native spans.
pub fn parse_with_options<'src, I>(
    input: I,
    options: ParserOptions,
) -> ParseReport<ParsedDocument<I::Span>, I::Span>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord + fmt::Display,
{
    let (result, diagnostics) = combinators::parse_document(input, options);
    let (output, faults) = result.into_output_errors();
    let mut errors = faults.iter().map(chumsky_error).collect::<Vec<_>>();
    errors.extend(diagnostics.0);
    errors.sort_by_key(|error| (error.span.start(), error.span.end()));
    let mut warnings = diagnostics.1;
    warnings.sort_by_key(|warning| (warning.span.start(), warning.span.end()));
    ParseReport {
        output,
        errors,
        warnings,
    }
}

/// Parses one physical ABC line.
pub fn parse_line(source: &str) -> ParseReport<Line<SimpleSpan<usize>, String>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = line_parser().parse(source).into_output_errors();
    let output = output.and_then(|line| line.value.into_owned(source).ok());
    let errors = faults
        .iter()
        .map(|error| chumsky_error_with_offset(error, 0))
        .collect();
    ParseReport {
        output,
        errors,
        warnings: Vec::new(),
    }
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
                |error| chumsky_error_with_offset(&error, 0),
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
                |error| chumsky_error_with_offset(&error, 0),
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
                |error| chumsky_error_with_offset(&error, 0),
            )
        })
}

/// Parses a line as music code, recovering after malformed elements.
pub fn parse_music_line(
    source: &str,
) -> ParseReport<Vec<Spanned<MusicElement<String>, SimpleSpan<usize>>>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = music_line_parser().parse(source).into_output_errors();
    let output = output.map(|items| {
        items
            .into_iter()
            .filter_map(|item| item.into_owned(source).ok())
            .collect()
    });
    let errors = faults
        .iter()
        .map(|error| chumsky_error_with_offset(error, 0))
        .collect();
    ParseReport {
        output,
        errors,
        warnings: Vec::new(),
    }
}

fn chumsky_error<S>(error: &Rich<'_, char, S>) -> ParseError<S>
where
    S: Clone,
{
    ParseError {
        kind: ErrorKind::InvalidMusic,
        message: rich_error_message(error),
        span: error.span().clone(),
    }
}

/// Converts a line-local Chumsky error to a document-relative parse error.
fn chumsky_error_with_offset(
    error: &Rich<'_, char, SimpleSpan<usize>>,
    offset: usize,
) -> ParseError {
    let span = error.span();
    ParseError {
        kind: ErrorKind::InvalidMusic,
        message: rich_error_message(error),
        span: span.start.saturating_add(offset)..span.end.saturating_add(offset),
    }
}

/// Formats Chumsky's reason and production contexts without internal spans.
fn rich_error_message<S>(error: &Rich<'_, char, S>) -> String {
    let mut message = error
        .reason()
        .to_string()
        .replace("found '\n'", "found end of line")
        .replace("found '\r'", "found end of line");
    let contexts = error
        .contexts()
        .map(|(context, _)| context.to_string())
        .filter(|context| context != "music element")
        .collect::<Vec<_>>();
    for context in contexts.iter().skip(contexts.len().saturating_sub(2)) {
        write!(message, " while parsing {context}")
            .expect("writing diagnostic context to a String cannot fail");
    }
    message
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
        assert_eq!(
            parse_directive("%%textual value").unwrap().kind,
            DirectiveKind::Other
        );
        assert!(parse_field("V:1 name=\"Soprano\" snm=\"S\" clef=treble").is_ok());
        assert_eq!(parse_chord("[CEG]4").unwrap().length.numerator, 4);
        assert!(parse_music_line("|: CDEF [CEG]2 :|").is_valid());
    }

    #[test]
    fn music_elements_have_semantic_structure() {
        let report =
            parse_music_line("^/2c'3/2 z/ X4 |: [1,3-5 (3:2:3 !trill! \"^text\" C>>D.-D &");
        assert!(report.is_valid(), "{:#?}", report.errors);
        let output = report.output.as_ref().unwrap();
        assert!(matches!(
            output[0].value,
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
        assert!(output.iter().any(|item| matches!(
            item.value,
            MusicElement::Bar(BarLine {
                kind: BarKind::RepeatStart,
                ..
            })
        )));
        assert!(output.iter().any(|item| matches!(
            item.value,
            MusicElement::Tuplet(Tuplet {
                p: 3,
                q: Some(2),
                r: Some(3)
            })
        )));
        assert!(
            output
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
        assert_eq!(
            parse_field("L: 1/16").unwrap().value,
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
        assert_eq!(
            parse_field("M: 3/4").unwrap().value,
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
        let report = parse_line("L:  not-a-length  ");
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(
            report.output,
            Some(Line::Field(Field {
                value: FieldValue::Unparsed(ref value),
                ..
            })) if value == "not-a-length"
        ));
    }

    #[test]
    fn malformed_structured_fields_report_expected_syntax() {
        for (source, context) in [
            ("L:1/x", "L: unit note length"),
            ("M:6/x", "M: meter"),
            ("Q:1/4=x", "Q: tempo"),
            ("K:?", "K: key signature"),
            ("X:nope", "X: reference number"),
            ("V:", "V: voice definition"),
            ("P:@", "P: part sequence"),
            ("U:symbol", "U: user symbol definition"),
            ("m:pattern", "m: macro definition"),
        ] {
            let report = parse_line(source);
            assert!(!report.is_valid(), "{source}");
            assert!(
                report.errors.iter().any(|error| {
                    error.message.contains("found") && error.message.contains(context)
                }),
                "{source}: {:#?}",
                report.errors
            );
            assert!(matches!(
                report.output,
                Some(Line::Field(Field {
                    value: FieldValue::Unparsed(_),
                    ..
                }))
            ));
        }
    }

    #[test]
    fn malformed_music_reports_expected_syntax_and_recovers() {
        for (source, expected) in [
            ("[CEG", "chord"),
            ("{C", "grace group"),
            ("\"Am", "annotation"),
            ("!trill", "decoration"),
            ("(999", "tuplet"),
            ("[M:6/x]", "inline M: meter"),
            ("?", "music element"),
        ] {
            let report = parse_music_line(source);
            assert!(!report.is_valid(), "{source}");
            assert!(
                report
                    .errors
                    .iter()
                    .any(|error| error.message.contains(expected)),
                "{source}: {:#?}",
                report.errors
            );
        }

        let document = parse("X:1\nK:C\n[CEG\nC |\n");
        assert!(!document.is_valid());
        assert!(
            document
                .errors
                .iter()
                .any(|error| { error.message.contains("']'") && error.message.contains("chord") }),
            "{:#?}",
            document.errors
        );
    }

    #[test]
    fn malformed_directives_report_expected_syntax() {
        let error = parse_directive("%%").unwrap_err();
        assert!(
            error.message.contains("stylesheet directive name"),
            "{error}"
        );

        let report = parse_line("%%");
        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message.contains("stylesheet directive name")),
            "{:#?}",
            report.errors
        );
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
        let report = parse("X:1\nK:C\n[CEG\nCDEF |\n");
        assert_eq!(report.errors.len(), 1);
        let tune = report.output.as_ref().unwrap().tunes().next().unwrap();
        assert_eq!(tune.lines.len(), 4);
        assert!(matches!(tune.lines[3].value, Line::Music(_)));
    }

    #[test]
    fn every_error_span_is_in_bounds() {
        let original = "X:1\nK:C\nCDEF GABc |\n";
        for index in 0..original.len() {
            let mut mutated = original.to_owned();
            mutated.replace_range(index..=index, "@");
            let report = parse(mutated.as_str());
            assert!(
                report
                    .errors
                    .iter()
                    .all(|fault| fault.span.end <= mutated.len())
            );
        }
    }
}
