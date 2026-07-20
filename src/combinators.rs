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

//! Chumsky parser constructors for ABC syntax.
//!
//! Document-level semantic parsers are intentionally boxed at selected
//! composition boundaries. Erasing those large concrete combinator types keeps
//! optimized-build monomorphization and code generation tractable. Token-level
//! music and field parsers remain unboxed so dynamic dispatch stays outside the
//! hottest character-by-character paths.

use std::fmt;

use chumsky::IterParser;
use chumsky::ParseResult;
use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::extra;
use chumsky::input::Input;
use chumsky::input::ValueInput;
use chumsky::prelude::any;
use chumsky::prelude::choice;
use chumsky::prelude::empty;
use chumsky::prelude::end;
use chumsky::prelude::just;
use chumsky::prelude::one_of;
use chumsky::prelude::select;
use chumsky::recovery::via_parser;
use chumsky::span::Span as ChumskySpan;

use super::Accidental;
use super::Annotation;
use super::AnnotationPlacement;
use super::BarKind;
use super::BarLine;
use super::BrokenRhythm;
use super::Chord;
use super::ChordMember;
use super::Decoration;
use super::Directive;
use super::DirectiveKind;
use super::Document;
use super::DocumentItem;
use super::EndingSelector;
use super::ErrorKind;
use super::Field;
use super::FieldKind;
use super::FieldParameter;
use super::FieldValue;
use super::Fraction;
use super::FreeText;
use super::GraceGroup;
use super::KeyAccidental;
use super::KeySignature;
use super::KeyTonic;
use super::Line;
use super::LineBreak;
use super::MacroDefinition;
use super::Meter;
use super::MultiMeasureRest;
use super::MusicElement;
use super::Note;
use super::NoteLength;
use super::Overlay;
use super::ParseError;
use super::ParseWarning;
use super::ParsedDocument;
use super::ParserOptions;
use super::PartSequence;
use super::PartToken;
use super::Pitch;
use super::PitchClass;
use super::Rest;
use super::RestKind;
use super::Slur;
use super::SourceText;
use super::Spanned;
use super::SymbolDefinition;
use super::Tempo;
use super::Tie;
use super::Tune;
use super::Tuplet;
use super::TypesetText;
use super::VariantEnding;
use super::VoiceDefinition;
use super::field_kind;

type ParserDiagnostics<S> = (Vec<ParseError<S>>, Vec<ParseWarning<S>>);
type ParserState<S> = extra::SimpleState<ParserDiagnostics<S>>;

type Extra<'src, I> = extra::Full<
    Rich<'src, char, <I as Input<'src>>::Span>,
    ParserState<<I as Input<'src>>::Span>,
    (),
>;
type ParsedMusic<S> = Vec<Spanned<MusicElement<SourceText<S>>, S>>;
type ParsedLine<S> = Spanned<Line<S, SourceText<S>>, S>;
type ParsedDocumentItem<S> = Spanned<DocumentItem<S, SourceText<S>>, S>;
type DocumentParse<'src, I> = ParseResult<
    ParsedDocument<<I as Input<'src>>::Span>,
    Rich<'src, char, <I as Input<'src>>::Span>,
>;
type DocumentParseWithDiagnostics<'src, I> = (
    DocumentParse<'src, I>,
    ParserDiagnostics<<I as Input<'src>>::Span>,
);

/// A blank-delimited document block parsed in its selected grammar mode.
enum ParsedBlock<S> {
    Tune {
        leading_comments: Vec<ParsedLine<S>>,
        tune: Spanned<Tune<S, SourceText<S>>, S>,
    },
    Text {
        items: Vec<Spanned<DocumentItem<S, SourceText<S>>, S>>,
    },
}

/// The first physical block, which alone may be a file header.
enum ParsedFirstBlock<S> {
    Header(Vec<ParsedLine<S>>),
    Content(ParsedBlock<S>),
}

/// One line or grouped typeset construct in a tune block.
enum ParsedTuneUnit<S> {
    Line(ParsedLine<S>),
    Typeset(Spanned<TypesetText<SourceText<S>>, S>),
}

/// A field-led block awaiting first-block header resolution.
struct ParsedTuneCandidate<S> {
    leading_comments: Vec<ParsedLine<S>>,
    units: Vec<ParsedTuneUnit<S>>,
}

/// One body line retained while parsing a grouped typeset block.
struct ParsedTypesetBodyLine<S> {
    text: Option<SourceText<S>>,
    span: S,
}

/// The source context of a structured information field.
#[derive(Clone, Copy)]
enum FieldContext {
    Physical,
    Inline,
}

/// A pitch letter and the octave offset implied by its case.
struct ParsedPitchLetter {
    class: PitchClass,
    octave_offset: i8,
}

/// Parses a character other than a physical line ending.
fn line_character<'src, I>() -> impl Parser<'src, I, char, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    any().filter(|character| !matches!(character, '\r' | '\n'))
}

/// Parses horizontal spacing without allocating text.
fn horizontal_space<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    one_of(" \t").repeated().ignored()
}

/// Parses a decimal u32 by folding digits and reports overflow at its span.
fn unsigned<'src, I>() -> impl Parser<'src, I, u32, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    any()
        .filter(char::is_ascii_digit)
        .map(|digit| digit.to_digit(10).unwrap_or_default())
        .try_foldl(
            any()
                .filter(char::is_ascii_digit)
                .map(|digit| digit.to_digit(10).unwrap_or_default())
                .repeated(),
            |value, digit, extra| {
                value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(digit))
                    .ok_or_else(|| Rich::custom(extra.span(), "integer is too large"))
            },
        )
        .labelled("integer")
}

/// Parses a positive fraction used by fields and tempo marks.
fn fraction<'src, I>() -> impl Parser<'src, I, Fraction, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    unsigned()
        .then_ignore(just('/'))
        .then(unsigned())
        .try_map(|(numerator, denominator), span| {
            (denominator != 0)
                .then_some(Fraction {
                    numerator,
                    denominator,
                })
                .ok_or_else(|| Rich::custom(span, "fraction denominator must not be zero"))
        })
        .labelled("fraction such as 1/8")
        .as_context()
}

/// Parses an optional ABC note-length suffix and computes its rational value.
fn note_length<'src, I>() -> impl Parser<'src, I, NoteLength, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    unsigned()
        .or_not()
        .then(just('/').repeated().count())
        .then(unsigned().or_not())
        .try_map(|((numerator, slashes), denominator), span| {
            let numerator = numerator.unwrap_or(1);
            let denominator = match (slashes, denominator) {
                (0, None) => 1,
                (1, Some(value)) if value != 0 => value,
                (count, None) if count > 0 => 2_u32
                    .checked_pow(u32::try_from(count).unwrap_or(u32::MAX))
                    .ok_or_else(|| Rich::custom(span, "note length denominator is too large"))?,
                _ => return Err(Rich::custom(span, "invalid note length")),
            };
            Ok(NoteLength {
                numerator,
                denominator,
            })
        })
        .labelled("note length")
        .as_context()
}

/// Parses an accidental amount following one repeated marker.
fn accidental_amount<'src, I>(
    marker: char,
) -> impl Parser<'src, I, Fraction, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just(marker)
        .repeated()
        .at_least(1)
        .count()
        .then(unsigned().or_not())
        .then(just('/').ignore_then(unsigned().or_not()).or_not())
        .try_map(|((markers, numerator), denominator), span| {
            let denominator = denominator.flatten().unwrap_or(1);
            if denominator == 0 {
                return Err(Rich::custom(
                    span,
                    "accidental denominator must not be zero",
                ));
            }
            let amount = Fraction {
                numerator: numerator.unwrap_or_else(|| u32::try_from(markers).unwrap_or(u32::MAX)),
                denominator,
            };
            Ok(amount)
        })
}

/// Parses a complete written accidental.
fn accidental<'src, I>() -> impl Parser<'src, I, Accidental, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just('=').to(Accidental::Natural),
        accidental_amount('^').map(Accidental::Sharp),
        accidental_amount('_').map(Accidental::Flat),
    ))
    .labelled("accidental")
    .as_context()
}

/// Parses a pitch letter with offset zero for uppercase and one for lowercase.
fn pitch_letter<'src, I>() -> impl Parser<'src, I, ParsedPitchLetter, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    select! {
        'A' => ParsedPitchLetter { class: PitchClass::A, octave_offset: 0 },
        'B' => ParsedPitchLetter { class: PitchClass::B, octave_offset: 0 },
        'C' => ParsedPitchLetter { class: PitchClass::C, octave_offset: 0 },
        'D' => ParsedPitchLetter { class: PitchClass::D, octave_offset: 0 },
        'E' => ParsedPitchLetter { class: PitchClass::E, octave_offset: 0 },
        'F' => ParsedPitchLetter { class: PitchClass::F, octave_offset: 0 },
        'G' => ParsedPitchLetter { class: PitchClass::G, octave_offset: 0 },
        'a' => ParsedPitchLetter { class: PitchClass::A, octave_offset: 1 },
        'b' => ParsedPitchLetter { class: PitchClass::B, octave_offset: 1 },
        'c' => ParsedPitchLetter { class: PitchClass::C, octave_offset: 1 },
        'd' => ParsedPitchLetter { class: PitchClass::D, octave_offset: 1 },
        'e' => ParsedPitchLetter { class: PitchClass::E, octave_offset: 1 },
        'f' => ParsedPitchLetter { class: PitchClass::F, octave_offset: 1 },
        'g' => ParsedPitchLetter { class: PitchClass::G, octave_offset: 1 },
    }
}

/// Parses apostrophes as positive and commas as negative octave modifiers.
fn octave_modifier<'src, I>() -> impl Parser<'src, I, i8, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    choice((
        just('\'')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| i8::try_from(count).unwrap_or(i8::MAX)),
        just(',')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| -i8::try_from(count).unwrap_or(i8::MAX)),
        empty().to(0),
    ))
}

/// Parses a pitched note directly into semantic pitch and duration values.
fn note<'src, I>() -> impl Parser<'src, I, Note, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    accidental()
        .or_not()
        .then(pitch_letter())
        .then(octave_modifier())
        .then(note_length())
        .map(|(((accidental, letter), octave_modifier), length)| Note {
            pitch: Pitch {
                class: letter.class,
                octave: letter.octave_offset.saturating_add(octave_modifier),
                accidental,
            },
            length,
        })
        .labelled("note")
        .as_context()
}

/// Parses a visible or invisible single rest.
fn rest<'src, I>() -> impl Parser<'src, I, Rest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just('z').to(RestKind::Visible),
        just('x').to(RestKind::Invisible),
    ))
    .then(note_length())
    .map(|(kind, length)| Rest { kind, length })
    .labelled("rest")
    .as_context()
}

/// Parses a multi-measure rest, defaulting its measure count to one.
fn multi_measure_rest<'src, I>() -> impl Parser<'src, I, MultiMeasureRest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((just('Z').to(false), just('X').to(true)))
        .then(unsigned().or_not())
        .map(|(invisible, measures)| MultiMeasureRest {
            invisible,
            measures: measures.unwrap_or(1),
        })
        .labelled("multi-measure rest")
        .as_context()
}

/// Parses a quoted text body and returns only its interior span.
fn quoted_text<'src, I>() -> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('"')
        .ignore_then(
            any()
                .filter(|character| !matches!(character, '"' | '\r' | '\n'))
                .repeated()
                .to_span(),
        )
        .then_ignore(just('"'))
        .map(SourceText::Span)
        .labelled("quoted text")
        .as_context()
}

/// Parses a non-whitespace token as a source span.
fn token_text<'src, I>() -> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    any()
        .filter(|character: &char| !character.is_whitespace() && !matches!(character, ']' | '='))
        .repeated()
        .at_least(1)
        .to_span()
        .map(SourceText::Span)
}

/// Parses source text to the end of a field, returning its native span.
fn remaining_text<'src, I>() -> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    line_character().repeated().to_span().map(SourceText::Span)
}

/// Parses a named or positional key/voice parameter without copying its text.
fn field_parameter<'src, I>()
-> impl Parser<'src, I, FieldParameter<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let named = any()
        .filter(|character: &char| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
        })
        .repeated()
        .at_least(1)
        .to_span()
        .map(SourceText::Span)
        .then_ignore(just('='))
        .then(choice((quoted_text(), token_text())))
        .map(|(name, value)| FieldParameter {
            name: Some(name),
            value,
        });
    choice((
        named,
        token_text().map(|value| FieldParameter { name: None, value }),
    ))
    .labelled("field parameter")
}

/// Builds a field with a preselected semantic kind and value.
const fn field<T>(key: char, value: FieldValue<T>) -> Field<T> {
    Field {
        key,
        kind: field_kind(key),
        value,
    }
}

/// Returns whether a field key has a dedicated structured value parser.
const fn has_structured_field_parser(key: char) -> bool {
    matches!(
        field_kind(key),
        FieldKind::UnitLength
            | FieldKind::Meter
            | FieldKind::Tempo
            | FieldKind::Key
            | FieldKind::Reference
            | FieldKind::Voice
            | FieldKind::Parts
            | FieldKind::UserSymbol
            | FieldKind::Macro
    )
}

/// Parses the simple and additive forms of an M: meter value.
fn meter<'src, I>() -> impl Parser<'src, I, Meter, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let fractional = just('(')
        .or_not()
        .ignore_then(
            unsigned()
                .separated_by(just('+'))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(')').or_not())
        .then_ignore(just('/'))
        .then(unsigned())
        .try_map(|(groups, denominator), span| {
            if denominator == 0 {
                return Err(Rich::custom(span, "meter denominator must not be zero"));
            }
            Ok(if let [numerator] = groups.as_slice() {
                Meter::Simple(Fraction {
                    numerator: *numerator,
                    denominator,
                })
            } else {
                Meter::Compound {
                    groups,
                    denominator,
                }
            })
        });
    choice((
        just("C|").to(Meter::Cut),
        just('C').to(Meter::Common),
        just("none").to(Meter::None),
        fractional,
    ))
}

/// Parses a Q: metronome mark while retaining quoted descriptions as spans.
fn tempo<'src, I>() -> impl Parser<'src, I, Tempo<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    quoted_text()
        .then_ignore(horizontal_space())
        .or_not()
        .then(
            fraction()
                .separated_by(one_of(" \t").repeated().at_least(1))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just('='))
        .then(unsigned())
        .then(horizontal_space().ignore_then(quoted_text()).or_not())
        .map(|(((prelude, beats), bpm), postlude)| Tempo {
            prelude,
            beats,
            bpm,
            postlude,
        })
}

/// Parses a K: tonic, optional mode, and key parameters.
fn key_signature<'src, I>()
-> impl Parser<'src, I, KeySignature<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let tonic = pitch_letter()
        .map(|letter| letter.class)
        .then(
            choice((
                just('#').to(KeyAccidental::Sharp),
                just('b').to(KeyAccidental::Flat),
            ))
            .or_not(),
        )
        .map(|(class, accidental)| Some(KeyTonic { class, accidental }));
    let no_tonic = choice((just("none"), just("perc"), just("HP"), just("Hp"))).to(None);
    choice((tonic, no_tonic))
        .then(
            any()
                .filter(char::is_ascii_alphabetic)
                .repeated()
                .at_least(1)
                .to_span()
                .map(SourceText::Span)
                .or_not(),
        )
        .then(
            one_of(" \t")
                .repeated()
                .at_least(1)
                .ignore_then(field_parameter())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|((tonic, mode), parameters)| KeySignature {
            tonic,
            mode: mode.unwrap_or_else(|| SourceText::Synthesized(String::new())),
            parameters,
        })
}

/// Parses a V: identifier and its optional properties.
fn voice<'src, I>()
-> impl Parser<'src, I, VoiceDefinition<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    token_text()
        .then(
            one_of(" \t")
                .repeated()
                .at_least(1)
                .ignore_then(field_parameter())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(id, properties)| VoiceDefinition { id, properties })
}

/// Parses a P: part-order expression into semantic sequence tokens.
fn parts<'src, I>()
-> impl Parser<'src, I, PartSequence<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        any()
            .filter(char::is_ascii_alphabetic)
            .repeated()
            .at_least(1)
            .to_span()
            .map(SourceText::Span)
            .map(PartToken::Part),
        unsigned().map(PartToken::Repeat),
        just('(').to(PartToken::Open),
        just(')').to(PartToken::Close),
        just('.').to(PartToken::Separator),
    ))
    .then_ignore(horizontal_space())
    .repeated()
    .at_least(1)
    .collect::<Vec<_>>()
    .map(|tokens| PartSequence { tokens })
}

/// Parses structured fields accepted both physically and inline.
fn common_structured_field_parser<'src, I>(
    context: FieldContext,
) -> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let (unit_length_label, meter_label, key_label, reference_label) = match context {
        FieldContext::Physical => (
            "L: unit note length",
            "M: meter (C, C|, none, or a fraction such as 6/8)",
            "K: key signature",
            "X: reference number",
        ),
        FieldContext::Inline => (
            "inline L: unit note length",
            "inline M: meter",
            "inline K: key signature",
            "inline X: reference number",
        ),
    };
    choice((
        just('L')
            .then_ignore(just(':'))
            .then(
                horizontal_space().ignore_then(fraction().labelled(unit_length_label).as_context()),
            )
            .map(|(key, value)| field(key, FieldValue::UnitLength(value))),
        just('M')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(meter().labelled(meter_label).as_context()))
            .map(|(key, value)| field(key, FieldValue::Meter(value))),
        just('K')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(key_signature().labelled(key_label).as_context()))
            .map(|(key, value)| field(key, FieldValue::Key(value))),
        just('X')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(unsigned().labelled(reference_label).as_context()))
            .map(|(key, value)| field(key, FieldValue::Reference(value))),
    ))
}

/// Builds strict structured and textual information-field alternatives.
pub fn field_parser<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let structured = choice((
        common_structured_field_parser(FieldContext::Physical),
        just('Q')
            .then_ignore(just(':'))
            .then(
                horizontal_space().ignore_then(
                    tempo()
                        .labelled("Q: tempo (for example 1/4=120)")
                        .as_context(),
                ),
            )
            .map(|(key, value)| field(key, FieldValue::Tempo(value))),
        just('V')
            .then_ignore(just(':'))
            .then(
                horizontal_space()
                    .ignore_then(voice().labelled("V: voice definition").as_context()),
            )
            .map(|(key, value)| field(key, FieldValue::Voice(value))),
        just('P')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(parts().labelled("P: part sequence").as_context()))
            .map(|(key, value)| field(key, FieldValue::Parts(value))),
        just('U')
            .then_ignore(just(':'))
            .then(
                horizontal_space()
                    .ignore_then(any().then_ignore(horizontal_space()).then_ignore(just('='))),
            )
            .then(horizontal_space().ignore_then(remaining_text()))
            .map(|((key, symbol), replacement)| {
                field(
                    key,
                    FieldValue::UserSymbol(SymbolDefinition {
                        symbol,
                        replacement,
                    }),
                )
            })
            .labelled("U: user symbol definition (symbol=replacement)")
            .as_context(),
        just('m')
            .then_ignore(just(':'))
            .then(
                horizontal_space().ignore_then(
                    any()
                        .filter(|character| *character != '=')
                        .repeated()
                        .at_least(1)
                        .to_span()
                        .map(SourceText::Span),
                ),
            )
            .then_ignore(just('='))
            .then(horizontal_space().ignore_then(remaining_text()))
            .map(|((key, pattern), replacement)| {
                field(
                    key,
                    FieldValue::Macro(MacroDefinition {
                        pattern,
                        replacement,
                    }),
                )
            })
            .labelled("m: macro definition (pattern=replacement)")
            .as_context(),
    ));
    let textual = any()
        .filter(|key: &char| key.is_ascii_alphabetic() && !has_structured_field_parser(*key))
        .then_ignore(just(':'))
        .then(remaining_text())
        .map(|(key, value)| field(key, FieldValue::Text(value)));
    choice((structured, textual))
}

/// Retains malformed structured fields as spans and emits a non-fatal error.
fn recovering_field_parser<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let fallback = any()
        .filter(char::is_ascii_alphabetic)
        .then_ignore(just(':'))
        .then(remaining_text())
        .map(|(key, value)| field(key, FieldValue::Unparsed(value)));
    field_parser().recover_with(via_parser(fallback))
}

/// Parses one bracketed chord directly from member and duration parsers.
pub fn chord_parser<'src, I>() -> impl Parser<'src, I, Chord, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((note().map(ChordMember::Note), rest().map(ChordMember::Rest)))
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just('['), just(']'))
        .then(note_length())
        .map(|(members, length)| Chord { members, length })
        .labelled("chord")
        .as_context()
}

/// Parses a variant-ending selector list such as [1,3-5.
fn variant_ending<'src, I>() -> impl Parser<'src, I, VariantEnding, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let selector = unsigned()
        .then(just('-').ignore_then(unsigned()).or_not())
        .map(|(start, end)| {
            end.map_or(EndingSelector::Number(start), |end| EndingSelector::Range {
                start,
                end,
            })
        });
    just('[')
        .ignore_then(
            selector
                .separated_by(just(','))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map(|selectors| VariantEnding { selectors })
        .labelled("variant ending")
        .as_context()
}

/// Parses standard bar spellings first, then accepts liberal bar sequences.
fn bar<'src, I>() -> impl Parser<'src, I, BarLine<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just("[|]").to(BarKind::Invisible),
        just(":||:").to(BarKind::RepeatBoth),
        just(":|:").to(BarKind::RepeatBoth),
        just("::").to(BarKind::RepeatBoth),
        just("|:").to(BarKind::RepeatStart),
        just(":|").to(BarKind::RepeatEnd),
        just("|]").to(BarKind::ThinThick),
        just("[|").to(BarKind::ThickThin),
        just("||").to(BarKind::Double),
        just(".|").to(BarKind::Dotted),
        just('|').to(BarKind::Single),
        one_of("|:")
            .repeated()
            .at_least(1)
            .ignored()
            .to(BarKind::Other),
    ))
    .map_with(|kind, extra| BarLine {
        kind,
        source: SourceText::Span(extra.span()),
    })
    .labelled("bar line")
}

/// Parses a grace group, including the optional acciaccatura slash.
fn grace<'src, I>() -> impl Parser<'src, I, GraceGroup, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('/')
        .or_not()
        .then(note().repeated().at_least(1).collect::<Vec<_>>())
        .delimited_by(just('{'), just('}'))
        .map(|(slash, notes)| GraceGroup {
            acciaccatura: slash.is_some(),
            notes,
        })
        .labelled("grace group")
        .as_context()
}

/// Parses inline structured fields whose values terminate at a closing bracket.
fn inline_field<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('[')
        .ignore_then(common_structured_field_parser(FieldContext::Inline))
        .then_ignore(just(']'))
        .labelled("inline field")
        .as_context()
}

/// Parses named, legacy, and shorthand decorations.
fn decoration<'src, I>()
-> impl Parser<'src, I, Decoration<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let named = |delimiter| {
        just(delimiter)
            .ignore_then(
                any()
                    .filter(move |character| {
                        *character != delimiter && !matches!(character, '\r' | '\n')
                    })
                    .repeated()
                    .at_least(1)
                    .to_span()
                    .map(SourceText::Span),
            )
            .then_ignore(just(delimiter))
    };
    let shorthand_name = select! {
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
    };
    let shorthand = shorthand_name.map(|name| Decoration {
        name: SourceText::Synthesized(name.into()),
        legacy_delimiter: false,
    });
    choice((
        named('!').map(|name| Decoration {
            name,
            legacy_delimiter: false,
        }),
        named('+').map(|name| Decoration {
            name,
            legacy_delimiter: true,
        }),
        shorthand,
    ))
    .labelled("decoration")
    .as_context()
}

/// Parses a quoted chord symbol or positioned annotation.
fn annotation<'src, I>()
-> impl Parser<'src, I, Annotation<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let placement = choice((
        just('^').to(AnnotationPlacement::Above),
        just('_').to(AnnotationPlacement::Below),
        just('<').to(AnnotationPlacement::Left),
        just('>').to(AnnotationPlacement::Right),
        just('@').to(AnnotationPlacement::Free),
        empty().to(AnnotationPlacement::ChordSymbol),
    ));
    just('"')
        .ignore_then(placement)
        .then(
            any()
                .filter(|character| !matches!(character, '"' | '\r' | '\n'))
                .repeated()
                .to_span()
                .map(SourceText::Span),
        )
        .then_ignore(just('"'))
        .map(|(placement, text)| Annotation { placement, text })
        .labelled("annotation")
        .as_context()
}

/// Parses compact and extended tuplet prefixes.
fn tuplet<'src, I>() -> impl Parser<'src, I, Tuplet, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('(')
        .ignore_then(unsigned())
        .then(just(':').ignore_then(unsigned().or_not()).or_not())
        .then(just(':').ignore_then(unsigned().or_not()).or_not())
        .validate(
            |((actual_notes, normal_notes), affected_notes), extra, emitter| {
                let mut convert = |value| {
                    if let Ok(value) = u8::try_from(value) {
                        value
                    } else {
                        emitter.emit(Rich::custom(extra.span(), "tuplet value exceeds u8"));
                        u8::MAX
                    }
                };
                Tuplet {
                    actual: convert(actual_notes),
                    normal: normal_notes.flatten().map(&mut convert),
                    affected: affected_notes.flatten().map(convert),
                }
            },
        )
        .labelled("tuplet")
        .as_context()
}

/// Parses repeated broken-rhythm operators while retaining direction.
fn broken_rhythm<'src, I>() -> impl Parser<'src, I, BrokenRhythm, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    choice((
        just('>')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| BrokenRhythm {
                greater: true,
                count: u8::try_from(count).unwrap_or(u8::MAX),
            }),
        just('<')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| BrokenRhythm {
                greater: false,
                count: u8::try_from(count).unwrap_or(u8::MAX),
            }),
    ))
    .labelled("broken-rhythm marker")
}

/// Parses one recognized music-code element without an extension fallback.
fn semantic_music_element_parser<'src, I>()
-> impl Parser<'src, I, MusicElement<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let beam_break = one_of(" \t")
        .repeated()
        .at_least(1)
        .to_span()
        .map(SourceText::Span)
        .map(MusicElement::BeamBreak);
    let line_break = choice((
        just('\\').to(LineBreak::Continue),
        just("$$").to(LineBreak::Paragraph),
        just('$').to(LineBreak::Break),
        just('y').to(LineBreak::Space),
    ))
    .map(MusicElement::LineBreak);
    let plain_open_slur = just('(')
        .then_ignore(
            any()
                .filter(|character: &char| !character.is_ascii_digit())
                .rewind()
                .ignored()
                .or(end()),
        )
        .to(MusicElement::Slur(Slur {
            opening: true,
            dotted: false,
        }));
    choice((
        note().map(MusicElement::Note),
        beam_break,
        multi_measure_rest().map(MusicElement::MultiMeasureRest),
        rest().map(MusicElement::Rest),
        line_break,
        inline_field().map(MusicElement::InlineField),
        chord_parser().map(MusicElement::Chord),
        variant_ending().map(MusicElement::Ending),
        bar().map(MusicElement::Bar),
        grace().map(MusicElement::Grace),
        annotation().map(MusicElement::Annotation),
        decoration().map(MusicElement::Decoration),
        tuplet().map(MusicElement::Tuplet),
        just("(&").to(MusicElement::Overlay(Overlay::Start)),
        just("&)").to(MusicElement::Overlay(Overlay::End)),
        just('&').to(MusicElement::Overlay(Overlay::NextVoice)),
        just(".(").to(MusicElement::Slur(Slur {
            opening: true,
            dotted: true,
        })),
        just(".)").to(MusicElement::Slur(Slur {
            opening: false,
            dotted: true,
        })),
        plain_open_slur,
        just(')').to(MusicElement::Slur(Slur {
            opening: false,
            dotted: false,
        })),
        just(".-").to(MusicElement::Tie(Tie { dotted: true })),
        just('-').to(MusicElement::Tie(Tie { dotted: false })),
        broken_rhythm().map(MusicElement::BrokenRhythm),
        just('`')
            .repeated()
            .at_least(1)
            .count()
            .map(MusicElement::BeamContinuation),
    ))
    .labelled("music element")
    .as_context()
}

/// Parses one music element while preserving the strict parser's failure.
fn diagnostic_music_element_parser<'src, I>()
-> impl Parser<'src, I, Spanned<MusicElement<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let fallback = line_character()
        .map_with(|_, extra| MusicElement::Extension(SourceText::Span(extra.span())));
    semantic_music_element_parser()
        .recover_with(via_parser(fallback))
        .map_with(|value, extra| Spanned {
            value,
            span: extra.span(),
        })
}

/// Builds a validating parser for one semantic music-code element.
pub fn music_element_parser<'src, I>()
-> impl Parser<'src, I, Spanned<MusicElement<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    diagnostic_music_element_parser::<I>()
}

/// Builds a parser for a complete physical music-code line.
pub fn music_line_parser<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    diagnostic_music_element_parser::<I>()
        .repeated()
        .at_least(1)
        .collect()
}

/// Builds a strict parser for one %% directive using span-backed text.
pub fn directive_parser<'src, I>()
-> impl Parser<'src, I, Directive<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let name_boundary = one_of(" \t\r\n").rewind().ignored().or(end());
    let known_name = choice((
        just("begintext")
            .then_ignore(name_boundary)
            .to(DirectiveKind::BeginText),
        just("endtext")
            .then_ignore(name_boundary)
            .to(DirectiveKind::EndText),
        just("center")
            .then_ignore(name_boundary)
            .to(DirectiveKind::Center),
        just("text")
            .then_ignore(name_boundary)
            .to(DirectiveKind::Text),
    ))
    .map_with(|kind, extra| (SourceText::Span(extra.span()), kind));
    let other_name = any()
        .filter(|character: &char| character.is_ascii_alphanumeric() || *character == '-')
        .repeated()
        .at_least(1)
        .to_span()
        .map(|span| (SourceText::Span(span), DirectiveKind::Other));
    let semantic = just("%%")
        .ignore_then(
            choice((known_name, other_name))
                .labelled("stylesheet directive name")
                .as_context(),
        )
        .then(
            one_of(" \t")
                .repeated()
                .ignore_then(remaining_text())
                .or_not(),
        )
        .map(|((name, kind), arguments)| {
            (
                name,
                arguments.unwrap_or_else(|| SourceText::Synthesized(String::new())),
                kind,
            )
        })
        .labelled("stylesheet directive")
        .as_context();
    semantic
        .rewind()
        .then(just("%%").ignore_then(remaining_text()))
        .map(|((name, arguments, kind), body)| Directive {
            name,
            arguments,
            kind,
            body,
        })
}

/// Classifies a non-blank tune line while preserving music parser failures.
fn tune_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let directive_fallback = just("%%")
        .ignore_then(remaining_text())
        .map(Line::DirectiveText);
    choice((
        directive_parser().map(Line::Directive),
        directive_fallback,
        just('%').ignore_then(remaining_text()).map(Line::Comment),
        recovering_field_parser().map(Line::Field),
        diagnostic_music_element_parser::<I>()
            .repeated()
            .at_least(1)
            .collect()
            .map(Line::Music),
    ))
}

/// Classifies a standalone line while preserving directive parser failures.
fn diagnostic_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let directive_fallback = just("%%")
        .ignore_then(remaining_text())
        .map(Line::DirectiveText);
    let directive = directive_parser()
        .map(Line::Directive)
        .recover_with(via_parser(directive_fallback));
    choice((
        directive,
        just('%').ignore_then(remaining_text()).map(Line::Comment),
        recovering_field_parser().map(Line::Field),
        diagnostic_music_element_parser::<I>()
            .repeated()
            .at_least(1)
            .collect()
            .map(Line::Music),
    ))
}

/// Classifies a non-blank text line without applying the music grammar.
fn text_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let directive_fallback = just("%%")
        .ignore_then(remaining_text())
        .map(Line::DirectiveText);
    choice((
        directive_parser().map(Line::Directive),
        directive_fallback,
        just('%').ignore_then(remaining_text()).map(Line::Comment),
        recovering_field_parser().map(Line::Field),
        line_character()
            .repeated()
            .at_least(1)
            .ignored()
            .to(Line::Music(Vec::new())),
    ))
}

/// Parses a complete music line without recovery for advisory classification.
fn strict_music_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    semantic_music_element_parser()
        .map_with(|value, extra| Spanned {
            value,
            span: extra.span(),
        })
        .repeated()
        .at_least(1)
        .collect()
        .map(Line::Music)
}

/// Requires a physical line to contain a non-whitespace character.
fn nonblank_line<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    one_of(" \t")
        .repeated()
        .ignore_then(line_character().filter(|character| !matches!(character, ' ' | '\t')))
        .rewind()
        .ignored()
}

/// Wraps a non-blank line parser with its source span.
fn spanned_nonblank_line<'src, I, P>(
    parser: P,
) -> impl Parser<'src, I, ParsedLine<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    P: Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone,
{
    nonblank_line::<I>()
        .ignore_then(parser)
        .map_with(|value, extra| Spanned {
            value,
            span: extra.span(),
        })
}

/// Returns whether a field unambiguously belongs to a tune rather than a file header.
const fn is_tune_only_header_field(kind: FieldKind) -> bool {
    matches!(
        kind,
        FieldKind::Key
            | FieldKind::Parts
            | FieldKind::Tempo
            | FieldKind::Title
            | FieldKind::Voice
            | FieldKind::Words
            | FieldKind::Reference
            | FieldKind::Symbols
            | FieldKind::Lyrics
    )
}

/// Recognizes the boundary following a nonblank block without consuming it.
fn block_end<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    choice((
        end(),
        newline()
            .ignore_then(horizontal_space())
            .then_ignore(newline().rewind().or(end()))
            .rewind(),
    ))
    .ignored()
}

/// Parses a complete `%%begintext` construct as one semantic node.
fn typeset_block_parser<'src, I>()
-> impl Parser<'src, I, Spanned<TypesetText<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let directive_line = spanned_nonblank_line::<I, _>(
        directive_parser()
            .filter(|directive| directive.kind == DirectiveKind::BeginText)
            .map(Line::Directive),
    );
    let closing_line = spanned_nonblank_line::<I, _>(
        directive_parser()
            .filter(|directive| directive.kind == DirectiveKind::EndText)
            .map(Line::Directive),
    );
    let body_line = closing_line
        .clone()
        .rewind()
        .not()
        .ignore_then(spanned_nonblank_line::<I, _>(text_line_parser()))
        .validate(|line, _, emitter| {
            let span = line.span;
            let text = match line.value {
                Line::Directive(value) => Some(value.body),
                Line::DirectiveText(value) => Some(value),
                _ => {
                    emitter.emit(Rich::custom(
                        span.clone(),
                        "typeset block lines must begin with %%",
                    ));
                    None
                }
            };
            ParsedTypesetBodyLine { text, span }
        });

    directive_line
        .then(
            newline()
                .ignore_then(body_line)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then(newline().ignore_then(closing_line).or_not())
        .validate(|((opening, body), closing), _, emitter| {
            let start = opening.span;
            let end = closing.as_ref().map_or_else(
                || {
                    body.last()
                        .map_or_else(|| start.clone(), |line| line.span.clone())
                },
                |line| line.span.clone(),
            );
            if closing.is_none() {
                emitter.emit(Rich::custom(start.clone(), "unclosed %%begintext block"));
            }
            Spanned {
                value: TypesetText::Block(body.into_iter().filter_map(|line| line.text).collect()),
                span: start.union(end),
            }
        })
        // Erase this multi-line grammar before it is reused by text and tune parsers.
        .boxed()
}

/// Parses one `%%text` or `%%center` line as semantic typeset text.
fn inline_typeset_parser<'src, I>()
-> impl Parser<'src, I, Spanned<TypesetText<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let inline = directive_parser()
        .map(|directive| match directive.kind {
            DirectiveKind::Text => Some(TypesetText::Text(directive.arguments)),
            DirectiveKind::Center => Some(TypesetText::Centered(directive.arguments)),
            _ => None,
        })
        .filter(Option::is_some)
        .map(|value| {
            Line::TypesetText(value.expect("inline typeset parser filters absent values"))
        });
    spanned_nonblank_line::<I, _>(inline)
        .map(|line| {
            let Line::TypesetText(value) = line.value else {
                unreachable!("inline typeset parser only accepts directives")
            };
            Spanned {
                value,
                span: line.span,
            }
        })
        // Keep the directive grammar from expanding every semantic text-item branch.
        .boxed()
}

/// Parses one ordinary free-text line while reserving semantic line starts.
fn free_text_line_parser<'src, I>()
-> impl Parser<'src, I, Spanned<SourceText<I::Span>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let semantic_start = choice((
        directive_parser().ignored(),
        just('%').then_ignore(just('%').not()).ignored(),
    ));
    semantic_start
        .rewind()
        .not()
        .ignore_then(spanned_nonblank_line::<I, _>(text_line_parser()))
        .validate(|line, _, emitter| {
            match &line.value {
                Line::Field(_) => emitter.emit(Rich::custom(
                    line.span.clone(),
                    "information fields are not allowed in free text",
                )),
                Line::DirectiveText(_) => emitter.emit(Rich::custom(
                    line.span.clone(),
                    "invalid stylesheet directive",
                )),
                _ => {}
            }
            Spanned {
                value: SourceText::Span(line.span.clone()),
                span: line.span,
            }
        })
        // Bound the type passed into the repeated free-text grammar below.
        .boxed()
}

/// Parses one or more adjacent ordinary lines as a single free-text item.
fn free_text_parser<'src, I>()
-> impl Parser<'src, I, Spanned<FreeText<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let line = free_text_line_parser::<I>();
    line.clone()
        .then(newline().ignore_then(line).repeated().collect::<Vec<_>>())
        .map(|(first, rest)| {
            let span = rest
                .last()
                .map_or_else(|| first.span.clone(), |line| line.span.clone());
            let lines = std::iter::once(first.value)
                .chain(rest.into_iter().map(|line| line.value))
                .collect();
            Spanned {
                value: FreeText { lines },
                span: first.span.union(span),
            }
        })
        // Hide the repeated-line combinator type before composing document items.
        .boxed()
}

/// Parses one semantic file-level text item.
fn text_item_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, Option<ParsedDocumentItem<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let block = typeset_block_parser::<I>().map(move |block| {
        options.keeps_typeset_text().then(|| Spanned {
            value: DocumentItem::TypesetText(block.value),
            span: block.span,
        })
    });
    let inline = inline_typeset_parser::<I>().map(move |text| {
        options.keeps_typeset_text().then(|| Spanned {
            value: DocumentItem::TypesetText(text.value),
            span: text.span,
        })
    });
    let comment = spanned_nonblank_line::<I, _>(
        just('%')
            .then_ignore(just('%').not())
            .ignore_then(remaining_text())
            .map(Line::Comment),
    )
    .map(|line| {
        let Line::Comment(text) = line.value else {
            unreachable!("comment parser only accepts comments")
        };
        Some(Spanned {
            value: DocumentItem::Comment(text),
            span: line.span,
        })
    });
    let directive = spanned_nonblank_line::<I, _>(directive_parser().map(Line::Directive))
        .validate(|line, _, emitter| {
            let Line::Directive(directive) = line.value else {
                unreachable!("directive parser only accepts directives")
            };
            if directive.kind == DirectiveKind::EndText {
                emitter.emit(Rich::custom(
                    line.span.clone(),
                    "%%endtext without %%begintext",
                ));
            }
            Some(Spanned {
                value: DocumentItem::Directive(directive),
                span: line.span,
            })
        });
    let free_text = free_text_parser::<I>().map(move |text| {
        options.keeps_free_text().then(|| Spanned {
            value: DocumentItem::FreeText(text.value),
            span: text.span,
        })
    });

    // This wide choice is cloned by block parsing, so erase its concrete type here.
    choice((block, inline, comment, directive, free_text)).boxed()
}

/// Parses a text-mode block directly into document items.
fn text_block_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let comment = spanned_nonblank_line::<I, _>(
        just('%')
            .then_ignore(just('%').not())
            .ignore_then(remaining_text())
            .map(Line::Comment),
    );
    let possible_music = comment
        .then_ignore(newline())
        .repeated()
        .ignore_then(spanned_nonblank_line::<I, _>(
            strict_music_line_parser().then_ignore(newline().rewind().or(end())),
        ))
        .map(|line| line.span)
        .or_not()
        .rewind();
    let item = text_item_parser::<I>(options);

    possible_music
        .then(
            item.clone().then(
                newline()
                    .ignore_then(item)
                    .repeated()
                    .collect::<Vec<_>>(),
            ),
        )
        .map_with(|(possible_music, (first, rest)), extra| {
            if let Some(span) = possible_music {
                extra.state().0.1.push(ParseWarning {
                    kind: ErrorKind::MissingReference,
                    message: "block parses as music but has no leading information field; treating it as free text"
                        .to_owned(),
                    span,
                });
            }
            ParsedBlock::Text {
                items: std::iter::once(first).chain(rest).flatten().collect(),
            }
        })
        // Prevent the repeated item grammar from propagating into document_parser.
        .boxed()
}

/// Parses a field-led block into semantic tune units before header resolution.
fn tune_candidate_parser<'src, I>()
-> impl Parser<'src, I, ParsedTuneCandidate<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let comment = spanned_nonblank_line::<I, _>(
        just('%')
            .then_ignore(just('%').not())
            .ignore_then(remaining_text())
            .map(Line::Comment),
    );
    let tune_line =
        spanned_nonblank_line::<I, _>(tune_line_parser()).validate(|line, _, emitter| {
            if matches!(line.value, Line::DirectiveText(_)) {
                emitter.emit(Rich::custom(
                    line.span.clone(),
                    "invalid stylesheet directive; expected stylesheet directive name",
                ));
            }
            line
        });
    let ordinary_unit = tune_line.map(ParsedTuneUnit::Line);
    let unit = choice((
        typeset_block_parser::<I>().map(ParsedTuneUnit::Typeset),
        inline_typeset_parser::<I>().map(ParsedTuneUnit::Typeset),
        ordinary_unit.clone(),
    ));
    let field_start = any()
        .filter(char::is_ascii_alphabetic)
        .then_ignore(just(':'))
        .rewind();

    comment
        .then_ignore(newline())
        .repeated()
        .collect::<Vec<_>>()
        .then(
            field_start
                .ignore_then(ordinary_unit)
                .then(newline().ignore_then(unit).repeated().collect::<Vec<_>>()),
        )
        .map(|(leading_comments, (first, rest))| ParsedTuneCandidate {
            leading_comments,
            units: std::iter::once(first).chain(rest).collect(),
        })
        // Tune candidates combine the largest line grammars; cap their type here.
        .boxed()
}

/// Returns whether a field-led first block contains only file-header material.
fn is_header_candidate<S>(candidate: &ParsedTuneCandidate<S>) -> bool {
    candidate.units.iter().all(|unit| match unit {
        ParsedTuneUnit::Line(line) => match &line.value {
            Line::Comment(_) => true,
            Line::Directive(directive) => directive.kind == DirectiveKind::Other,
            Line::Field(field) => !is_tune_only_header_field(field.kind),
            _ => false,
        },
        ParsedTuneUnit::Typeset(_) => false,
    })
}

/// Emits strict, non-fatal guidance for tune-header field ordering.
fn validate_tune_header_order<S>(units: &[ParsedTuneUnit<S>], state: &mut ParserState<S>)
where
    S: Clone,
{
    let fields = units.iter().filter_map(|unit| match unit {
        ParsedTuneUnit::Line(Spanned {
            value: Line::Field(field),
            span,
        }) => Some((field.kind, span)),
        _ => None,
    });
    let first_field = fields.clone().next();
    let reference = fields
        .clone()
        .find(|(kind, _)| *kind == FieldKind::Reference);
    if let (Some((first_kind, _)), Some((_, span))) = (first_field, reference)
        && first_kind != FieldKind::Reference
    {
        state.0.1.push(ParseWarning {
            kind: ErrorKind::InvalidFieldOrder,
            message: "X: reference field should be the first information field in a tune"
                .to_owned(),
            span: span.clone(),
        });
    }

    let header_fields = units
        .iter()
        .take_while(|unit| {
            !matches!(
                unit,
                ParsedTuneUnit::Line(Spanned {
                    value: Line::Music(_),
                    ..
                })
            )
        })
        .filter_map(|unit| match unit {
            ParsedTuneUnit::Line(Spanned {
                value: Line::Field(field),
                span,
            }) => Some((field.kind, span)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if let (Some((last_kind, _)), Some((_, key_span))) = (
        header_fields.last(),
        header_fields
            .iter()
            .rev()
            .find(|(kind, _)| *kind == FieldKind::Key),
    ) && *last_kind != FieldKind::Key
    {
        state.0.1.push(ParseWarning {
            kind: ErrorKind::InvalidFieldOrder,
            message: "K: key field should be the last information field in a tune header"
                .to_owned(),
            span: (*key_span).clone(),
        });
    }
}

/// Converts a resolved tune candidate and applies retention and strict validation.
fn resolve_tune_candidate<S>(
    candidate: ParsedTuneCandidate<S>,
    options: ParserOptions,
    state: &mut ParserState<S>,
) -> ParsedBlock<S>
where
    S: ChumskySpan + Clone,
    S::Context: PartialEq + fmt::Debug,
    S::Offset: Ord,
{
    let first_span = match &candidate.units[0] {
        ParsedTuneUnit::Line(line) => line.span.clone(),
        ParsedTuneUnit::Typeset(text) => text.span.clone(),
    };
    let end_span = match &candidate.units[candidate.units.len() - 1] {
        ParsedTuneUnit::Line(line) => line.span.clone(),
        ParsedTuneUnit::Typeset(text) => text.span.clone(),
    };
    let has_reference = candidate.units.iter().any(|unit| {
        matches!(
            unit,
            ParsedTuneUnit::Line(Spanned {
                value: Line::Field(Field { key: 'X', .. }),
                ..
            })
        )
    });
    if options.is_strict() && !has_reference {
        state.0.0.push(ParseError {
            kind: ErrorKind::MissingReference,
            message: "tune is missing required X: reference field".to_owned(),
            span: first_span.clone(),
        });
    }
    if options.is_strict() {
        validate_tune_header_order(&candidate.units, state);
    }
    let lines = candidate
        .units
        .into_iter()
        .filter_map(|unit| match unit {
            ParsedTuneUnit::Line(line) => Some(line),
            ParsedTuneUnit::Typeset(text) => options.keeps_typeset_text().then(|| Spanned {
                value: Line::TypesetText(text.value),
                span: text.span,
            }),
        })
        .collect();
    ParsedBlock::Tune {
        leading_comments: candidate.leading_comments,
        tune: Spanned {
            value: Tune { lines },
            span: first_span.union(end_span),
        },
    }
}

/// Resolves a field-led first block as either a file header or a tune.
fn resolve_first_tune_candidate<S>(
    candidate: ParsedTuneCandidate<S>,
    options: ParserOptions,
    state: &mut ParserState<S>,
) -> ParsedFirstBlock<S>
where
    S: ChumskySpan + Clone,
    S::Context: PartialEq + fmt::Debug,
    S::Offset: Ord,
{
    if is_header_candidate(&candidate) {
        ParsedFirstBlock::Header(
            candidate
                .leading_comments
                .into_iter()
                .chain(candidate.units.into_iter().filter_map(|unit| match unit {
                    ParsedTuneUnit::Line(line) => Some(line),
                    ParsedTuneUnit::Typeset(_) => None,
                }))
                .collect(),
        )
    } else {
        ParsedFirstBlock::Content(resolve_tune_candidate(candidate, options, state))
    }
}

/// Parses a tune-mode block directly into semantic tune lines.
fn tune_block_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    tune_candidate_parser::<I>()
        .map_with(move |candidate, extra| resolve_tune_candidate(candidate, options, extra.state()))
        // Keep candidate resolution opaque when tune and text blocks are combined.
        .boxed()
}

/// Parses a field-led first block and resolves its file-header ambiguity.
fn first_tune_or_header_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedFirstBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    tune_candidate_parser::<I>()
        .map_with(move |candidate, extra| {
            resolve_first_tune_candidate(candidate, options, extra.state())
        })
        // The first-block branch is composed separately from later tune blocks.
        .boxed()
}

/// Parses an initial block that is unambiguously eligible as a file header.
fn initial_header_parser<'src, I>()
-> impl Parser<'src, I, Vec<ParsedLine<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let comment_value = just('%')
        .then_ignore(just('%').not())
        .ignore_then(remaining_text());
    let directive_value =
        directive_parser().filter(|directive| directive.kind == DirectiveKind::Other);
    let field_start = any()
        .filter(|key: &char| {
            key.is_ascii_alphabetic() && !is_tune_only_header_field(field_kind(*key))
        })
        .then_ignore(just(':'))
        .rewind();
    let shape_line = choice((
        comment_value.clone().ignored(),
        directive_value.clone().ignored(),
        field_start.ignore_then(line_character().repeated().ignored()),
    ));
    let shape = shape_line
        .clone()
        .then(newline().ignore_then(shape_line).repeated())
        .then_ignore(block_end::<I>())
        .rewind();
    let line = spanned_nonblank_line::<I, _>(choice((
        comment_value.map(Line::Comment),
        directive_value.map(Line::Directive),
        field_start.ignore_then(tune_line_parser()),
    )));

    shape
        .ignore_then(
            line.clone()
                .then(newline().ignore_then(line).repeated().collect::<Vec<_>>())
                .map(|(first, rest)| std::iter::once(first).chain(rest).collect()),
        )
        // Isolate the speculative header grammar from the top-level choice type.
        .boxed()
}

/// Parses one non-header document block in tune or text mode.
fn block_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    // Erase both substantial alternatives before repeated document composition.
    choice((tune_block_parser(options), text_block_parser(options))).boxed()
}

/// Builds a parser for one source-spanned physical ABC line.
pub fn line_parser<'src, I>() -> impl Parser<'src, I, ParsedLine<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        diagnostic_line_parser(),
        one_of(" \t").repeated().at_least(1).to(Line::Blank),
        empty().to(Line::Blank),
    ))
    .map_with(|value, extra| Spanned {
        value,
        span: extra.span(),
    })
}

/// Parses a platform-independent physical newline.
fn newline<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    just('\r').or_not().then_ignore(just('\n')).ignored()
}

/// Converts tune-leading comments into file-level comment items.
fn comment_items<S>(
    comments: Vec<ParsedLine<S>>,
) -> impl Iterator<Item = Spanned<DocumentItem<S, SourceText<S>>, S>> {
    comments.into_iter().filter_map(|line| {
        if let Line::Comment(text) = line.value {
            Some(Spanned {
                value: DocumentItem::Comment(text),
                span: line.span,
            })
        } else {
            None
        }
    })
}

/// Returns the document items represented by one non-initial block.
fn block_items<S>(block: ParsedBlock<S>) -> Vec<Spanned<DocumentItem<S, SourceText<S>>, S>> {
    match block {
        ParsedBlock::Tune {
            leading_comments,
            tune,
        } => comment_items(leading_comments)
            .chain(std::iter::once(Spanned {
                value: DocumentItem::Tune(tune.value),
                span: tune.span,
            }))
            .collect(),
        ParsedBlock::Text { items } => items,
    }
}

/// Assembles already-semantic blocks while applying first-block comment placement.
fn assemble_document<S>(
    first: Option<ParsedFirstBlock<S>>,
    rest: Vec<ParsedBlock<S>>,
) -> ParsedDocument<S> {
    let (header, first_items) = match first {
        None => (Vec::new(), Vec::new()),
        Some(ParsedFirstBlock::Header(lines)) => (lines, Vec::new()),
        Some(ParsedFirstBlock::Content(ParsedBlock::Tune {
            leading_comments,
            tune,
        })) => (
            leading_comments,
            vec![Spanned {
                value: DocumentItem::Tune(tune.value),
                span: tune.span,
            }],
        ),
        Some(ParsedFirstBlock::Content(ParsedBlock::Text { items })) => (Vec::new(), items),
    };
    Document {
        header,
        items: first_items
            .into_iter()
            .chain(rest.into_iter().flat_map(block_items))
            .collect(),
    }
}

/// Builds a complete-document parser with explicit text-retention behavior.
fn document_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedDocument<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let blank_line = horizontal_space::<I>().then_ignore(newline());
    let block_separator = newline()
        .ignore_then(blank_line.clone().repeated().at_least(1))
        .ignored();
    let block = block_parser::<I>(options);
    let first = choice((
        first_tune_or_header_parser::<I>(options),
        initial_header_parser::<I>().map(ParsedFirstBlock::Header),
        text_block_parser::<I>(options).map(ParsedFirstBlock::Content),
    ));
    blank_line
        .repeated()
        .ignore_then(
            first.or_not().then(
                block_separator
                    .clone()
                    .ignore_then(block)
                    .repeated()
                    .collect::<Vec<_>>(),
            ),
        )
        .then_ignore(block_separator.or_not())
        .then_ignore(newline().or_not())
        .then_ignore(horizontal_space())
        .then_ignore(end())
        .map(|(first, rest)| assemble_document(first, rest))
        // Cap the public parse entry point's monomorphized type and codegen cost.
        .boxed()
}

/// Parses a document while retaining typed recovering diagnostics.
pub fn parse_document<'src, I>(
    input: I,
    options: ParserOptions,
) -> DocumentParseWithDiagnostics<'src, I>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let mut state = extra::SimpleState((Vec::new(), Vec::new()));
    let result = document_parser(options).parse_with_state(input, &mut state);
    (result, state.0)
}
