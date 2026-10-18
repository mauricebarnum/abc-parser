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

//! Line classification, block/tune resolution, complete-document assembly, and incremental block parsing.

use std::fmt;

use chumsky::IterParser;
use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::extra;
use chumsky::input::ValueInput;
use chumsky::prelude::any;
use chumsky::prelude::choice;
use chumsky::prelude::empty;
use chumsky::prelude::end;
use chumsky::prelude::just;
use chumsky::prelude::one_of;
use chumsky::recovery::via_parser;
use chumsky::span::SimpleSpan;
use chumsky::span::Span as ChumskySpan;

use super::DocumentParseWithDiagnostics;
use super::Extra;
use super::ParsedDocumentItem;
use super::ParsedLine;
use super::ParserState;
use super::ParserStateValue;
use super::fields::directive_parser;
use super::fields::directive_text_line_parser;
use super::fields::field_continuation;
use super::fields::field_parser;
use super::fields::is_body_forbidden_field;
use super::fields::is_tune_only_header_field;
use super::fields::recovering_field_parser;
use super::music::music_line_parser;
use super::music::strict_music_line_parser;
use super::primitives::comment_line;
use super::primitives::horizontal_space;
use super::primitives::line_character;
use super::primitives::newline;
use super::primitives::spanned_nonblank_line;
use super::primitives::trailing_comment;
use super::primitives::unsigned;
use crate::DirectiveKind;
use crate::Document;
use crate::DocumentItem;
use crate::ErrorKind;
use crate::FieldKind;
use crate::FieldValue;
use crate::FreeText;
use crate::Line;
use crate::LineBreak;
use crate::MusicElement;
use crate::ParseError;
use crate::ParseReport;
use crate::ParseWarning;
use crate::ParsedDocument;
use crate::ParserOptions;
use crate::RelatedSpan;
use crate::SourceText;
use crate::Spanned;
use crate::Tune;
use crate::TypesetText;
use crate::chumsky_error;
use crate::field_kind;

/// A blank-delimited document block parsed in its selected grammar mode.
pub(crate) enum ParsedBlock<S> {
    Tune {
        leading_comments: Vec<ParsedLine<S>>,
        tune: Spanned<Tune<S, SourceText<S>>, S>,
    },
    Text {
        items: Vec<Spanned<DocumentItem<S, SourceText<S>>, S>>,
    },
}

/// The first physical block, which alone may be a file header.
pub(crate) enum ParsedFirstBlock<S> {
    Header(Vec<ParsedLine<S>>),
    Content(ParsedBlock<S>),
}

/// One line or grouped typeset construct in a tune block.
pub(crate) enum ParsedTuneUnit<S> {
    Line(ParsedLine<S>),
    Typeset(Spanned<TypesetText<SourceText<S>>, S>),
}

/// A field-led block awaiting first-block header resolution.
pub(crate) struct ParsedTuneCandidate<S> {
    leading_comments: Vec<ParsedLine<S>>,
    units: Vec<ParsedTuneUnit<S>>,
}

/// Returns the first information-field span among resolved tune units.
fn first_field_span_in_units<S: Clone>(units: &[ParsedTuneUnit<S>]) -> Option<S> {
    units.iter().find_map(|unit| match unit {
        ParsedTuneUnit::Line(line) => match &line.value {
            Line::Field(_) => Some(line.span.clone()),
            _ => None,
        },
        ParsedTuneUnit::Typeset(_) => None,
    })
}

/// Returns the first information-field span among parsed physical lines.
fn first_field_span_in_lines<S: Clone>(lines: &[ParsedLine<S>]) -> Option<S> {
    lines.iter().find_map(|line| match &line.value {
        Line::Field(_) => Some(line.span.clone()),
        _ => None,
    })
}

/// One body line retained while parsing a grouped typeset block.
struct ParsedTypesetBodyLine<S> {
    text: Option<SourceText<S>>,
    span: S,
}

/// Classifies a non-blank tune line while preserving music parser failures.
pub(crate) fn tune_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        directive_parser()
            .then_ignore(trailing_comment())
            .map(Line::Directive),
        directive_text_line_parser(),
        comment_line(),
        field_continuation(),
        recovering_field_parser()
            .then_ignore(trailing_comment())
            .map(Line::Field),
        music_line_parser::<I>()
            .then_ignore(trailing_comment())
            .map(Line::Music),
    ))
}

/// Classifies a standalone line while preserving directive parser failures.
pub(crate) fn diagnostic_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let directive = directive_parser()
        .then_ignore(trailing_comment())
        .map(Line::Directive)
        .recover_with(via_parser(directive_text_line_parser()));
    choice((
        directive,
        comment_line(),
        field_continuation(),
        recovering_field_parser()
            .then_ignore(trailing_comment())
            .map(Line::Field),
        music_line_parser::<I>()
            .then_ignore(trailing_comment())
            .map(Line::Music),
    ))
}

/// Classifies a non-blank text line without applying the music grammar.
pub(crate) fn text_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        directive_parser()
            .then_ignore(trailing_comment())
            .map(Line::Directive),
        directive_text_line_parser(),
        comment_line(),
        field_continuation(),
        recovering_field_parser()
            .then_ignore(trailing_comment())
            .map(Line::Field),
        line_character()
            .repeated()
            .at_least(1)
            .ignored()
            .to(Line::Music(Vec::new())),
    ))
}

/// Recognizes the boundary following a nonblank block without consuming it.
pub(crate) fn block_end<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
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
pub(crate) fn typeset_block_parser<'src, I>()
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
pub(crate) fn inline_typeset_parser<'src, I>()
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
pub(crate) fn free_text_line_parser<'src, I>()
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
pub(crate) fn free_text_parser<'src, I>()
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
pub(crate) fn text_item_parser<'src, I>(
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
    let comment = spanned_nonblank_line::<I, _>(comment_line()).map(|line| {
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
pub(crate) fn text_block_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedBlock<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let comment = spanned_nonblank_line::<I, _>(comment_line());
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
            let state = &mut extra.state().0;
            // Previous-block gating: this block is text, so the signal resets
            // whether or not the music-like warning fires.
            let previous_field_led = state.last_field_led_span.take();
            if let Some(span) = possible_music {
                let (message, related) = match previous_field_led {
                    Some(header_span) => (
                        "block parses as music but has no leading information field; a preceding information field block may have been separated from this music by an extra blank line; treating it as free text"
                            .to_owned(),
                        vec![RelatedSpan {
                            span: header_span,
                            message: "preceding information field block".to_owned(),
                        }],
                    ),
                    None => (
                        "block parses as music but has no leading information field; treating it as free text"
                            .to_owned(),
                        Vec::new(),
                    ),
                };
                state.warnings.push(ParseWarning {
                    kind: ErrorKind::MissingReference,
                    message,
                    span,
                    related,
                });
            }
            ParsedBlock::Text {
                items: std::iter::once(first).chain(rest).flatten().collect(),
            }
        })
        // Prevent the repeated item grammar from propagating into document_parser.
        .boxed()
}

/// Parses an `H:` field and its deprecated implicit continuation lines.
pub(crate) fn deprecated_history_group_parser<'src, I>()
-> impl Parser<'src, I, Vec<ParsedTuneUnit<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let field_start = any()
        .filter(char::is_ascii_alphabetic)
        .then_ignore(just(':'))
        .rewind();
    let history = spanned_nonblank_line::<I, _>(
        just("H:")
            .rewind()
            .ignore_then(field_parser())
            .then_ignore(trailing_comment())
            .map(Line::Field),
    );
    let continuation = spanned_nonblank_line::<I, _>(
        field_start
            .not()
            .ignore_then(
                line_character()
                    .repeated()
                    .at_least(1)
                    .to_span()
                    .map(SourceText::Span),
            )
            .map(Line::DeprecatedHistoryContinuation),
    );

    history
        .then(
            newline()
                .ignore_then(continuation)
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map_with(|(history, continuations), extra| {
            let span = extra.span();
            extra.state().0.warnings.push(ParseWarning {
                kind: ErrorKind::DeprecatedSyntax,
                message: "deprecated implicit multiline H: history syntax is retained as raw text"
                    .to_owned(),
                span,
                related: Vec::new(),
            });
            std::iter::once(ParsedTuneUnit::Line(history))
                .chain(continuations.into_iter().map(ParsedTuneUnit::Line))
                .collect()
        })
}

/// Parses a field-led block into semantic tune units before header resolution.
pub(crate) fn tune_candidate_parser<'src, I>()
-> impl Parser<'src, I, ParsedTuneCandidate<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let comment = spanned_nonblank_line::<I, _>(comment_line());
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
    let ordinary_unit = tune_line.map(|line| vec![ParsedTuneUnit::Line(line)]);
    let unit = choice((
        deprecated_history_group_parser::<I>(),
        typeset_block_parser::<I>().map(|text| vec![ParsedTuneUnit::Typeset(text)]),
        inline_typeset_parser::<I>().map(|text| vec![ParsedTuneUnit::Typeset(text)]),
        ordinary_unit,
    ))
    .boxed();
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
                .ignore_then(unit.clone())
                .then(newline().ignore_then(unit).repeated().collect::<Vec<_>>()),
        )
        .map(|(leading_comments, (first, rest))| ParsedTuneCandidate {
            leading_comments,
            units: first
                .into_iter()
                .chain(rest.into_iter().flatten())
                .collect(),
        })
        // Tune candidates combine the largest line grammars; cap their type here.
        .boxed()
}

/// Returns whether a field-led first block contains only file-header material.
fn is_header_candidate<S>(candidate: &ParsedTuneCandidate<S>) -> bool {
    candidate.units.iter().all(|unit| match unit {
        ParsedTuneUnit::Line(line) => match &line.value {
            Line::Comment(_)
            | Line::FieldContinuation(_)
            | Line::DeprecatedHistoryContinuation(_) => true,
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
        state.0.warnings.push(ParseWarning {
            kind: ErrorKind::InvalidFieldOrder,
            message: "X: reference field should be the first information field in a tune"
                .to_owned(),
            span: span.clone(),
            related: Vec::new(),
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
        state.0.warnings.push(ParseWarning {
            kind: ErrorKind::InvalidFieldOrder,
            message: "K: key field should be the last information field in a tune header"
                .to_owned(),
            span: (*key_span).clone(),
            related: Vec::new(),
        });
    }
}

/// Emits strict, non-fatal guidance for constructs that §3 and §6.1.1 forbid
/// inside a tune body once music code has begun.
fn validate_tune_body_strictness<S>(units: &[ParsedTuneUnit<S>], state: &mut ParserState<S>)
where
    S: ChumskySpan + Clone,
    S::Offset: PartialEq,
{
    let mut music_started = false;
    let mut last_music_ended_with_continuation = false;
    let mut last_music_span: Option<S> = None;
    for unit in units {
        if let ParsedTuneUnit::Line(line) = unit {
            match &line.value {
                Line::Music(_) => {
                    music_started = true;
                    last_music_ended_with_continuation = music_ends_with_continuation(&line.value);
                    last_music_span = Some(line.span.clone());
                }
                Line::Field(field) if music_started => {
                    if is_body_forbidden_field(field.kind) {
                        state.0.warnings.push(ParseWarning {
                            kind: ErrorKind::HeaderFieldInBody,
                            message: format!(
                                "{}: field is not permitted in the tune body (§3)",
                                field.key
                            ),
                            span: line.span.clone(),
                            related: Vec::new(),
                        });
                    } else if field.kind == FieldKind::Instruction
                        && matches!(
                            &field.value,
                            FieldValue::Text(SourceText::Span(span))
                                if state.0.is_linebreak_instruction(span)
                        )
                    {
                        state.0.warnings.push(ParseWarning {
                            kind: ErrorKind::InstructionInBody,
                            message: "I:linebreak instruction is not allowed in the tune body"
                                .to_owned(),
                            span: line.span.clone(),
                            related: Vec::new(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    if let (true, Some(span)) = (last_music_ended_with_continuation, last_music_span) {
        state.0.warnings.push(ParseWarning {
            kind: ErrorKind::MusicLineContinuationBeforeEmpty,
            message:
                "music line ends with a \\ continuation and is followed by an empty line (§6.1.1)"
                    .to_owned(),
            span,
            related: Vec::new(),
        });
    }
}

/// Returns whether the music line ends with a `\` continuation.
fn music_ends_with_continuation<S>(line: &Line<S, SourceText<S>>) -> bool {
    match line {
        Line::Music(elements) => elements.last().is_some_and(|element| {
            matches!(element.value, MusicElement::LineBreak(LineBreak::Continue))
        }),
        _ => false,
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
    if let Some(span) = first_field_span_in_units(&candidate.units) {
        state.0.note_field_led_start(span);
    }
    if options.is_strict() {
        let fields = candidate.units.iter().filter_map(|unit| match unit {
            ParsedTuneUnit::Line(Spanned {
                value: Line::Field(field),
                span,
            }) => Some((field.kind, span)),
            _ => None,
        });
        let references = fields
            .clone()
            .filter(|(kind, _)| *kind == FieldKind::Reference)
            .collect::<Vec<_>>();
        if references.is_empty() {
            state.0.errors.push(ParseError {
                kind: ErrorKind::MissingReference,
                message: "tune is missing required X: reference field".to_owned(),
                span: first_span.clone(),
            });
        } else if references.len() > 1 {
            state.0.errors.push(ParseError {
                kind: ErrorKind::InvalidField,
                message: "tune contains more than one X: reference field".to_owned(),
                span: references[1].1.clone(),
            });
        }
        for (kind, name) in [(FieldKind::Title, "T: title"), (FieldKind::Key, "K: key")] {
            if !fields.clone().any(|(field_kind, _)| field_kind == kind) {
                state.0.errors.push(ParseError {
                    kind: ErrorKind::InvalidField,
                    message: format!("tune is missing required {name} field"),
                    span: first_span.clone(),
                });
            }
        }
        validate_tune_header_order(&candidate.units, state);
        validate_tune_body_strictness(&candidate.units, state);
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
        if let Some(span) = first_field_span_in_units(&candidate.units) {
            state.0.note_field_led_start(span);
        }
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
pub(crate) fn tune_block_parser<'src, I>(
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
pub(crate) fn first_tune_or_header_parser<'src, I>(
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
pub(crate) fn initial_header_parser<'src, I>()
-> impl Parser<'src, I, Vec<ParsedLine<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let comment_line = comment_line::<I>();
    let directive_value =
        directive_parser().filter(|directive| directive.kind == DirectiveKind::Other);
    let field_start = any()
        .filter(|key: &char| {
            key.is_ascii_alphabetic() && !is_tune_only_header_field(field_kind(*key))
        })
        .then_ignore(just(':'))
        .rewind();
    let shape_line = choice((
        comment_line.clone().ignored(),
        directive_value.clone().ignored(),
        field_start.ignore_then(line_character().repeated().ignored()),
    ));
    let shape = shape_line
        .clone()
        .then(newline().ignore_then(shape_line).repeated())
        .then_ignore(block_end::<I>())
        .rewind();
    let line = spanned_nonblank_line::<I, _>(choice((
        comment_line,
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
pub(crate) fn block_parser<'src, I>(
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

/// Selects strict interpretation when the first line declares ABC 2.1 or newer.
pub(crate) fn strict_version_marker<'src, I>() -> impl Parser<'src, I, (), Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just("%abc-")
        .ignore_then(unsigned().then(just('.').ignore_then(unsigned()).or_not()))
        .filter(|(major, minor)| *major > 2 || (*major == 2 && minor.unwrap_or(0) >= 1))
        .then_ignore(line_character().repeated())
        .then_ignore(newline().rewind().or(end()))
        .rewind()
        .ignored()
}

/// Builds the document body after BOM and version interpretation.
pub(crate) fn document_body_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedDocument<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let blocks = blocks_combinator(options, true);
    blocks.map(|seq| {
        let (first, rest) = seq
            .into_iter()
            .next()
            .map_or((None, Vec::new()), |entry| (Some(entry.0), entry.1));
        assemble_document(first, rest)
    })
}

/// Parses zero or more blank-line-separated blocks from `input`.
///
/// `at_document_start` enables the document prologue (optional BOM, optional
/// strict version marker) and the first-block header-ambiguity resolution;
/// when `false` the slice is parsed as a contiguous region with no header
/// resolution. The output pairs the resolved first block (or `Header`)
/// with the sequence of subsequent blocks.
///
/// The two callers — [`document_body_parser`] for the full-document path
/// and [`parse_blocks`] for the incremental path — share this grammar so
/// the two paths cannot diverge.
#[allow(clippy::type_complexity)]
pub(crate) fn blocks_combinator<'src, I>(
    options: ParserOptions,
    at_document_start: bool,
) -> impl Parser<'src, I, Vec<(ParsedFirstBlock<I::Span>, Vec<ParsedBlock<I::Span>>)>, Extra<'src, I>>
+ Clone
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
    let first = if at_document_start {
        choice((
            first_tune_or_header_parser::<I>(options),
            initial_header_parser::<I>().map_with(|lines, extra| {
                if let Some(span) = first_field_span_in_lines(&lines) {
                    extra.state().0.note_field_led_start(span);
                }
                ParsedFirstBlock::Header(lines)
            }),
            text_block_parser::<I>(options).map(ParsedFirstBlock::Content),
        ))
        .boxed()
    } else {
        block.clone().map(ParsedFirstBlock::Content).boxed()
    };
    let pair = first.or_not().then(
        block_separator
            .clone()
            .ignore_then(block)
            .repeated()
            .collect::<Vec<_>>(),
    );
    let sequence = pair
        .map(|(first, rest)| match first {
            Some(first) => vec![(first, rest)],
            None => Vec::new(),
        })
        .boxed();
    if at_document_start {
        just('\u{feff}')
            .or_not()
            .ignored()
            .ignore_then(blank_line.repeated())
            .ignore_then(sequence)
            .then_ignore(block_separator.or_not())
            .then_ignore(newline().or_not())
            .then_ignore(horizontal_space())
            .then_ignore(end())
            .boxed()
    } else {
        sequence
            .then_ignore(block_separator.or_not())
            .then_ignore(newline().or_not())
            .then_ignore(end())
            .boxed()
    }
}

/// Builds a complete-document parser with version-selected strictness.
pub(crate) fn document_parser<'src, I>(
    options: ParserOptions,
) -> impl Parser<'src, I, ParsedDocument<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    just('\u{feff}')
        .or_not()
        .ignored()
        .ignore_then(choice((
            strict_version_marker().ignore_then(document_body_parser(options.strict(true))),
            document_body_parser(options),
        )))
        // Cap the public parse entry point's monomorphized type and codegen cost.
        .boxed()
}

/// Parses a document while retaining typed recovering diagnostics.
pub(crate) fn parse_document<'src, I>(
    input: I,
    options: ParserOptions,
) -> DocumentParseWithDiagnostics<'src, I>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord,
{
    let mut state = extra::SimpleState(ParserStateValue {
        errors: Vec::new(),
        warnings: Vec::new(),
        last_field_led_span: None,
        body_linebreak_instruction_spans: Vec::new(),
    });
    let result = document_parser(options).parse_with_state(input, &mut state);
    (result, (state.0.errors, state.0.warnings))
}

/// One blank-line-delimited block parsed in isolation.
///
/// This is the public, slice-relative output of [`parse_blocks`]. A block
/// is either a tune (one or more header lines followed by an optional body)
/// or a text-mode block containing free text, typeset text, comments, or
/// stylesheet directives. Leading comments of a tune become
/// [`DocumentItem::Comment`] entries in `items`.
///
/// The spans and [`SourceText`] offsets in `items` and `header` are
/// relative to the slice passed to `parse_blocks`; callers re-base them to
/// the document-absolute coordinate space when combining results.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::type_complexity)]
pub struct Block {
    /// Physical-line span of the block content, slice-relative, excluding
    /// the surrounding blank-line separators.
    pub span: SimpleSpan<usize>,
    /// Document items produced by the block, in source order.
    pub items: Vec<
        Spanned<DocumentItem<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
    >,
    /// File-header lines when this block resolved as the file header.
    pub header:
        Vec<Spanned<Line<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>>,
    /// Span of the first information field when the block is field-led
    /// (`Some` exactly for tune and file-header blocks).
    pub first_field: Option<SimpleSpan<usize>>,
}

/// Context needed to parse blocks in isolation with full-parse fidelity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlocksContext {
    /// The slice begins the document; enables the BOM/version-marker
    /// prologue and file-header resolution of the first block.
    pub at_document_start: bool,
    /// First-field span of the immediately preceding field-led block,
    /// seeding the `MissingReference` extra-blank-line hint.
    pub previous_field_led: Option<SimpleSpan<usize>>,
}

/// Parses one or more blank-line-separated blocks from `input`.
///
/// Diagnostics and spans are relative to `input` (the caller re-bases them
/// to the document-absolute coordinate space). When
/// [`BlocksContext::at_document_start`] is `true`, the parser reproduces
/// the document prologue (optional BOM, optional strict version marker
/// that forces strict interpretation, leading blank lines) and resolves
/// the first block as a file header or tune; when `false`, the slice is
/// treated as a region of the document and the first block is parsed
/// without header resolution.
pub fn parse_blocks(
    input: &str,
    options: ParserOptions,
    context: BlocksContext,
) -> ParseReport<Vec<Block>, SimpleSpan<usize>> {
    let options = if context.at_document_start {
        prologue_options(input, options)
    } else {
        options
    };
    let mut state = extra::SimpleState(ParserStateValue {
        errors: Vec::new(),
        warnings: Vec::new(),
        last_field_led_span: context.previous_field_led,
        body_linebreak_instruction_spans: Vec::new(),
    });
    let parser = blocks_combinator(options, context.at_document_start);
    let result = parser.parse_with_state(input, &mut state);
    let (output, faults) = result.into_output_errors();
    let mut errors: Vec<ParseError<SimpleSpan<usize>>> = faults.iter().map(chumsky_error).collect();
    errors.extend(state.0.errors);
    errors.sort_by_key(|error| (error.span.start(), error.span.end()));
    let mut warnings = state.0.warnings;
    warnings.sort_by_key(|warning| (warning.span.start(), warning.span.end()));
    ParseReport {
        output: output.map(rebase_blocks),
        errors,
        warnings,
    }
}

/// Returns the strict-marker decision for the first line of a document.
///
/// Returns `Some(true)` if `line` is a `%abc-` version marker whose version
/// is 2.1 or later (in which case the marker is consumed by the document
/// prologue and strict interpretation is selected). Returns `None` for
/// any other line, including `%abc` without the dash, `%abc-2.0`,
/// `%abc-2` (no minor), `%abc-2x` (junk after the major digit), and any
/// other text. Used by incremental callers that need to know whether a
/// line-1 edit invalidates a previously cached strictness choice.
pub fn version_marker(line: &str) -> Option<bool> {
    let trimmed = line.strip_suffix('\r').unwrap_or(line);
    let rest = trimmed.strip_prefix("%abc-")?;
    let mut chars = rest.chars();
    let major = chars.next()?.to_digit(10)?;
    let (minor, has_minor) = if chars.next() == Some('.') {
        (chars.next()?.to_digit(10)?, true)
    } else {
        (0, false)
    };
    let strict = major > 2 || (major == 2 && has_minor && minor >= 1);
    strict.then_some(true)
}

/// Inspects the optional BOM and strict version marker to select parser options.
fn prologue_options(input: &str, options: ParserOptions) -> ParserOptions {
    let slice = input.strip_prefix('\u{feff}').unwrap_or(input);
    let first_line = slice.split(['\r', '\n']).next().unwrap_or("");
    if version_marker(first_line) == Some(true) {
        options.strict(true)
    } else {
        options
    }
}

/// Converts the internal per-block shape into the public [`Block`] shape.
#[allow(clippy::type_complexity)]
fn rebase_blocks(
    blocks: Vec<(
        ParsedFirstBlock<SimpleSpan<usize>>,
        Vec<ParsedBlock<SimpleSpan<usize>>>,
    )>,
) -> Vec<Block> {
    blocks
        .into_iter()
        .flat_map(|(first, rest)| {
            let mut out = Vec::with_capacity(1 + rest.len());
            out.push(block_from_first(first, &rest));
            out.extend(rest.into_iter().map(block_from_subsequent));
            out
        })
        .collect()
}

fn block_from_first(
    first: ParsedFirstBlock<SimpleSpan<usize>>,
    rest: &[ParsedBlock<SimpleSpan<usize>>],
) -> Block {
    match first {
        ParsedFirstBlock::Header(lines) => {
            let first_field = first_field_span_in_lines(&lines);
            let span_start = lines.first().map_or_else(
                || match rest.first() {
                    Some(ParsedBlock::Tune { tune, .. }) => tune.span.start,
                    Some(ParsedBlock::Text { items }) => items.first().map_or(0, |i| i.span.start),
                    None => 0,
                },
                |l| l.span.start,
            );
            let span_end = lines.last().map_or(span_start, |l| l.span.end);
            Block {
                span: SimpleSpan {
                    start: span_start,
                    end: span_end,
                    context: (),
                },
                items: Vec::new(),
                header: lines,
                first_field,
            }
        }
        ParsedFirstBlock::Content(ParsedBlock::Tune {
            leading_comments,
            tune,
        }) => tune_block(leading_comments, tune),
        ParsedFirstBlock::Content(ParsedBlock::Text { items }) => text_block(items),
    }
}

fn block_from_subsequent(block: ParsedBlock<SimpleSpan<usize>>) -> Block {
    match block {
        ParsedBlock::Tune {
            leading_comments,
            tune,
        } => tune_block(leading_comments, tune),
        ParsedBlock::Text { items } => text_block(items),
    }
}

#[allow(clippy::type_complexity)]
fn tune_block(
    leading_comments: Vec<
        Spanned<Line<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
    >,
    tune: Spanned<Tune<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
) -> Block {
    let span_start = leading_comments
        .first()
        .map_or(tune.span.start, |l| l.span.start);
    let span_end = tune.span.end;
    let first_field = first_field_span_in_lines(&tune.value.lines);
    let mut items: Vec<
        Spanned<DocumentItem<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
    > = comment_items(leading_comments).collect();
    items.push(Spanned {
        value: DocumentItem::Tune(tune.value),
        span: tune.span,
    });
    Block {
        span: SimpleSpan {
            start: span_start,
            end: span_end,
            context: (),
        },
        items,
        header: Vec::new(),
        first_field,
    }
}

#[allow(clippy::type_complexity)]
fn text_block(
    items: Vec<
        Spanned<DocumentItem<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
    >,
) -> Block {
    let span_start = items.first().map_or(0, |i| i.span.start);
    let span_end = items.last().map_or(span_start, |i| i.span.end);
    Block {
        span: SimpleSpan {
            start: span_start,
            end: span_end,
            context: (),
        },
        first_field: None,
        header: Vec::new(),
        items,
    }
}
