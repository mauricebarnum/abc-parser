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
use super::Field;
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
use chumsky::recovery::via_parser;
use chumsky::span::Span as ChumskySpan;
use std::fmt;

type WarningState<S> = extra::SimpleState<Vec<S>>;
type Extra<'src, I> = extra::Full<
    Rich<'src, char, <I as Input<'src>>::Span>,
    WarningState<<I as Input<'src>>::Span>,
    (),
>;
type ParsedMusic<S> = Vec<Spanned<MusicElement<SourceText<S>>, S>>;
type ParsedLine<S> = Spanned<Line<S, SourceText<S>>, S>;
type DocumentParse<'src, I> = ParseResult<
    ParsedDocument<<I as Input<'src>>::Span>,
    Rich<'src, char, <I as Input<'src>>::Span>,
>;
type DocumentParseWithWarnings<'src, I> = (DocumentParse<'src, I>, Vec<<I as Input<'src>>::Span>);

/// A blank-delimited document block parsed in its selected grammar mode.
enum ParsedBlock<S> {
    Tune {
        leading_comments: Vec<ParsedLine<S>>,
        lines: Vec<ParsedLine<S>>,
    },
    Text {
        lines: Vec<ParsedLine<S>>,
        possible_music: Option<S>,
    },
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

/// Parses one sharp or flat spelling, including microtonal fractions.
fn raised_accidental<'src, I>(
    marker: char,
) -> impl Parser<'src, I, Accidental, Extra<'src, I>> + Clone
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
        .try_map(move |((markers, numerator), denominator), span| {
            let denominator = denominator.flatten().unwrap_or(1);
            if denominator == 0 {
                return Err(Rich::custom(
                    span,
                    "accidental denominator must not be zero",
                ));
            }
            let amount = Fraction {
                numerator: numerator.unwrap_or(u32::try_from(markers).unwrap_or(u32::MAX)),
                denominator,
            };
            Ok(if marker == '^' {
                Accidental::Sharp(amount)
            } else {
                Accidental::Flat(amount)
            })
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
        raised_accidental('^'),
        raised_accidental('_'),
    ))
    .labelled("accidental")
    .as_context()
}

/// Parses a pitch class while preserving upper/lowercase octave semantics.
fn pitch_class<'src, I>() -> impl Parser<'src, I, (PitchClass, i8), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    one_of("ABCDEFGabcdefg").map(|letter: char| {
        let class = match letter.to_ascii_uppercase() {
            'A' => PitchClass::A,
            'B' => PitchClass::B,
            'C' => PitchClass::C,
            'D' => PitchClass::D,
            'E' => PitchClass::E,
            'F' => PitchClass::F,
            'G' => PitchClass::G,
            _ => unreachable!("one_of restricts the pitch alphabet"),
        };
        (class, i8::from(letter.is_ascii_lowercase()))
    })
}

/// Parses apostrophe/comma octave modifiers as a signed displacement.
fn octave<'src, I>() -> impl Parser<'src, I, i8, Extra<'src, I>> + Clone
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
        .then(pitch_class())
        .then(octave())
        .then(note_length())
        .map(
            |(((accidental, (class, base_octave)), modifier), length)| Note {
                pitch: Pitch {
                    class,
                    octave: base_octave.saturating_add(modifier),
                    accidental,
                },
                length,
            },
        )
        .labelled("note")
        .as_context()
}

/// Parses a visible or invisible single rest.
fn rest<'src, I>() -> impl Parser<'src, I, Rest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    one_of("zx")
        .then(note_length())
        .map(|(marker, length)| Rest {
            kind: if marker == 'z' {
                RestKind::Visible
            } else {
                RestKind::Invisible
            },
            length,
        })
        .labelled("rest")
        .as_context()
}

/// Parses a multi-measure rest, defaulting its measure count to one.
fn multi_measure_rest<'src, I>() -> impl Parser<'src, I, MultiMeasureRest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    one_of("ZX")
        .then(unsigned().or_not())
        .map(|(marker, measures)| MultiMeasureRest {
            invisible: marker == 'X',
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
fn field<T>(key: char, value: FieldValue<T>) -> Field<T> {
    Field {
        key,
        kind: field_kind(key),
        value,
    }
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
    let tonic = pitch_class()
        .map(|(class, _)| class)
        .then(one_of("#b").or_not())
        .map(|(class, accidental)| {
            Some(KeyTonic {
                class,
                accidental: accidental.map(|marker| {
                    if marker == '#' {
                        KeyAccidental::Sharp
                    } else {
                        KeyAccidental::Flat
                    }
                }),
            })
        });
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

/// Builds strict structured and textual information-field alternatives.
pub fn field_parser<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let structured = choice((
        just("L:")
            .ignore_then(horizontal_space())
            .ignore_then(fraction().labelled("L: unit note length").as_context())
            .map(|value| field('L', FieldValue::UnitLength(value))),
        just("M:")
            .ignore_then(horizontal_space())
            .ignore_then(
                meter()
                    .labelled("M: meter (C, C|, none, or a fraction such as 6/8)")
                    .as_context(),
            )
            .map(|value| field('M', FieldValue::Meter(value))),
        just("Q:")
            .ignore_then(horizontal_space())
            .ignore_then(
                tempo()
                    .labelled("Q: tempo (for example 1/4=120)")
                    .as_context(),
            )
            .map(|value| field('Q', FieldValue::Tempo(value))),
        just("K:")
            .ignore_then(horizontal_space())
            .ignore_then(key_signature().labelled("K: key signature").as_context())
            .map(|value| field('K', FieldValue::Key(value))),
        just("X:")
            .ignore_then(horizontal_space())
            .ignore_then(unsigned().labelled("X: reference number").as_context())
            .map(|value| field('X', FieldValue::Reference(value))),
        just("V:")
            .ignore_then(horizontal_space())
            .ignore_then(voice().labelled("V: voice definition").as_context())
            .map(|value| field('V', FieldValue::Voice(value))),
        just("P:")
            .ignore_then(horizontal_space())
            .ignore_then(parts().labelled("P: part sequence").as_context())
            .map(|value| field('P', FieldValue::Parts(value))),
        just("U:")
            .ignore_then(horizontal_space())
            .ignore_then(any().then_ignore(horizontal_space()).then_ignore(just('=')))
            .then(horizontal_space().ignore_then(remaining_text()))
            .map(|(symbol, replacement)| {
                field(
                    'U',
                    FieldValue::UserSymbol(SymbolDefinition {
                        symbol,
                        replacement,
                    }),
                )
            })
            .labelled("U: user symbol definition (symbol=replacement)")
            .as_context(),
        just("m:")
            .ignore_then(horizontal_space())
            .ignore_then(
                any()
                    .filter(|character| *character != '=')
                    .repeated()
                    .at_least(1)
                    .to_span()
                    .map(SourceText::Span),
            )
            .then_ignore(just('='))
            .then(horizontal_space().ignore_then(remaining_text()))
            .map(|(pattern, replacement)| {
                field(
                    'm',
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
        .filter(|key: &char| {
            key.is_ascii_alphabetic()
                && !matches!(key, 'L' | 'M' | 'Q' | 'K' | 'X' | 'V' | 'P' | 'U' | 'm')
        })
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
        .ignore_then(choice((
            just("L:")
                .ignore_then(horizontal_space())
                .ignore_then(
                    fraction()
                        .labelled("inline L: unit note length")
                        .as_context(),
                )
                .map(|value| field('L', FieldValue::UnitLength(value))),
            just("M:")
                .ignore_then(horizontal_space())
                .ignore_then(meter().labelled("inline M: meter").as_context())
                .map(|value| field('M', FieldValue::Meter(value))),
            just("K:")
                .ignore_then(horizontal_space())
                .ignore_then(
                    key_signature()
                        .labelled("inline K: key signature")
                        .as_context(),
                )
                .map(|value| field('K', FieldValue::Key(value))),
            just("X:")
                .ignore_then(horizontal_space())
                .ignore_then(
                    unsigned()
                        .labelled("inline X: reference number")
                        .as_context(),
                )
                .map(|value| field('X', FieldValue::Reference(value))),
        )))
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
            .map(move |name| Decoration {
                name,
                legacy_delimiter: delimiter == '+',
            })
    };
    let shorthand = one_of(".~HLMOPSTuv").map(|symbol| Decoration {
        name: SourceText::Synthesized(
            match symbol {
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
                _ => unreachable!("one_of restricts shorthand decorations"),
            }
            .into(),
        ),
        legacy_delimiter: false,
    });
    choice((named('!'), named('+'), shorthand))
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
    just('"')
        .ignore_then(one_of("^_<>@").or_not())
        .then(
            any()
                .filter(|character| !matches!(character, '"' | '\r' | '\n'))
                .repeated()
                .to_span()
                .map(SourceText::Span),
        )
        .then_ignore(just('"'))
        .map(|(marker, text)| Annotation {
            placement: match marker {
                Some('^') => AnnotationPlacement::Above,
                Some('_') => AnnotationPlacement::Below,
                Some('<') => AnnotationPlacement::Left,
                Some('>') => AnnotationPlacement::Right,
                Some('@') => AnnotationPlacement::Free,
                _ => AnnotationPlacement::ChordSymbol,
            },
            text,
        })
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
        .validate(|((p, q), r), extra, emitter| {
            let mut convert = |value| {
                if let Ok(value) = u8::try_from(value) {
                    value
                } else {
                    emitter.emit(Rich::custom(extra.span(), "tuplet value exceeds u8"));
                    u8::MAX
                }
            };
            Tuplet {
                p: convert(p),
                q: q.flatten().map(&mut convert),
                r: r.flatten().map(convert),
            }
        })
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
        inline_field().map(MusicElement::InlineField),
        chord_parser().map(MusicElement::Chord),
        variant_ending().map(MusicElement::Ending),
        grace().map(MusicElement::Grace),
        annotation().map(MusicElement::Annotation),
        decoration().map(MusicElement::Decoration),
        multi_measure_rest().map(MusicElement::MultiMeasureRest),
        rest().map(MusicElement::Rest),
        note().map(MusicElement::Note),
        tuplet().map(MusicElement::Tuplet),
        bar().map(MusicElement::Bar),
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
        just(char::from(96))
            .repeated()
            .at_least(1)
            .count()
            .map(MusicElement::BeamContinuation),
        line_break,
        beam_break,
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

/// Parses a blank-delimited block directly in tune or text mode.
fn block_parser<'src, I>() -> impl Parser<'src, I, ParsedBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let comment = just('%')
        .then_ignore(just('%').not())
        .ignore_then(remaining_text())
        .map(Line::Comment);
    let spanned_comment = spanned_nonblank_line::<I, _>(comment);
    let tune_line = spanned_nonblank_line::<I, _>(tune_line_parser());
    let text_line = spanned_nonblank_line::<I, _>(text_line_parser());
    let possible_music_line = spanned_nonblank_line::<I, _>(
        strict_music_line_parser().then_ignore(newline().rewind().or(end())),
    );
    let field_start = any()
        .filter(char::is_ascii_alphabetic)
        .then_ignore(just(':'))
        .rewind();

    let tune = spanned_comment
        .clone()
        .then_ignore(newline())
        .repeated()
        .collect::<Vec<_>>()
        .then(
            field_start.ignore_then(tune_line.clone()).then(
                newline()
                    .ignore_then(tune_line)
                    .repeated()
                    .collect::<Vec<_>>(),
            ),
        )
        .map(|(leading_comments, (reference, mut lines))| {
            lines.insert(0, reference);
            ParsedBlock::Tune {
                leading_comments,
                lines,
            }
        });
    let possible_music = spanned_comment
        .then_ignore(newline())
        .repeated()
        .collect::<Vec<_>>()
        .then(
            possible_music_line.then(
                newline()
                    .ignore_then(text_line.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            ),
        )
        .map(|(mut leading_comments, (first, mut lines))| {
            let possible_music = first.span.clone();
            leading_comments.push(first);
            leading_comments.append(&mut lines);
            ParsedBlock::Text {
                lines: leading_comments,
                possible_music: Some(possible_music),
            }
        });
    let text = text_line
        .clone()
        .then(
            newline()
                .ignore_then(text_line)
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|(first, mut lines)| {
            lines.insert(0, first);
            ParsedBlock::Text {
                lines,
                possible_music: None,
            }
        });

    choice((tune, possible_music, text))
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

/// Computes the encompassing span for a non-empty physical-line block.
fn block_span<S, T>(lines: &[Spanned<Line<S, T>, S>]) -> S
where
    S: ChumskySpan + Clone,
    S::Context: PartialEq + fmt::Debug,
    S::Offset: Ord,
{
    lines[0].span.union(lines[lines.len() - 1].span.clone())
}

/// Returns whether the first block contains only non-tune header material.
fn is_initial_header<S, T>(lines: &[Spanned<Line<S, T>, S>]) -> bool {
    !lines.iter().any(|line| {
        matches!(
            &line.value,
            Line::Field(field) if field.key == 'X'
        )
    }) && lines.iter().all(|line| {
        matches!(&line.value, Line::Comment(_) | Line::Field(_))
            || matches!(
                &line.value,
                Line::Directive(directive) if directive.kind == DirectiveKind::Other
            )
    })
}

/// Converts unclassified physical lines into one lossless free-text item.
fn push_free_text<S>(
    items: &mut Vec<Spanned<DocumentItem<S, SourceText<S>>, S>>,
    lines: &mut Vec<Spanned<Line<S, SourceText<S>>, S>>,
) where
    S: ChumskySpan + Clone,
    S::Context: PartialEq + fmt::Debug,
    S::Offset: Ord,
{
    if lines.is_empty() {
        return;
    }
    let span = block_span(lines);
    let text = lines
        .drain(..)
        .map(|line| SourceText::Span(line.span))
        .collect();
    items.push(Spanned {
        value: DocumentItem::FreeText(FreeText { lines: text }),
        span,
    });
}

/// Builds a complete-document parser with explicit text-retention behavior.
#[allow(
    clippy::too_many_lines,
    reason = "the document state machine keeps context-sensitive recovery in one Chumsky validation pass"
)]
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
    blank_line
        .repeated()
        .ignore_then(
            block_parser::<I>()
                .separated_by(block_separator)
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(newline().or_not())
        .then_ignore(horizontal_space())
        .then_ignore(end())
        .validate(move |blocks, extra, emitter| {
            let mut document = Document {
                header: Vec::new(),
                items: Vec::new(),
            };

            for (block_index, block) in blocks.into_iter().enumerate() {
                let block = match block {
                    ParsedBlock::Tune {
                        mut leading_comments,
                        lines,
                    } => {
                        if block_index == 0 && is_initial_header(&lines) {
                            leading_comments.extend(lines);
                            document.header = leading_comments;
                            continue;
                        }
                        if block_index == 0 {
                            document.header = leading_comments;
                        } else {
                            for line in leading_comments {
                                if let Line::Comment(text) = line.value {
                                    document.items.push(Spanned {
                                        value: DocumentItem::Comment(text),
                                        span: line.span,
                                    });
                                }
                            }
                        }
                        lines
                    }
                    ParsedBlock::Text {
                        lines,
                        possible_music,
                    } => {
                        if let Some(span) = possible_music {
                            extra.state().push(span);
                        }
                        if block_index == 0 && is_initial_header(&lines) {
                            document.header = lines;
                            continue;
                        }

                        let mut free_lines = Vec::new();
                        let mut iterator = lines.into_iter();
                        while let Some(line) = iterator.next() {
                            match line.value {
                                Line::Directive(directive)
                                    if directive.kind == DirectiveKind::Text
                                        || directive.kind == DirectiveKind::Center =>
                                {
                                    if options.keeps_free_text() {
                                        push_free_text(&mut document.items, &mut free_lines);
                                    } else {
                                        free_lines.clear();
                                    }
                                    if options.keeps_typeset_text() {
                                        let text = if directive.kind == DirectiveKind::Text {
                                            TypesetText::Text(directive.arguments)
                                        } else {
                                            TypesetText::Centered(directive.arguments)
                                        };
                                        document.items.push(Spanned {
                                            value: DocumentItem::TypesetText(text),
                                            span: line.span,
                                        });
                                    }
                                }
                                Line::Directive(directive)
                                    if directive.kind == DirectiveKind::BeginText =>
                                {
                                    if options.keeps_free_text() {
                                        push_free_text(&mut document.items, &mut free_lines);
                                    } else {
                                        free_lines.clear();
                                    }
                                    let start = line.span;
                                    let mut end = start.clone();
                                    let mut body = Vec::new();
                                    let mut closed = false;
                                    for body_line in iterator.by_ref() {
                                        end = body_line.span.clone();
                                        match body_line.value {
                                            Line::Directive(value)
                                                if value.kind == DirectiveKind::EndText =>
                                            {
                                                closed = true;
                                                break;
                                            }
                                            Line::Directive(value) => body.push(value.body),
                                            Line::DirectiveText(value) => body.push(value),
                                            _ => emitter.emit(Rich::custom(
                                                body_line.span,
                                                "typeset block lines must begin with %%",
                                            )),
                                        }
                                    }
                                    if !closed {
                                        emitter.emit(Rich::custom(
                                            start.clone(),
                                            "unclosed %%begintext block",
                                        ));
                                    }
                                    if options.keeps_typeset_text() {
                                        document.items.push(Spanned {
                                            value: DocumentItem::TypesetText(TypesetText::Block(
                                                body,
                                            )),
                                            span: start.union(end),
                                        });
                                    }
                                }
                                Line::Comment(text) => {
                                    if options.keeps_free_text() {
                                        push_free_text(&mut document.items, &mut free_lines);
                                    } else {
                                        free_lines.clear();
                                    }
                                    document.items.push(Spanned {
                                        value: DocumentItem::Comment(text),
                                        span: line.span,
                                    });
                                }
                                Line::Directive(directive) => {
                                    if options.keeps_free_text() {
                                        push_free_text(&mut document.items, &mut free_lines);
                                    } else {
                                        free_lines.clear();
                                    }
                                    if directive.kind == DirectiveKind::EndText {
                                        emitter.emit(Rich::custom(
                                            line.span.clone(),
                                            "%%endtext without %%begintext",
                                        ));
                                    }
                                    document.items.push(Spanned {
                                        value: DocumentItem::Directive(directive),
                                        span: line.span,
                                    });
                                }
                                Line::Field(_) => {
                                    emitter.emit(Rich::custom(
                                        line.span.clone(),
                                        "information fields are not allowed in free text",
                                    ));
                                    free_lines.push(line);
                                }
                                Line::DirectiveText(_) => {
                                    emitter.emit(Rich::custom(
                                        line.span.clone(),
                                        "invalid stylesheet directive",
                                    ));
                                    free_lines.push(line);
                                }
                                _ => free_lines.push(line),
                            }
                        }
                        if options.keeps_free_text() {
                            push_free_text(&mut document.items, &mut free_lines);
                        }
                        continue;
                    }
                };

                let span = block_span(&block);
                let mut tune_lines = Vec::new();
                let mut iterator = block.into_iter();
                while let Some(line) = iterator.next() {
                    match line.value {
                        Line::Directive(directive) if directive.kind == DirectiveKind::Text => {
                            if options.keeps_typeset_text() {
                                tune_lines.push(Spanned {
                                    value: Line::TypesetText(TypesetText::Text(
                                        directive.arguments,
                                    )),
                                    span: line.span,
                                });
                            }
                        }
                        Line::Directive(directive) if directive.kind == DirectiveKind::Center => {
                            if options.keeps_typeset_text() {
                                tune_lines.push(Spanned {
                                    value: Line::TypesetText(TypesetText::Centered(
                                        directive.arguments,
                                    )),
                                    span: line.span,
                                });
                            }
                        }
                        Line::Directive(directive)
                            if directive.kind == DirectiveKind::BeginText =>
                        {
                            let start = line.span;
                            let mut end = start.clone();
                            let mut body = Vec::new();
                            let mut closed = false;
                            for body_line in iterator.by_ref() {
                                end = body_line.span.clone();
                                match body_line.value {
                                    Line::Directive(value)
                                        if value.kind == DirectiveKind::EndText =>
                                    {
                                        closed = true;
                                        break;
                                    }
                                    Line::Directive(value) => body.push(value.body),
                                    Line::DirectiveText(value) => body.push(value),
                                    _ => emitter.emit(Rich::custom(
                                        body_line.span,
                                        "typeset block lines must begin with %%",
                                    )),
                                }
                            }
                            if !closed {
                                emitter.emit(Rich::custom(
                                    start.clone(),
                                    "unclosed %%begintext block",
                                ));
                            }
                            if options.keeps_typeset_text() {
                                tune_lines.push(Spanned {
                                    value: Line::TypesetText(TypesetText::Block(body)),
                                    span: start.union(end),
                                });
                            }
                        }
                        Line::DirectiveText(_) => {
                            emitter.emit(Rich::custom(
                                line.span.clone(),
                                "invalid stylesheet directive; expected stylesheet directive name",
                            ));
                            tune_lines.push(line);
                        }
                        _ => tune_lines.push(line),
                    }
                }
                document.items.push(Spanned {
                    value: DocumentItem::Tune(Tune { lines: tune_lines }),
                    span,
                });
            }
            document
        })
}

/// Parses a document while retaining advisory block-classification spans.
pub(super) fn parse_document<'src, I>(
    input: I,
    options: ParserOptions,
) -> DocumentParseWithWarnings<'src, I>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let mut warnings = WarningState::default();
    let result = document_parser(options).parse_with_state(input, &mut warnings);
    (result, warnings.0)
}
