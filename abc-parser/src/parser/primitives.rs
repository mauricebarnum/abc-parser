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

//! Low-level lexical, numeric, pitch-letter, and text/comment combinators.

use chumsky::IterParser;
use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::input::ValueInput;
use chumsky::prelude::any;
use chumsky::prelude::choice;
use chumsky::prelude::end;
use chumsky::prelude::just;
use chumsky::prelude::one_of;
use chumsky::prelude::select;

use super::Extra;
use super::ParsedLine;
use crate::Fraction;
use crate::Line;
use crate::NoteLength;
use crate::PitchClass;
use crate::SourceText;
use crate::Spanned;

/// A pitch letter and the octave offset implied by its case.
pub(crate) struct ParsedPitchLetter {
    pub(crate) class: PitchClass,
    pub(crate) octave_offset: i8,
}

/// Parses a character other than a physical line ending.
pub(crate) fn line_character<'src, I>() -> impl Parser<'src, I, char, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    any().filter(|character| !matches!(character, '\r' | '\n'))
}

/// Parses horizontal spacing without allocating text.
pub(crate) fn horizontal_space<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    one_of(" \t").repeated().ignored()
}

/// Parses a decimal u32 by folding digits and reports overflow at its span.
pub(crate) fn unsigned<'src, I>() -> impl Parser<'src, I, u32, Extra<'src, I>> + Clone
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
pub(crate) fn fraction<'src, I>() -> impl Parser<'src, I, Fraction, Extra<'src, I>> + Clone
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

/// Parses an L: unit length, whose whole-note form may omit `/1`.
pub(crate) fn unit_length<'src, I>() -> impl Parser<'src, I, Fraction, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    unsigned()
        .then(just('/').ignore_then(unsigned()).or_not())
        .try_map(|(numerator, denominator), span| {
            let denominator = denominator.unwrap_or(1);
            (denominator != 0)
                .then_some(Fraction {
                    numerator,
                    denominator,
                })
                .ok_or_else(|| Rich::custom(span, "fraction denominator must not be zero"))
        })
        .then_ignore(one_of(" \t%]\r\n").rewind().ignored().or(end()))
        .labelled("unit note length such as 1/8 or 1")
        .as_context()
}

/// Parses an optional ABC note-length suffix and computes its rational value.
pub fn note_length_parser<'src, I>() -> impl Parser<'src, I, NoteLength, Extra<'src, I>> + Clone
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

/// Parses a pitch letter with offset zero for uppercase and one for lowercase.
pub(crate) fn pitch_letter<'src, I>()
-> impl Parser<'src, I, ParsedPitchLetter, Extra<'src, I>> + Clone
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

/// Parses a quoted text body and returns only its interior span.
pub(crate) fn quoted_text<'src, I>()
-> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
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

/// Parses a quoted text body into text owned directly by the AST.
pub(crate) fn quoted_owned_text<'src, I>() -> impl Parser<'src, I, String, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    just('"')
        .ignore_then(
            any()
                .filter(|character| !matches!(character, '"' | '\r' | '\n'))
                .repeated()
                .collect::<String>(),
        )
        .then_ignore(just('"'))
        .labelled("quoted text")
        .as_context()
}

/// Parses a non-whitespace token as a source span.
pub(crate) fn token_text<'src, I>()
-> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
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
pub(crate) fn remaining_text<'src, I>()
-> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just('\\').then(line_character()).ignored(),
        any()
            .filter(|character| !matches!(character, '%' | '\r' | '\n'))
            .ignored(),
    ))
    .repeated()
    .to_span()
    .map(SourceText::Span)
}

/// Parses the uninterpreted remainder of a comment line.
pub(crate) fn comment_text<'src, I>()
-> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    line_character().repeated().to_span().map(SourceText::Span)
}

/// Parses the uninterpreted remainder of an inline information field value.
pub(crate) fn inline_field_text<'src, I>()
-> impl Parser<'src, I, SourceText<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    any()
        .filter(|character| !matches!(character, ']' | '\r' | '\n'))
        .repeated()
        .to_span()
        .map(SourceText::Span)
}

/// Parses a comment line, including the permitted leading horizontal space.
pub(crate) fn comment_line<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    horizontal_space()
        .ignore_then(just('%').then_ignore(just('%').not()))
        .ignore_then(comment_text())
        .map(Line::Comment)
}

/// Consumes optional trailing whitespace and an end-of-line comment.
pub(crate) fn trailing_comment<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    horizontal_space()
        .then(
            just('%')
                .then_ignore(just('%').not())
                .ignore_then(comment_text())
                .or_not(),
        )
        .ignored()
}

/// Requires a physical line to contain a non-whitespace character.
pub(crate) fn nonblank_line<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
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
pub(crate) fn spanned_nonblank_line<'src, I, P>(
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

/// Parses a platform-independent physical newline.
pub(crate) fn newline<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    choice((
        just("\r\n").ignored(),
        just('\n').ignored(),
        just('\r').ignored(),
    ))
}
