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

//! Abstract syntax tree types and parser configuration for ABC music notation.

use std::ops::Range;

use super::SourceText;

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
    /// Text continuing the preceding information field via `+:`.
    FieldContinuation(T),
    /// Raw text continuing an `H:` field using deprecated implicit syntax.
    DeprecatedHistoryContinuation(T),
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
    /// A standard field whose value is explicitly empty.
    Empty,
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

impl Meter {
    /// Computes the default unit note length selected by this meter.
    ///
    /// ABC 2.1 §3.1.7 requires `L:1/16` when the effective meter is below 3/4 and
    /// `L:1/8` otherwise, including `M:C`, `M:C|`, and `M:none`.
    ///
    /// # Examples
    ///
    /// ```
    /// use abc_parser::Fraction;
    /// use abc_parser::Meter;
    ///
    /// assert_eq!(Meter::None.default_unit_length(), Fraction::EIGHTH);
    /// assert_eq!(Meter::Common.default_unit_length(), Fraction::EIGHTH);
    /// assert_eq!(
    ///     Meter::Simple(Fraction { numerator: 2, denominator: 4 }).default_unit_length(),
    ///     Fraction::SIXTEENTH
    /// );
    /// ```
    #[must_use]
    pub fn default_unit_length(&self) -> Fraction {
        let below_three_quarters = match self {
            Self::Simple(fraction) => {
                u128::from(fraction.numerator) * 4 < u128::from(fraction.denominator) * 3
            }
            Self::Compound {
                groups,
                denominator,
            } => {
                let numerator = groups.iter().map(|group| u128::from(*group)).sum::<u128>();
                numerator * 4 < u128::from(*denominator) * 3
            }
            Self::Common | Self::Cut | Self::None => false,
        };
        if below_three_quarters {
            Fraction::SIXTEENTH
        } else {
            Fraction::EIGHTH
        }
    }
}

/// A tempo from a `Q:` field.
///
/// Parsed quoted descriptions are owned directly by the source-backed AST,
/// while deprecated syntax remains raw until source resolution.
///
/// ```
/// use abc_parser::FieldValue;
/// use abc_parser::Tempo;
/// use abc_parser::parse_field;
///
/// let field = parse_field("Q:\"Allegro\" 1/4=120").unwrap();
/// assert!(matches!(
///     field.value,
///     FieldValue::Tempo(Tempo::MetronomeMark {
///         prelude: Some(ref text),
///         bpm: 120,
///         ..
///     }) if text == "Allegro"
/// ));
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Tempo<T = String> {
    /// A current metronome mark with optional surrounding descriptions.
    MetronomeMark {
        /// Optional owned text before the metronome mark.
        prelude: Option<T>,
        /// Beat lengths on the left of `=`.
        beats: Vec<Fraction>,
        /// Beats per minute.
        bpm: u32,
        /// Optional owned text after the metronome mark.
        postlude: Option<T>,
    },
    /// An owned quoted tempo indication without a metronome mark.
    TextOnly(T),
    /// A recognized deprecated tempo payload retained without interpretation.
    Deprecated(T),
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
    /// Deprecated `E:` explicit element spacing.
    ElementSpacing,
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
    /// Strict validation requires every tune to contain exactly one `X:`
    /// reference field and at least one `T:` title and `K:` key field. It
    /// also warns when `X:` is not the first information field or a
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

impl Fraction {
    /// An eighth note (1/8).
    pub const EIGHTH: Self = Self {
        numerator: 1,
        denominator: 8,
    };

    /// A sixteenth note (1/16).
    pub const SIXTEENTH: Self = Self {
        numerator: 1,
        denominator: 16,
    };

    /// Returns the greatest common divisor of two 128-bit integers.
    ///
    /// Returns 1 when both inputs are zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use abc_parser::Fraction;
    ///
    /// assert_eq!(Fraction::gcd(12, 8), 4);
    /// assert_eq!(Fraction::gcd(7, 0), 7);
    /// assert_eq!(Fraction::gcd(0, 0), 1);
    /// ```
    #[must_use]
    pub const fn gcd(mut left: u128, mut right: u128) -> u128 {
        while right != 0 {
            let remainder = left % right;
            left = right;
            right = remainder;
        }
        if left == 0 { 1 } else { left }
    }
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
///
/// First and second endings may omit `[` when adjacent to a bar line, as in
/// `|1` or `:|2`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariantEnding {
    /// Whether the selector was introduced by an explicit `[` marker.
    pub explicit_bracket: bool,
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
    /// Notes and broken-rhythm operators inside the braces.
    pub elements: Vec<GraceElement>,
}

/// One construct permitted inside a grace-note group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraceElement {
    /// A grace note.
    Note(Note),
    /// A broken-rhythm operator between grace notes.
    BrokenRhythm(BrokenRhythm),
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

/// A tuplet timing ratio and the number of following notes it affects.
///
/// `actual` notes occupy the normal duration of `normal` notes. For example,
/// `(3:2:2` applies three-in-the-time-of-two timing to the next two notes.
///
/// # Examples
///
/// ```
/// use abc_parser::Tuplet;
///
/// let tuplet = Tuplet {
///     actual: 3,
///     normal: Some(2),
///     affected: Some(2),
/// };
/// assert_eq!(tuplet.normal_note_count(false), Some(2));
/// assert_eq!(tuplet.affected_note_count(), 2);
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tuplet {
    /// Number of tuplet notes in the timing ratio (`p` in `(p:q:r`).
    pub actual: u8,
    /// Explicit number of ordinary notes whose duration they occupy (`q`).
    ///
    /// When omitted, the default depends on `actual` and whether the active
    /// meter is compound.
    pub normal: Option<u8>,
    /// Explicit number of following notes governed by the tuplet (`r`).
    ///
    /// When omitted, this defaults to `actual`.
    pub affected: Option<u8>,
}

impl Tuplet {
    /// Returns the number of following notes governed by this tuplet.
    pub const fn affected_note_count(self) -> u8 {
        match self.affected {
            Some(affected_notes) => affected_notes,
            None => self.actual,
        }
    }

    /// Returns the ordinary-note count used by the tuplet timing ratio.
    ///
    /// `compound_meter` selects the ABC default for compact tuplets with five,
    /// seven, or nine actual notes. Returns `None` when no explicit value or
    /// standard compact default exists.
    pub const fn normal_note_count(self, compound_meter: bool) -> Option<u8> {
        if let Some(normal_notes) = self.normal {
            return Some(normal_notes);
        }
        match self.actual {
            3 | 6 => Some(2),
            2 | 4 | 8 => Some(3),
            5 | 7 | 9 => Some(if compound_meter { 3 } else { 2 }),
            _ => None,
        }
    }
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

/// Maps an information-field letter to its standardized [`FieldKind`].
pub const fn field_kind(key: char) -> FieldKind {
    match key {
        'A' => FieldKind::Area,
        'B' => FieldKind::Book,
        'C' => FieldKind::Composer,
        'D' => FieldKind::Discography,
        'E' => FieldKind::ElementSpacing,
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

#[cfg(test)]
mod tests {
    use super::Fraction;
    use super::Meter;

    #[test]
    fn meter_selects_the_standard_default_unit_length() {
        assert_eq!(Meter::None.default_unit_length(), Fraction::EIGHTH);
        assert_eq!(Meter::Common.default_unit_length(), Fraction::EIGHTH);
        assert_eq!(
            Meter::Simple(Fraction {
                numerator: 2,
                denominator: 4,
            })
            .default_unit_length(),
            Fraction::SIXTEENTH
        );
        assert_eq!(
            Meter::Simple(Fraction {
                numerator: 3,
                denominator: 4,
            })
            .default_unit_length(),
            Fraction::EIGHTH
        );
        assert_eq!(
            Meter::Compound {
                groups: vec![2, 3, 2],
                denominator: 8,
            }
            .default_unit_length(),
            Fraction::EIGHTH
        );
    }
}
