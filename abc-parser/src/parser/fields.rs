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

//! Information-field and stylesheet-directive combinators.

use chumsky::IterParser;
use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::input::ValueInput;
use chumsky::prelude::any;
use chumsky::prelude::choice;
use chumsky::prelude::empty;
use chumsky::prelude::end;
use chumsky::prelude::just;
use chumsky::prelude::one_of;
use chumsky::recovery::via_parser;

use super::Extra;
use super::primitives::fraction;
use super::primitives::horizontal_space;
use super::primitives::inline_field_text;
use super::primitives::pitch_letter;
use super::primitives::quoted_owned_text;
use super::primitives::quoted_text;
use super::primitives::remaining_text;
use super::primitives::token_text;
use super::primitives::trailing_comment;
use super::primitives::unit_length;
use super::primitives::unsigned;
use crate::Directive;
use crate::DirectiveKind;
use crate::ErrorKind;
use crate::Field;
use crate::FieldKind;
use crate::FieldParameter;
use crate::FieldValue;
use crate::Fraction;
use crate::KeyAccidental;
use crate::KeySignature;
use crate::KeyTonic;
use crate::Line;
use crate::MacroDefinition;
use crate::Meter;
use crate::ParseWarning;
use crate::PartSequence;
use crate::PartToken;
use crate::SourceText;
use crate::SymbolDefinition;
use crate::Tempo;
use crate::VoiceDefinition;
use crate::field_kind;

/// The source context of a structured information field.
#[derive(Clone, Copy)]
pub(crate) enum FieldContext {
    Physical,
    Inline,
}

/// Inspects whether the upcoming field text starts with the `linebreak` token,
/// ignoring leading horizontal space, without consuming any input.
pub(crate) fn is_linebreak_lookahead<'src, I>() -> impl Parser<'src, I, bool, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let linebreak_word = any()
        .filter(|c: &char| c.eq_ignore_ascii_case(&'l'))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'i')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'n')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'e')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'b')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'r')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'e')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'a')))
        .then(any().filter(|c: &char| c.eq_ignore_ascii_case(&'k')))
        .ignored();

    horizontal_space()
        .ignore_then(linebreak_word)
        .rewind()
        .to(true)
        .or(empty().to(false))
}

/// Parses a named or positional key/voice parameter without copying its text.
pub(crate) fn field_parameter<'src, I>()
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

/// Parses one fixed-key field using its dedicated value grammar.
pub(crate) fn structured_field_parser<'src, I, O, P, F>(
    key: char,
    value: P,
    into_value: F,
) -> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    P: Parser<'src, I, O, Extra<'src, I>> + Clone,
    F: Fn(O) -> FieldValue<SourceText<I::Span>> + Clone,
{
    just(key)
        .then_ignore(just(':'))
        .ignore_then(horizontal_space().ignore_then(value))
        .map(move |value| field(key, into_value(value)))
}

/// Parses the simple and additive forms of an M: meter value.
pub(crate) fn meter<'src, I>() -> impl Parser<'src, I, Meter, Extra<'src, I>> + Clone
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

/// Parses current and deprecated Q: tempo forms.
pub(crate) fn tempo<'src, I>()
-> impl Parser<'src, I, Tempo<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let owned_text = quoted_owned_text().map(SourceText::Synthesized);
    let mark = fraction()
        .separated_by(one_of(" \t").repeated().at_least(1))
        .at_least(1)
        .at_most(4)
        .collect::<Vec<_>>()
        .then_ignore(just('='))
        .then(unsigned());
    let described_mark = owned_text
        .clone()
        .then_ignore(horizontal_space())
        .or_not()
        .then(mark)
        .then(horizontal_space().ignore_then(owned_text.clone()).or_not())
        .map(|((prelude, (beats, bpm)), postlude)| Tempo::MetronomeMark {
            prelude,
            beats,
            bpm,
            postlude,
        });
    let text_only = owned_text.map(Tempo::TextOnly);
    let digits = any()
        .filter(char::is_ascii_digit)
        .repeated()
        .at_least(1)
        .ignored();
    let deprecated = choice((
        just('C')
            .then_ignore(horizontal_space())
            .then_ignore(just('='))
            .then_ignore(horizontal_space())
            .ignore_then(digits),
        digits,
    ))
    .to_span()
    .then_ignore(one_of(" \t%]\r\n").rewind().ignored().or(end()))
    .map_with(|span: I::Span, extra| {
        extra.state().0.warnings.push(ParseWarning {
            kind: ErrorKind::DeprecatedSyntax,
            message: "deprecated Q: tempo syntax is retained without interpretation".to_owned(),
            span: span.clone(),
            related: Vec::new(),
        });
        Tempo::Deprecated(SourceText::Span(span))
    });
    choice((described_mark, text_only, deprecated))
}

/// Parses a K: tonic, optional mode, and key parameters.
pub(crate) fn key_signature<'src, I>()
-> impl Parser<'src, I, KeySignature<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let parameter_tail = one_of(" \t")
        .repeated()
        .at_least(1)
        .ignore_then(field_parameter())
        .repeated()
        .collect::<Vec<_>>();
    let boundary = one_of(" \t]").rewind().ignored().or(end());
    let clef_parameter = choice((
        just("treble"),
        just("bass"),
        just("baritone"),
        just("tenor"),
        just("alto"),
        just("mezzosoprano"),
        just("soprano"),
        just("perc"),
    ))
    .then(
        any()
            .filter(|character: &char| character.is_ascii_digit() || matches!(character, '+' | '-'))
            .repeated(),
    )
    .then_ignore(boundary)
    .to_span()
    .map(SourceText::Span)
    .map(|value| FieldParameter { name: None, value });
    let named_parameter = any()
        .filter(|character: &char| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
        })
        .repeated()
        .at_least(1)
        .then_ignore(just('='))
        .rewind()
        .ignore_then(field_parameter());
    let keyless = choice((named_parameter, clef_parameter))
        .then(parameter_tail.clone())
        .map(|(first, rest)| KeySignature {
            tonic: None,
            mode: SourceText::Synthesized(String::new()),
            parameters: std::iter::once(first).chain(rest).collect(),
        });
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
    let pitched_or_special = choice((tonic, no_tonic))
        .then(
            any()
                .filter(char::is_ascii_alphabetic)
                .repeated()
                .at_least(1)
                .to_span()
                .map(SourceText::Span)
                .or_not(),
        )
        .then(parameter_tail)
        .map(|((tonic, mode), parameters)| KeySignature {
            tonic,
            mode: mode.unwrap_or_else(|| SourceText::Synthesized(String::new())),
            parameters,
        });
    choice((keyless, pitched_or_special))
}

/// Parses a V: identifier and its optional properties.
pub(crate) fn voice<'src, I>()
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
pub(crate) fn parts<'src, I>()
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
    .try_map(|tokens, span| {
        let mut depth = 0_u32;
        for token in &tokens {
            match token {
                PartToken::Open => depth = depth.saturating_add(1),
                PartToken::Close if depth == 0 => {
                    return Err(Rich::custom(span, "part sequence has an unmatched ')'"));
                }
                PartToken::Close => depth -= 1,
                _ => {}
            }
        }
        (depth == 0)
            .then_some(PartSequence { tokens })
            .ok_or_else(|| Rich::custom(span, "part sequence has an unclosed '('"))
    })
}

/// Parses structured fields accepted both physically and inline.
pub(crate) fn common_structured_field_parser<'src, I>(
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
    let empty_value = choice((just('K'), just('X')))
        .then_ignore(just(':'))
        .then_ignore(horizontal_space())
        .then_ignore(one_of("%]\r\n").rewind().ignored().or(end()))
        .map(|key| field(key, FieldValue::Empty));
    choice((
        empty_value,
        structured_field_parser(
            'L',
            unit_length().labelled(unit_length_label).as_context(),
            FieldValue::UnitLength,
        ),
        structured_field_parser(
            'M',
            meter().labelled(meter_label).as_context(),
            FieldValue::Meter,
        ),
        structured_field_parser(
            'K',
            key_signature().labelled(key_label).as_context(),
            FieldValue::Key,
        ),
        structured_field_parser(
            'X',
            unsigned()
                .try_map(|value, span| {
                    (value > 0)
                        .then_some(value)
                        .ok_or_else(|| Rich::custom(span, "reference number must be positive"))
                })
                .labelled(reference_label)
                .as_context(),
            FieldValue::Reference,
        ),
    ))
}

/// Parses a `+:` continuation of the preceding information field.
pub(crate) fn field_continuation<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just("+:")
        .ignore_then(remaining_text())
        .then_ignore(trailing_comment())
        .map(Line::FieldContinuation)
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
        structured_field_parser(
            'Q',
            tempo()
                .labelled("Q: tempo (for example 1/4=120)")
                .as_context(),
            FieldValue::Tempo,
        ),
        structured_field_parser(
            'V',
            voice().labelled("V: voice definition").as_context(),
            FieldValue::Voice,
        ),
        structured_field_parser(
            'P',
            parts().labelled("P: part sequence").as_context(),
            FieldValue::Parts,
        ),
        just('U')
            .then_ignore(just(':'))
            .then(
                horizontal_space().ignore_then(
                    one_of("HIJKLMNOPQRSTUVWhijklmnopqrstuvw~")
                        .then_ignore(horizontal_space())
                        .then_ignore(just('=')),
                ),
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
    let textual_instruction = textual_instruction_field_parser::<I>();
    let textual = textual_field_parser();
    choice((structured, textual_instruction, textual))
}

/// Parses an `I:` instruction field value as text, recording `I:linebreak` directives.
pub(crate) fn textual_instruction_field_parser<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('I')
        .then_ignore(just(':'))
        .then(is_linebreak_lookahead::<I>().then(remaining_text::<I>()))
        .map_with(|(key, (is_linebreak, value)), extra| {
            if let (true, SourceText::Span(span)) = (is_linebreak, &value) {
                extra.state().0.note_linebreak_instruction(span.clone());
            }
            field(key, FieldValue::Text(value))
        })
}

/// Parses a non-structured, non-instruction information field value as raw text.
pub(crate) fn textual_field_parser<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    any()
        .filter(|key: &char| key.is_ascii_alphabetic() && !has_structured_field_parser(*key))
        .then_ignore(just(':'))
        .then(remaining_text())
        .map_with(|(key, value), extra| {
            let deprecated = match key {
                'A' => Some("deprecated A: area field; use O: origin instead"),
                'E' => Some("deprecated E: element-spacing field is retained as text"),
                _ => None,
            };
            if let Some(message) = deprecated {
                let span = extra.span();
                extra.state().0.warnings.push(ParseWarning {
                    kind: ErrorKind::DeprecatedSyntax,
                    message: message.to_owned(),
                    span,
                    related: Vec::new(),
                });
            } else if matches!(field_kind(key), FieldKind::Extension(_)) {
                let span = extra.span();
                extra.state().0.warnings.push(ParseWarning {
                    kind: ErrorKind::UnrecognizedField,
                    message: format!("unrecognized information field {key}:"),
                    span,
                    related: Vec::new(),
                });
            }
            field(key, FieldValue::Text(value))
        })
}

/// Retains malformed structured fields as spans and emits a non-fatal error.
pub(crate) fn recovering_field_parser<'src, I>()
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

/// Parses inline structured fields whose values terminate at a closing bracket.
pub(crate) fn inline_field<'src, I>()
-> impl Parser<'src, I, Field<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let additional_structured = choice((
        just('Q')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(tempo().labelled("inline Q: tempo").as_context()))
            .map(|(key, value)| field(key, FieldValue::Tempo(value))),
        just('V')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(voice().labelled("inline V: voice").as_context()))
            .map(|(key, value)| field(key, FieldValue::Voice(value))),
        just('P')
            .then_ignore(just(':'))
            .then(horizontal_space().ignore_then(parts().labelled("inline P: part").as_context()))
            .map(|(key, value)| field(key, FieldValue::Parts(value))),
    ));
    let textual = any()
        .filter(|key: &char| {
            key.is_ascii_alphabetic() && !matches!(*key, 'L' | 'M' | 'K' | 'X' | 'Q' | 'V' | 'P')
        })
        .then_ignore(just(':'))
        .then(inline_field_text())
        .map_with(|(key, value), extra| {
            if matches!(field_kind(key), FieldKind::Extension(_)) {
                let span = extra.span();
                extra.state().0.warnings.push(ParseWarning {
                    kind: ErrorKind::UnrecognizedField,
                    message: format!("unrecognized inline information field [{key}:...]"),
                    span,
                    related: Vec::new(),
                });
            }
            field(key, FieldValue::Text(value))
        });
    just('[')
        .ignore_then(choice((
            common_structured_field_parser(FieldContext::Inline),
            additional_structured,
            textual,
        )))
        .then_ignore(just(']'))
        .labelled("inline field")
        .as_context()
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
    let semantic = choice((known_name, other_name))
        .labelled("stylesheet directive name")
        .as_context()
        .then(
            one_of(" \t")
                .repeated()
                .ignore_then(remaining_text())
                .or_not(),
        )
        .map_with(|((name, kind), arguments), extra| Directive {
            name,
            arguments: arguments.unwrap_or_else(|| SourceText::Synthesized(String::new())),
            kind,
            body: SourceText::Span(extra.span()),
        });
    just("%%")
        .ignore_then(semantic)
        .labelled("stylesheet directive")
        .as_context()
}

/// Preserves malformed stylesheet directive text as a semantic line.
pub(crate) fn directive_text_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just("%%")
        .ignore_then(remaining_text())
        .then_ignore(trailing_comment())
        .map(Line::DirectiveText)
}

/// Returns whether a field unambiguously belongs to a tune rather than a file header.
pub(crate) const fn is_tune_only_header_field(kind: FieldKind) -> bool {
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

/// Returns whether a field is restricted to the file or tune header per §3 line 443.
///
/// Letters `A-G`, `X-Z`, `a-g`, and `x-z` are not permitted in the tune body
/// to avoid confusion with note symbols, rests, and spacers. The deprecated
/// `A:` and `E:` fields are excluded because they already emit their own
/// `DeprecatedSyntax` advisory and would double-warn here.
pub(crate) const fn is_body_forbidden_field(kind: FieldKind) -> bool {
    matches!(
        kind,
        FieldKind::Book
            | FieldKind::Composer
            | FieldKind::Discography
            | FieldKind::FileUrl
            | FieldKind::Group
            | FieldKind::History
            | FieldKind::Origin
            | FieldKind::Source
            | FieldKind::Reference
            | FieldKind::Transcription
            | FieldKind::Extension(_)
    )
}
