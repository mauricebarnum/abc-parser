// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Parsed diagnostics and editor-facing source analysis.

use std::ops::Range;

use abc_parser::Directive;
use abc_parser::DocumentItem;
use abc_parser::EmitOptions;
use abc_parser::ErrorKind;
use abc_parser::Field;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::Line;
use abc_parser::MusicElement;
use abc_parser::NoteLengthStyle as AbcNoteLengthStyle;
use abc_parser::SimpleSpan;
use abc_parser::SourceText;
use abc_parser::Spanned;
use abc_parser::ToAbc;
use abc_parser::Tune;
use abc_parser::TypesetText;
use abc_parser::parse_note_length;
use tower_lsp_server::ls_types::CompletionItem;
use tower_lsp_server::ls_types::CompletionItemKind;
use tower_lsp_server::ls_types::CompletionTextEdit;
use tower_lsp_server::ls_types::DocumentSymbol;
use tower_lsp_server::ls_types::FoldingRange;
use tower_lsp_server::ls_types::FoldingRangeKind;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::Range as LspRange;
use tower_lsp_server::ls_types::SelectionRange;
use tower_lsp_server::ls_types::SemanticToken;
use tower_lsp_server::ls_types::SemanticTokens;
use tower_lsp_server::ls_types::SymbolKind;
use tower_lsp_server::ls_types::TextEdit;

use crate::config::NoteLengthStyle;
use crate::document::BlockRecord;
use crate::document::rebase;
use crate::position::LineIndex;

const FIELD_COMPLETIONS: &[(char, &str)] = &[
    ('X', "Tune reference number"),
    ('T', "Tune title"),
    ('C', "Composer"),
    ('M', "Meter"),
    ('L', "Unit note length"),
    ('Q', "Tempo"),
    ('K', "Key signature"),
    ('V', "Voice"),
    ('P', "Part sequence"),
    ('R', "Rhythm"),
    ('w', "Aligned lyrics"),
    ('W', "Unaligned words"),
];

pub const fn error_kind_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnclosedDelimiter => "unclosed-delimiter",
        ErrorKind::InvalidField => "invalid-field",
        ErrorKind::InvalidDirective => "invalid-directive",
        ErrorKind::InvalidMusic => "invalid-music",
        ErrorKind::MissingReference => "missing-reference",
        ErrorKind::InvalidFieldOrder => "invalid-field-order",
        ErrorKind::UnrecognizedField => "unrecognized-field",
        ErrorKind::HeaderFieldInBody => "header-field-in-body",
        ErrorKind::InstructionInBody => "instruction-in-body",
        ErrorKind::MusicLineContinuationBeforeEmpty => "music-continuation-before-empty",
        _ => "parser-diagnostic",
    }
}

type ParsedLine = Line<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>;

fn find_line_at_offset(
    blocks: &[BlockRecord],
    offset: usize,
) -> Option<(&ParsedLine, Range<usize>)> {
    let block = blocks
        .iter()
        .find(|b| b.lines.start <= offset && offset <= b.lines.end)?;
    for spanned in &block.parsed.header {
        let span = rebase(spanned.span, block.base);
        if span.start <= offset && offset <= span.end {
            return Some((&spanned.value, span));
        }
    }
    for item in &block.parsed.items {
        if let DocumentItem::Tune(tune) = &item.value {
            for spanned in &tune.lines {
                let span = rebase(spanned.span, block.base);
                if span.start <= offset && offset <= span.end {
                    return Some((&spanned.value, span));
                }
            }
        }
    }
    None
}

pub fn hover(
    blocks: &[BlockRecord],
    index: &LineIndex,
    offset: usize,
) -> Option<(Range<usize>, String)> {
    let (line_value, line) = find_line_at_offset(blocks, offset)?;
    match line_value {
        Line::Field(field) => {
            let key = field.key;
            let explanation = FIELD_COMPLETIONS
                .iter()
                .find_map(|(candidate, description)| (*candidate == key).then_some(*description))
                .unwrap_or("ABC information field");
            Some((
                line.start..line.start + key.len_utf8(),
                format!("**{key}:** — {explanation}"),
            ))
        }
        Line::Music(_) => {
            let text = &index.source()[line.clone()];
            let relative = offset.saturating_sub(line.start).min(text.len());
            let (start, character) = text
                .char_indices()
                .take_while(|(position, _)| *position <= relative)
                .last()?;
            let absolute = line.start + start;
            match character {
                'A'..='G' | 'a'..='g' => {
                    let suffix = duration_suffix(&text[start + character.len_utf8()..]);
                    let detail = suffix
                        .and_then(|value| parse_note_length(value).ok())
                        .map_or_else(
                            || "unit note length".to_owned(),
                            |length| {
                                let equivalent = if length.numerator == 1
                                    && length.denominator.is_power_of_two()
                                {
                                    let shorthand = "/".repeat(length.denominator.ilog2() as usize);
                                    let explicit = length.denominator;
                                    format!("; shorthand `{shorthand}` = explicit `/{explicit}`")
                                } else {
                                    String::new()
                                };
                                format!(
                                    "duration multiplier {}/{}{equivalent}",
                                    length.numerator, length.denominator
                                )
                            },
                        );
                    Some((
                        absolute..absolute + character.len_utf8(),
                        format!("ABC note `{character}` — {detail}"),
                    ))
                }
                'z' => Some((absolute..absolute + 1, "Visible rest".to_owned())),
                'x' => Some((absolute..absolute + 1, "Invisible spacer rest".to_owned())),
                '|' | ':' => Some((absolute..absolute + 1, "Bar or repeat delimiter".to_owned())),
                _ => None,
            }
        }
        Line::Comment(_) | _ => None,
    }
}

pub fn completions(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    offset: usize,
) -> Vec<CompletionItem> {
    let Some(line) = index.line_bounds(offset) else {
        return Vec::new();
    };
    let prefix = &index.source()[line.start..offset.min(line.end)];
    if prefix.starts_with("%%") {
        let Some(range) = index.lsp_range(line.start..offset.min(line.end), encoding) else {
            return Vec::new();
        };
        return ["text", "center", "begintext", "endtext"]
            .into_iter()
            .map(|name| CompletionItem {
                label: format!("%%{name}"),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("ABC stylesheet directive".to_owned()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                    range,
                    format!("%%{name}"),
                ))),
                ..CompletionItem::default()
            })
            .collect();
    }
    if prefix.is_empty() || (prefix.len() == 1 && prefix.chars().all(char::is_alphabetic)) {
        let Some(range) = index.lsp_range(line.start..offset.min(line.end), encoding) else {
            return Vec::new();
        };
        return FIELD_COMPLETIONS
            .iter()
            .map(|(key, description)| CompletionItem {
                label: format!("{key}:"),
                kind: Some(CompletionItemKind::FIELD),
                detail: Some((*description).to_owned()),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                    range,
                    format!("{key}:"),
                ))),
                ..CompletionItem::default()
            })
            .collect();
    }
    if prefix.starts_with("M:") {
        return simple_completions(&["4/4", "3/4", "6/8", "C", "C|", "none"]);
    }
    if prefix.starts_with("K:") {
        return simple_completions(&[
            "C", "G", "D", "A", "F", "Bb", "Eb", "Am", "Em", "Bm", "F#m", "Dm", "Gm", "Cm",
        ]);
    }
    Vec::new()
}

fn simple_completions(values: &[&str]) -> Vec<CompletionItem> {
    values
        .iter()
        .map(|value| CompletionItem {
            label: (*value).to_owned(),
            kind: Some(CompletionItemKind::VALUE),
            ..CompletionItem::default()
        })
        .collect()
}

pub fn document_symbols(
    blocks: &[BlockRecord],
    index: &LineIndex,
    encoding: &PositionEncodingKind,
) -> Vec<DocumentSymbol> {
    let source = index.source();
    let mut symbols = Vec::new();
    for record in blocks {
        for item in &record.parsed.items {
            if let DocumentItem::Tune(tune) = &item.value
                && let Some(symbol) =
                    tune_symbol(tune, item.span, record.base, source, index, encoding)
            {
                symbols.push(symbol);
            }
        }
    }
    symbols
}

fn tune_symbol(
    tune: &Tune<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>,
    tune_span: SimpleSpan<usize>,
    base: isize,
    source: &str,
    index: &LineIndex,
    encoding: &PositionEncodingKind,
) -> Option<DocumentSymbol> {
    let range = index.lsp_range(rebase(tune_span, base), encoding)?;
    let reference_field = tune.lines.iter().find_map(|line| match &line.value {
        Line::Field(field) if field.kind == FieldKind::Reference => {
            let text = match &field.value {
                FieldValue::Reference(number) => number.to_string(),
                FieldValue::Unparsed(raw) | FieldValue::Text(raw) => {
                    resolve_text(raw, base, source)
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_owned()
                }
                _ => String::new(),
            };
            Some((rebase(line.span, base), text))
        }
        _ => None,
    });
    let title_field = tune.lines.iter().find_map(|line| match &line.value {
        Line::Field(field) if field.kind == FieldKind::Title => {
            let text = match &field.value {
                FieldValue::Text(raw) | FieldValue::Unparsed(raw) => {
                    resolve_text(raw, base, source).map(str::trim)
                }
                _ => None,
            }?;
            (!text.is_empty()).then(|| (rebase(line.span, base), text.to_owned()))
        }
        _ => None,
    });
    let name = if let Some((_, title)) = &title_field {
        title.clone()
    } else if let Some((_, reference)) = &reference_field
        && !reference.is_empty()
    {
        format!("Tune {reference}")
    } else {
        "Tune".to_owned()
    };
    let detail = reference_field
        .as_ref()
        .map(|(_, reference)| format!("X:{reference}"));
    let selection_span = reference_field
        .map(|(span, _)| span)
        .or_else(|| title_field.map(|(span, _)| span))
        .or_else(|| tune.lines.first().map(|line| rebase(line.span, base)));
    let selection_range = match selection_span {
        Some(span) => index.lsp_range(span, encoding)?,
        None => range,
    };
    #[allow(deprecated)]
    Some(DocumentSymbol {
        name,
        detail,
        kind: SymbolKind::MODULE,
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: voice_symbols(tune, base, source, index, encoding),
    })
}

fn voice_symbols(
    tune: &Tune<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>,
    base: isize,
    source: &str,
    index: &LineIndex,
    encoding: &PositionEncodingKind,
) -> Option<Vec<DocumentSymbol>> {
    let voices = tune
        .lines
        .iter()
        .filter_map(|line| {
            let Line::Field(field) = &line.value else {
                return None;
            };
            if field.kind != FieldKind::Voice {
                return None;
            }
            let line_range = rebase(line.span, base);
            let range = index.lsp_range(line_range, encoding)?;
            let (name, selection_range) = match &field.value {
                FieldValue::Voice(voice) => match &voice.id {
                    SourceText::Span(id_span) => {
                        let id_range = rebase(*id_span, base);
                        let id = source.get(id_range.clone())?.to_owned();
                        let sel = index.lsp_range(id_range, encoding)?;
                        (id, sel)
                    }
                    SourceText::Synthesized(id) => (id.clone(), range),
                },
                FieldValue::Unparsed(SourceText::Span(val_span)) => {
                    let val_range = rebase(*val_span, base);
                    let val_text = source.get(val_range.clone())?;
                    let trimmed = val_text.trim_start();
                    let leading = val_text.len() - trimmed.len();
                    let id = trimmed.split_whitespace().next()?;
                    let id_start = val_range.start + leading;
                    let id_range = id_start..id_start + id.len();
                    let sel = index.lsp_range(id_range, encoding)?;
                    (id.to_owned(), sel)
                }
                _ => return None,
            };
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name,
                detail: Some("Voice".to_owned()),
                kind: SymbolKind::VARIABLE,
                tags: None,
                deprecated: None,
                range,
                selection_range,
                children: None,
            })
        })
        .collect::<Vec<_>>();
    (!voices.is_empty()).then_some(voices)
}

fn resolve_text<'a>(
    text: &'a SourceText<SimpleSpan<usize>>,
    base: isize,
    source: &'a str,
) -> Option<&'a str> {
    match text {
        SourceText::Span(span) => source.get(rebase(*span, base)),
        SourceText::Synthesized(value) => Some(value.as_str()),
    }
}

pub fn folding_ranges(index: &LineIndex) -> Vec<FoldingRange> {
    let lines = index.line_iter().collect::<Vec<_>>();
    let mut tune_start = None;
    let mut text_start = None;
    let mut ranges = Vec::new();
    for (line_number, (_, line)) in lines.iter().enumerate() {
        if line.starts_with("%%begintext") {
            text_start = Some(line_number);
        } else if line.starts_with("%%endtext")
            && let Some(start) = text_start.take()
        {
            push_fold(&mut ranges, start, line_number, "text");
        }
        if line.starts_with("X:") {
            if let Some(start) = tune_start.replace(line_number) {
                push_fold(&mut ranges, start, line_number.saturating_sub(1), "tune");
            }
        } else if line.trim().is_empty()
            && let Some(start) = tune_start.take()
        {
            push_fold(&mut ranges, start, line_number.saturating_sub(1), "tune");
        }
    }
    let last_line = lines.len().saturating_sub(1);
    if let Some(start) = tune_start {
        push_fold(&mut ranges, start, last_line, "tune");
    }
    if let Some(start) = text_start {
        push_fold(&mut ranges, start, last_line, "text");
    }
    ranges.sort_by_key(|range| (range.start_line, range.end_line));
    ranges
}

fn push_fold(ranges: &mut Vec<FoldingRange>, start: usize, end: usize, label: &str) {
    if end <= start {
        return;
    }
    ranges.push(FoldingRange {
        start_line: u32::try_from(start).unwrap_or(u32::MAX),
        start_character: None,
        end_line: u32::try_from(end).unwrap_or(u32::MAX),
        end_character: None,
        kind: Some(FoldingRangeKind::Region),
        collapsed_text: Some(label.to_owned()),
    });
}

pub fn selection_range(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    offset: usize,
) -> Option<SelectionRange> {
    let line = index.line_bounds(offset)?;
    let token = token_range_at(index.source(), offset).unwrap_or_else(|| line.clone());
    let document = SelectionRange {
        range: index.whole_range(encoding),
        parent: None,
    };
    let line_selection = SelectionRange {
        range: index.lsp_range(line, encoding)?,
        parent: Some(Box::new(document)),
    };
    Some(SelectionRange {
        range: index.lsp_range(token, encoding)?,
        parent: Some(Box::new(line_selection)),
    })
}

fn token_range_at(source: &str, offset: usize) -> Option<Range<usize>> {
    let is_token = |value: char| value.is_alphanumeric() || "_'/^=!+-.".contains(value);
    let mut start = offset.min(source.len());
    while start > 0 {
        let value = source[..start].chars().next_back()?;
        if !is_token(value) {
            break;
        }
        start -= value.len_utf8();
    }
    let mut end = offset.min(source.len());
    while end < source.len() {
        let value = source[end..].chars().next()?;
        if !is_token(value) {
            break;
        }
        end += value.len_utf8();
    }
    (start < end).then_some(start..end)
}

const TOKEN_COMMENT: u32 = 0;
const TOKEN_KEYWORD: u32 = 1;
const TOKEN_NUMBER: u32 = 2;
const TOKEN_STRING: u32 = 3;
const TOKEN_ENUM_MEMBER: u32 = 4;
const TOKEN_OPERATOR: u32 = 5;
const TOKEN_DECORATOR: u32 = 6;
const TOKEN_VARIABLE: u32 = 7;

const MODIFIER_DECLARATION: u32 = 1 << 0;
const MODIFIER_DEPRECATED: u32 = 1 << 1;

pub fn semantic_tokens(
    blocks: &[BlockRecord],
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    filter: Option<LspRange>,
) -> SemanticTokens {
    let filter = filter.and_then(|range| index.byte_range(range, encoding));
    let mut raw = ast_tokens(blocks, index.source());
    if let Some(filter) = filter {
        raw.retain(|token| token.range.start < filter.end && token.range.end > filter.start);
    }
    raw.sort_by_key(|token| token.range.start);
    let mut previous_line = 0;
    let mut previous_start = 0;
    let data = raw
        .into_iter()
        .filter_map(|token| {
            let start = index.position(token.range.start, encoding)?;
            let end = index.position(token.range.end, encoding)?;
            if start.line != end.line {
                return None;
            }
            let delta_line = start.line - previous_line;
            let delta_start = if delta_line == 0 {
                start.character - previous_start
            } else {
                start.character
            };
            previous_line = start.line;
            previous_start = start.character;
            Some(SemanticToken {
                delta_line,
                delta_start,
                length: end.character - start.character,
                token_type: token.kind,
                token_modifiers_bitset: token.modifiers,
            })
        })
        .collect();
    SemanticTokens {
        result_id: None,
        data,
    }
}

struct RawToken {
    range: Range<usize>,
    kind: u32,
    modifiers: u32,
}

fn push_token(tokens: &mut Vec<RawToken>, range: Range<usize>, kind: u32, modifiers: u32) {
    if range.start < range.end {
        tokens.push(RawToken {
            range,
            kind,
            modifiers,
        });
    }
}

fn ast_tokens(blocks: &[BlockRecord], source: &str) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    for record in blocks {
        for line in &record.parsed.header {
            collect_line_tokens(line, record.base, source, &mut tokens);
        }
        for item in &record.parsed.items {
            let item_range = rebase(item.span, record.base);
            match &item.value {
                DocumentItem::Tune(tune) => {
                    for line in &tune.lines {
                        collect_line_tokens(line, record.base, source, &mut tokens);
                    }
                }
                DocumentItem::Comment(_) => {
                    push_token(&mut tokens, item_range, TOKEN_COMMENT, 0);
                }
                DocumentItem::Directive(directive) => {
                    collect_directive_tokens(
                        directive,
                        item_range,
                        record.base,
                        source,
                        &mut tokens,
                    );
                }
                DocumentItem::TypesetText(typeset) => {
                    collect_typeset_tokens(typeset, item_range, source, &mut tokens);
                }
                DocumentItem::FreeText(_) | _ => {}
            }
        }
    }
    tokens
}

fn collect_line_tokens(
    line: &Spanned<ParsedLine, SimpleSpan<usize>>,
    base: isize,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    let line_range = rebase(line.span, base);
    match &line.value {
        Line::Comment(_) => {
            push_token(tokens, line_range, TOKEN_COMMENT, 0);
        }
        Line::Directive(directive) => {
            collect_directive_tokens(directive, line_range, base, source, tokens);
        }
        Line::DirectiveText(body) => {
            collect_leading_directive_keyword(line_range.clone(), source, tokens);
            if let SourceText::Span(body_span) = body {
                let body_end = rebase(*body_span, base).end;
                collect_trailing_comment(body_end..line_range.end, source, tokens);
            }
        }
        Line::TypesetText(typeset) => {
            collect_typeset_tokens(typeset, line_range, source, tokens);
        }
        Line::Field(field) => {
            collect_field_line_tokens(field, line_range, base, source, tokens);
        }
        Line::FieldContinuation(body) => {
            if let Some(slice) = source.get(line_range.clone()) {
                let trimmed = slice.trim_start_matches([' ', '\t']);
                let leading = slice.len() - trimmed.len();
                if trimmed.starts_with("+:") {
                    let start = line_range.start + leading;
                    push_token(tokens, start..start + 1, TOKEN_KEYWORD, 0);
                    push_token(tokens, start + 1..start + 2, TOKEN_OPERATOR, 0);
                }
            }
            if let SourceText::Span(body_span) = body {
                let body_end = rebase(*body_span, base).end;
                collect_trailing_comment(body_end..line_range.end, source, tokens);
            }
        }
        Line::Music(elements) => {
            collect_music_line_tokens(elements, line_range, base, source, tokens);
        }
        Line::Blank | Line::DeprecatedHistoryContinuation(_) | _ => {}
    }
}

fn collect_directive_tokens(
    directive: &Directive<SourceText<SimpleSpan<usize>>>,
    line_range: Range<usize>,
    base: isize,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    if let SourceText::Span(name_span) = directive.name {
        let name_end = rebase(name_span, base).end;
        if let Some(slice) = source.get(line_range.start..name_end) {
            let trimmed = slice.trim_start_matches([' ', '\t']);
            let leading = slice.len() - trimmed.len();
            push_token(
                tokens,
                line_range.start + leading..name_end,
                TOKEN_KEYWORD,
                0,
            );
        }
    } else {
        collect_leading_directive_keyword(line_range.clone(), source, tokens);
    }
    if let SourceText::Span(body_span) = directive.body {
        let body_end = rebase(body_span, base).end;
        collect_trailing_comment(body_end..line_range.end, source, tokens);
    }
}

fn collect_leading_directive_keyword(
    range: Range<usize>,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    let trimmed = slice.trim_start_matches([' ', '\t']);
    if !trimmed.starts_with("%%") {
        return;
    }
    let leading = slice.len() - trimmed.len();
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == '%')
        .filter(|&idx| idx > 2)
        .unwrap_or_else(|| trimmed.find(char::is_whitespace).unwrap_or(trimmed.len()));
    let start = range.start + leading;
    push_token(tokens, start..start + end, TOKEN_KEYWORD, 0);
}

fn collect_typeset_tokens(
    typeset: &TypesetText<SourceText<SimpleSpan<usize>>>,
    range: Range<usize>,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    match typeset {
        TypesetText::Text(_) | TypesetText::Centered(_) => {
            collect_leading_directive_keyword(range, source, tokens);
        }
        TypesetText::Block(_) => {
            let Some(slice) = source.get(range.clone()) else {
                return;
            };
            let first_len = slice.find(['\r', '\n']).unwrap_or(slice.len());
            collect_leading_directive_keyword(range.start..range.start + first_len, source, tokens);
            if let Some(last_nl) = slice.rfind(['\r', '\n']) {
                let last_start = range.start + last_nl + 1;
                if let Some(last_line) = source.get(last_start..range.end)
                    && last_line
                        .trim_start_matches([' ', '\t'])
                        .starts_with("%%endtext")
                {
                    collect_leading_directive_keyword(last_start..range.end, source, tokens);
                }
            }
        }
        _ => {}
    }
}

fn collect_trailing_comment(range: Range<usize>, source: &str, tokens: &mut Vec<RawToken>) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    if let Some(rel) = slice.find('%') {
        push_token(tokens, range.start + rel..range.end, TOKEN_COMMENT, 0);
    }
}

fn find_unescaped_comment(slice: &str) -> Option<usize> {
    let mut escaped = false;
    let mut quoted = false;
    for (idx, ch) in slice.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '%' if !quoted => return Some(idx),
            _ => {}
        }
    }
    None
}

fn collect_field_line_tokens(
    field: &Field<SourceText<SimpleSpan<usize>>>,
    line_range: Range<usize>,
    base: isize,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    let Some(slice) = source.get(line_range.clone()) else {
        return;
    };
    let trimmed = slice.trim_start_matches([' ', '\t']);
    let leading = slice.len() - trimmed.len();
    let key_start = line_range.start + leading;
    let key_end = key_start + field.key.len_utf8();
    if key_end > line_range.end {
        return;
    }
    let modifiers = u32::from(matches!(
        field.kind,
        FieldKind::Area | FieldKind::ElementSpacing
    )) * MODIFIER_DEPRECATED;
    push_token(tokens, key_start..key_end, TOKEN_KEYWORD, modifiers);
    let value_start = if source.get(key_end..key_end + 1) == Some(":") {
        push_token(tokens, key_end..key_end + 1, TOKEN_OPERATOR, 0);
        key_end + 1
    } else {
        key_end
    };
    let value_slice = source.get(value_start..line_range.end).unwrap_or("");
    let value_end = if let Some(comment_rel) = find_unescaped_comment(value_slice) {
        let comment_start = value_start + comment_rel;
        push_token(tokens, comment_start..line_range.end, TOKEN_COMMENT, 0);
        comment_start
    } else {
        line_range.end
    };

    if let FieldValue::Voice(voice) = &field.value
        && let SourceText::Span(id_span) = voice.id
    {
        let id_range = rebase(id_span, base);
        push_token(
            tokens,
            id_range.clone(),
            TOKEN_VARIABLE,
            MODIFIER_DECLARATION,
        );
        collect_structured_field_value_tokens(
            id_range.end.min(value_end)..value_end,
            source,
            tokens,
        );
    } else if matches!(
        field.kind,
        FieldKind::Reference
            | FieldKind::UnitLength
            | FieldKind::Meter
            | FieldKind::Tempo
            | FieldKind::Key
            | FieldKind::Voice
            | FieldKind::Parts
            | FieldKind::UserSymbol
            | FieldKind::Macro
    ) {
        collect_structured_field_value_tokens(value_start..value_end, source, tokens);
    }
}

fn collect_structured_field_value_tokens(
    range: Range<usize>,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    let mut chars = slice.char_indices().peekable();
    while let Some((pos, ch)) = chars.next() {
        let abs = range.start + pos;
        match ch {
            '"' => {
                let mut end = pos + ch.len_utf8();
                for (next_pos, next_ch) in chars.by_ref() {
                    end = next_pos + next_ch.len_utf8();
                    if next_ch == '"' {
                        break;
                    }
                }
                push_token(tokens, abs..range.start + end, TOKEN_STRING, 0);
            }
            '0'..='9' => {
                let mut end = pos + ch.len_utf8();
                while let Some(&(next_pos, '0'..='9')) = chars.peek() {
                    chars.next();
                    end = next_pos + 1;
                }
                push_token(tokens, abs..range.start + end, TOKEN_NUMBER, 0);
            }
            '/' | '=' | '+' | '(' | ')' => {
                push_token(tokens, abs..abs + ch.len_utf8(), TOKEN_OPERATOR, 0);
            }
            _ => {}
        }
    }
}

type SpannedMusicElement = Spanned<MusicElement<SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>;

fn collect_music_line_tokens(
    elements: &[SpannedMusicElement],
    line_range: Range<usize>,
    base: isize,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    for element in elements {
        let elem_range = rebase(element.span, base);
        match &element.value {
            MusicElement::Note(_)
            | MusicElement::Rest(_)
            | MusicElement::MultiMeasureRest(_)
            | MusicElement::Chord(_)
            | MusicElement::Grace(_) => {
                collect_music_group_tokens(elem_range, source, tokens);
            }
            MusicElement::Decoration(decoration) => {
                let modifiers = u32::from(decoration.legacy_delimiter) * MODIFIER_DEPRECATED;
                push_token(tokens, elem_range, TOKEN_DECORATOR, modifiers);
            }
            MusicElement::Annotation(_) => {
                push_token(tokens, elem_range, TOKEN_STRING, 0);
            }
            MusicElement::InlineField(field) => {
                if elem_range.len() >= 2 {
                    push_token(
                        tokens,
                        elem_range.start..elem_range.start + 1,
                        TOKEN_OPERATOR,
                        0,
                    );
                    collect_field_line_tokens(
                        field,
                        elem_range.start + 1..elem_range.end - 1,
                        base,
                        source,
                        tokens,
                    );
                    push_token(
                        tokens,
                        elem_range.end - 1..elem_range.end,
                        TOKEN_OPERATOR,
                        0,
                    );
                }
            }
            MusicElement::Bar(_)
            | MusicElement::Slur(_)
            | MusicElement::Tie(_)
            | MusicElement::BrokenRhythm(_)
            | MusicElement::Overlay(_)
            | MusicElement::LineBreak(_) => {
                push_token(tokens, elem_range, TOKEN_OPERATOR, 0);
            }
            MusicElement::Ending(_) | MusicElement::Tuplet(_) => {
                collect_operator_and_number_tokens(elem_range, source, tokens);
            }
            MusicElement::Extension(_) => {
                collect_lexical_fallback_tokens(elem_range, source, tokens);
            }
            MusicElement::BeamBreak(_) | MusicElement::BeamContinuation(_) | _ => {}
        }
    }
    let music_end = elements
        .last()
        .map_or(line_range.start, |element| rebase(element.span, base).end);
    collect_trailing_comment(music_end..line_range.end, source, tokens);
}

fn collect_music_group_tokens(range: Range<usize>, source: &str, tokens: &mut Vec<RawToken>) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    for (pos, ch) in slice.char_indices() {
        let kind = match ch {
            'A'..='G' | 'a'..='g' | 'z' | 'x' | 'Z' | 'X' => TOKEN_ENUM_MEMBER,
            '0'..='9' => TOKEN_NUMBER,
            '[' | ']' | '{' | '}' | '^' | '_' | '=' | '/' | '<' | '>' => TOKEN_OPERATOR,
            _ => continue,
        };
        let abs = range.start + pos;
        push_token(tokens, abs..abs + ch.len_utf8(), kind, 0);
    }
}

fn collect_operator_and_number_tokens(
    range: Range<usize>,
    source: &str,
    tokens: &mut Vec<RawToken>,
) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    for (pos, ch) in slice.char_indices() {
        let kind = match ch {
            '0'..='9' => TOKEN_NUMBER,
            '[' | '(' | ':' | ',' | '-' => TOKEN_OPERATOR,
            _ => continue,
        };
        let abs = range.start + pos;
        push_token(tokens, abs..abs + ch.len_utf8(), kind, 0);
    }
}

fn collect_lexical_fallback_tokens(range: Range<usize>, source: &str, tokens: &mut Vec<RawToken>) {
    let Some(slice) = source.get(range.clone()) else {
        return;
    };
    for (pos, ch) in slice.char_indices() {
        let kind = match ch {
            '"' => TOKEN_STRING,
            '0'..='9' => TOKEN_NUMBER,
            'A'..='G' | 'a'..='g' => TOKEN_ENUM_MEMBER,
            '|' | ':' | '/' | '<' | '>' | '(' | ')' | '[' | ']' | '^' | '_' | '=' => TOKEN_OPERATOR,
            '!' | '+' => TOKEN_DECORATOR,
            _ => continue,
        };
        let abs = range.start + pos;
        let modifiers = u32::from(ch == '+') * MODIFIER_DEPRECATED;
        push_token(tokens, abs..abs + ch.len_utf8(), kind, modifiers);
    }
}

pub fn duration_edits(
    blocks: &[BlockRecord],
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    style: NoteLengthStyle,
    scope: Range<usize>,
) -> Vec<TextEdit> {
    let emit_style = match style {
        NoteLengthStyle::Shorthand => AbcNoteLengthStyle::Shorthand,
        NoteLengthStyle::Explicit => AbcNoteLengthStyle::ExplicitDenominator,
        NoteLengthStyle::Preserve => return Vec::new(),
    };
    let options = EmitOptions::new().with_note_length_style(emit_style);
    note_duration_ranges(blocks, index.source())
        .into_iter()
        .filter(|(range, _)| range.start >= scope.start && range.end <= scope.end)
        .filter_map(|(range, suffix)| {
            let length = parse_note_length(suffix).ok()?;
            if length.numerator != 1
                || length.denominator <= 1
                || !length.denominator.is_power_of_two()
            {
                return None;
            }
            let replacement = length.to_abc_with_options(options);
            (replacement != suffix).then(|| {
                TextEdit::new(
                    index
                        .lsp_range(range, encoding)
                        .expect("duration token is on character boundaries"),
                    replacement,
                )
            })
        })
        .collect()
}

fn note_duration_ranges<'a>(
    blocks: &'a [BlockRecord],
    source: &'a str,
) -> Vec<(Range<usize>, &'a str)> {
    let mut found = Vec::new();
    for record in blocks {
        for item in &record.parsed.items {
            if let DocumentItem::Tune(tune) = &item.value {
                for line in &tune.lines {
                    if let Line::Music(elements) = &line.value {
                        for element in elements {
                            collect_element_duration(element, record.base, source, &mut found);
                        }
                    }
                }
            }
        }
    }
    found
}

fn collect_element_duration<'a>(
    element: &Spanned<MusicElement<SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>,
    base: isize,
    source: &'a str,
    found: &mut Vec<(Range<usize>, &'a str)>,
) {
    let elem_range = rebase(element.span, base);
    if elem_range.end > source.len() || elem_range.start >= elem_range.end {
        return;
    }
    let elem_text = &source[elem_range.clone()];
    match &element.value {
        MusicElement::Note(_) => {
            if let Some(pitch_idx) = elem_text.find(|c: char| matches!(c, 'A'..='G' | 'a'..='g')) {
                let after_pitch = &elem_text[pitch_idx + 1..];
                let oct_len = after_pitch
                    .find(|c: char| c != '\'' && c != ',')
                    .unwrap_or(after_pitch.len());
                let suffix_offset = pitch_idx + 1 + oct_len;
                if suffix_offset < elem_text.len() {
                    found.push((
                        elem_range.start + suffix_offset..elem_range.end,
                        &elem_text[suffix_offset..],
                    ));
                }
            }
        }
        MusicElement::Rest(_) => {
            if elem_text.len() > 1 {
                found.push((elem_range.start + 1..elem_range.end, &elem_text[1..]));
            }
        }
        MusicElement::Chord(_) => {
            collect_chord_durations(&elem_range, elem_text, found);
        }
        _ => {}
    }
}

fn collect_chord_durations<'a>(
    elem_range: &Range<usize>,
    elem_text: &'a str,
    found: &mut Vec<(Range<usize>, &'a str)>,
) {
    let Some(close_bracket) = elem_text.rfind(']') else {
        return;
    };
    if close_bracket + 1 < elem_text.len() {
        found.push((
            elem_range.start + close_bracket + 1..elem_range.end,
            &elem_text[close_bracket + 1..],
        ));
    }
    let inside = &elem_text[1..close_bracket];
    let bytes = inside.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if matches!(bytes[cursor], b'A'..=b'G' | b'a'..=b'g') {
            cursor += 1;
            while cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b',') {
                cursor += 1;
            }
            let dur_start = cursor;
            while cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'/')
            {
                cursor += 1;
            }
            if cursor > dur_start {
                found.push((
                    elem_range.start + 1 + dur_start..elem_range.start + 1 + cursor,
                    &inside[dur_start..cursor],
                ));
            }
        } else if matches!(bytes[cursor], b'z' | b'x') {
            cursor += 1;
            let dur_start = cursor;
            while cursor < bytes.len() && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'/')
            {
                cursor += 1;
            }
            if cursor > dur_start {
                found.push((
                    elem_range.start + 1 + dur_start..elem_range.start + 1 + cursor,
                    &inside[dur_start..cursor],
                ));
            }
        } else {
            cursor += 1;
        }
    }
}

fn duration_suffix(text: &str) -> Option<&str> {
    let end = text
        .find(|value: char| !value.is_ascii_digit() && value != '/')
        .unwrap_or(text.len());
    (end > 0).then_some(&text[..end])
}

#[cfg(test)]
mod tests {
    use tower_lsp_server::ls_types::DiagnosticSeverity;
    use tower_lsp_server::ls_types::NumberOrString;

    use super::*;
    use crate::config::Config;
    use crate::config::DiagnosticLevel;
    use crate::document::DocumentModel;

    #[test]
    fn duration_edits_only_rewrite_equivalent_music_suffixes() {
        let source = "X:1\nT:A/2 stays text\nK:C\nA/2 B/ C// D/4 E3/\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let shorthand = duration_edits(
            &model.blocks,
            &model.index,
            &model.encoding,
            NoteLengthStyle::Shorthand,
            0..model.index.source().len(),
        );
        assert_eq!(
            shorthand
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["/", "//"]
        );
        let explicit = duration_edits(
            &model.blocks,
            &model.index,
            &model.encoding,
            NoteLengthStyle::Explicit,
            0..model.index.source().len(),
        );
        assert_eq!(
            explicit
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["/2", "/4"]
        );
    }

    #[test]
    fn duration_edits_does_not_corrupt_non_music_elements() {
        let source = "%%begintext\nThis is section A/2 of the book.\n%%endtext\n\nX:1\nT:Title A/2\nK:C\n\"Am7\" \"C/2\" [V:1 name=\"A/2\"] !accent! +D.C.+ A/2 [CEG]/2 [C/2E/2] |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let shorthand = duration_edits(
            &model.blocks,
            &model.index,
            &model.encoding,
            NoteLengthStyle::Shorthand,
            0..model.index.source().len(),
        );
        assert_eq!(
            shorthand
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["/", "/", "/", "/"]
        );
    }

    #[test]
    fn analysis_honors_optional_diagnostic_levels() {
        let source = "X:1\nK:C\n+trill+A\n";
        let encoding = PositionEncodingKind::UTF16;
        let mut config = Config::default();
        config.validation.legacy_decoration = DiagnosticLevel::Off;
        let disabled = DocumentModel::full(source.to_owned(), 0, encoding.clone(), config);
        assert!(disabled.diagnostics.iter().all(|diagnostic| diagnostic.code
            != Some(NumberOrString::String("legacy-decoration".to_owned()))));

        config.validation.legacy_decoration = DiagnosticLevel::Hint;
        let enabled = DocumentModel::full(source.to_owned(), 0, encoding, config);
        assert!(enabled.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("legacy-decoration".to_owned()))
                && diagnostic.severity == Some(DiagnosticSeverity::HINT)
        }));
    }

    #[test]
    fn legacy_decorations_ignores_comments_annotations_and_free_text() {
        let source = "\
% +comment+ decoration ignored
X:1
T:Title
K:C
%%begintext
Some +freetext+ here
%%endtext
\"+add9+\" C +trill+ D\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let legacy = model
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("legacy-decoration".to_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(legacy.len(), 1);
        assert_eq!(legacy[0].range.start.line, 7);
        assert_eq!(legacy[0].range.start.character, 11);
        assert_eq!(legacy[0].range.end.character, 18);
    }

    #[test]
    fn bar_duration_diagnostics_skip_only_the_trailing_open_bar() {
        let source = "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C | CDEFG | CCCCC\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let diagnostics = model
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("bar-duration".to_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].range.start.line, 4);
        assert_eq!(diagnostics[0].range.start.character, 9);
        assert_eq!(diagnostics[0].message, "bar duration is 1 beat, expected 4");
        assert_eq!(diagnostics[1].range.start.character, 17);
        assert_eq!(
            diagnostics[1].message,
            "bar duration is 5 beats, expected 4"
        );
    }

    #[test]
    fn closing_the_trailing_bar_activates_its_diagnostic() {
        let open = "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C\n";
        let closed = "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C |\n";
        let encoding = PositionEncodingKind::UTF16;
        let code = Some(NumberOrString::String("bar-duration".to_owned()));
        assert!(
            DocumentModel::full(open.to_owned(), 0, encoding.clone(), Config::default())
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != code)
        );
        assert!(
            DocumentModel::full(closed.to_owned(), 0, encoding, Config::default())
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code)
        );
    }

    #[test]
    fn bar_duration_level_and_opening_pickup_are_editor_specific() {
        let source = "X:1\nT:Cairo Waltz\nM:4/4\nL:1/8\nK:D\nabcd | a2b2c2d|\n";
        let encoding = PositionEncodingKind::UTF16;
        let mut config = Config::default();
        config.validation.bar_duration = DiagnosticLevel::Information;
        let model = DocumentModel::full(source.to_owned(), 0, encoding.clone(), config);
        let diagnostics = model
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("bar-duration".to_owned()))
            })
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].severity,
            Some(DiagnosticSeverity::INFORMATION)
        );
        assert_eq!(diagnostics[0].range.start.character, 14);
        assert_eq!(
            diagnostics[0].message,
            "bar duration is 3 1/2 beats, expected 4"
        );

        config.validation.bar_duration = DiagnosticLevel::Off;
        assert!(
            DocumentModel::full(source.to_owned(), 0, encoding, config)
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code
                    != Some(NumberOrString::String("bar-duration".to_owned())))
        );
    }

    #[test]
    fn analysis_preserves_variant_ending_parser_diagnostics() {
        let source = "X:1\nK:C\n:| 2 c\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert_eq!(model.diagnostics.len(), 1);
        let diagnostic = &model.diagnostics[0];
        assert_eq!(diagnostic.range.start.line, 2);
        assert_eq!(diagnostic.range.start.character, 3);
        assert_eq!(diagnostic.range.end.line, 2);
        assert_eq!(diagnostic.range.end.character, 4);
        assert_eq!(
            diagnostic.message,
            "variant ending 2 must be adjacent to the bar line or begin with '['"
        );
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("invalid-music".to_owned()))
        );
    }

    #[test]
    fn symbols_and_folds_describe_tunes_voices_and_text_blocks() {
        let source = "%%begintext\nnotes\n%%endtext\n\nX:7\nT:Example\nV:top\nK:C\nC\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let symbols = document_symbols(&model.blocks, &model.index, &model.encoding);
        assert_eq!(symbols[0].name, "Example");
        assert_eq!(symbols[0].children.as_ref().map(Vec::len), Some(1));
        let folds = folding_ranges(&model.index);
        assert!(
            folds
                .iter()
                .any(|fold| fold.collapsed_text.as_deref() == Some("text"))
        );
        assert!(
            folds
                .iter()
                .any(|fold| fold.collapsed_text.as_deref() == Some("tune"))
        );
    }

    #[test]
    fn document_symbols_excludes_inter_tune_text_and_includes_headerless_tunes() {
        let source = "\
X:1
T:First Tune
K:C
CDEF |

Some free text between tunes.

%%begintext
Typeset text block between tunes.
X:888
V:fake
%%endtext

T:Headerless Tune
V:melody
K:G
GABc |
";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let symbols = document_symbols(&model.blocks, &model.index, &model.encoding);
        assert_eq!(symbols.len(), 2);

        assert_eq!(symbols[0].name, "First Tune");
        assert_eq!(symbols[0].detail.as_deref(), Some("X:1"));
        assert_eq!(symbols[0].range.start.line, 0);
        assert_eq!(symbols[0].range.end.line, 3);
        assert_eq!(symbols[0].range.end.character, 6);
        assert!(symbols[0].children.is_none());

        assert_eq!(symbols[1].name, "Headerless Tune");
        assert_eq!(symbols[1].detail, None);
        assert_eq!(symbols[1].range.start.line, 13);
        assert_eq!(symbols[1].range.end.line, 16);
        assert_eq!(symbols[1].range.end.character, 6);
        let voices = symbols[1].children.as_ref().expect("voice children");
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].name, "melody");
    }

    #[test]
    fn structural_completions_replace_the_typed_prefix() {
        let field = LineIndex::new("M".to_owned());
        let items = completions(&field, &PositionEncodingKind::UTF16, 1);
        let meter = items
            .iter()
            .find(|item| item.label == "M:")
            .expect("meter field completion");
        assert!(matches!(
            &meter.text_edit,
            Some(CompletionTextEdit::Edit(edit))
                if edit.range == LspRange::new(
                    tower_lsp_server::ls_types::Position::new(0, 0),
                    tower_lsp_server::ls_types::Position::new(0, 1),
                ) && edit.new_text == "M:"
        ));

        let directive = LineIndex::new("%%beg".to_owned());
        let items = completions(&directive, &PositionEncodingKind::UTF16, 5);
        let begin = items
            .iter()
            .find(|item| item.label == "%%begintext")
            .expect("begin-text directive completion");
        assert!(matches!(
            &begin.text_edit,
            Some(CompletionTextEdit::Edit(edit)) if edit.new_text == "%%begintext"
        ));
    }

    #[test]
    fn key_completions_cover_three_sharps_and_three_flats() {
        let index = LineIndex::new("K:".to_owned());
        let labels = completions(&index, &PositionEncodingKind::UTF16, 2)
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "C", "G", "D", "A", "F", "Bb", "Eb", "Am", "Em", "Bm", "F#m", "Dm", "Gm", "Cm",
            ]
        );
    }

    #[test]
    fn hover_dispatches_on_parsed_line_variant() {
        let source = "\
X:1
T:Sample
K:C
% T:commented out
%%begintext
1: first item
a: hello
%%endtext
:: C2 D |

1: top-level free text
a: second free-text line
";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );

        // Genuine field line: X:1
        let x_offset = source.find("X:1").expect("X:1");
        let (x_range, x_hover) = hover(&model.blocks, &model.index, x_offset).expect("field hover");
        assert_eq!(x_range, x_offset..x_offset + 1);
        assert_eq!(x_hover, "**X:** — Tune reference number");

        // Comment line inside tune
        let comment_offset = source.find("% T:commented").expect("comment");
        assert_eq!(hover(&model.blocks, &model.index, comment_offset), None);

        // Tune-local %%begintext lines with ':' at byte 1
        let item_offset = source.find("1: first item").expect("1: first item");
        assert_eq!(hover(&model.blocks, &model.index, item_offset), None);
        let hello_offset = source.find("a: hello").expect("a: hello");
        assert_eq!(hover(&model.blocks, &model.index, hello_offset), None);

        // Music line starting with '::': repeat delimiter and subsequent notes
        let repeat_offset = source.find(":: C2").expect(":: C2");
        let (bar_range, bar_hover) =
            hover(&model.blocks, &model.index, repeat_offset).expect("bar hover");
        assert_eq!(bar_range, repeat_offset..repeat_offset + 1);
        assert_eq!(bar_hover, "Bar or repeat delimiter");

        let note_offset = repeat_offset + 3; // 'C' in ":: C2"
        let (note_range, note_hover) =
            hover(&model.blocks, &model.index, note_offset).expect("note hover");
        assert_eq!(note_range, note_offset..note_offset + 1);
        assert_eq!(note_hover, "ABC note `C` — duration multiplier 2/1");

        // Top-level free text block with ':' at byte 1
        let free_offset = source.find("1: top-level").expect("1: top-level");
        assert_eq!(hover(&model.blocks, &model.index, free_offset), None);
        let free2_offset = source.find("a: second").expect("a: second");
        assert_eq!(hover(&model.blocks, &model.index, free2_offset), None);
    }

    #[test]
    fn semantic_tokens_emits_from_parsed_ast_spans() {
        let source = "\
%%begintext
A B C | 1:2 \"quoted\" x: not a field
é: multi-byte prefix
%%endtext

é: top-level free text | A B C

X:1
T:A | B \"Title\" é:1
V:melody name=\"Flute\"
K:C
\"Am7\" !fermata! +trill+ C2 | % comment A B C
";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let result = semantic_tokens(&model.blocks, &model.index, &model.encoding, None);

        // Reconstruct (line, start_char, length, token_type, modifiers)
        let mut decoded = Vec::new();
        let mut line = 0_u32;
        let mut start = 0_u32;
        for token in &result.data {
            if token.delta_line > 0 {
                line += token.delta_line;
                start = token.delta_start;
            } else {
                start += token.delta_start;
            }
            decoded.push((
                line,
                start,
                token.length,
                token.token_type,
                token.token_modifiers_bitset,
            ));
        }

        // Lines 1, 2 (inside %%begintext) and line 5 (top-level free text) must have no tokens
        assert!(decoded.iter().all(|(l, ..)| *l != 1 && *l != 2 && *l != 5));

        assert_eq!(
            decoded,
            vec![
                // Line 0: %%begintext
                (0, 0, 11, TOKEN_KEYWORD, 0),
                // Line 3: %%endtext
                (3, 0, 9, TOKEN_KEYWORD, 0),
                // Line 7: X:1
                (7, 0, 1, TOKEN_KEYWORD, 0),
                (7, 1, 1, TOKEN_OPERATOR, 0),
                (7, 2, 1, TOKEN_NUMBER, 0),
                // Line 8: T:A | B "Title" é:1 (textual field: only T and :)
                (8, 0, 1, TOKEN_KEYWORD, 0),
                (8, 1, 1, TOKEN_OPERATOR, 0),
                // Line 9: V:melody name="Flute"
                (9, 0, 1, TOKEN_KEYWORD, 0),
                (9, 1, 1, TOKEN_OPERATOR, 0),
                (9, 2, 6, TOKEN_VARIABLE, MODIFIER_DECLARATION),
                (9, 13, 1, TOKEN_OPERATOR, 0),
                (9, 14, 7, TOKEN_STRING, 0),
                // Line 10: K:C
                (10, 0, 1, TOKEN_KEYWORD, 0),
                (10, 1, 1, TOKEN_OPERATOR, 0),
                // Line 11: "Am7" !fermata! +trill+ C2 | % comment A B C
                (11, 0, 5, TOKEN_STRING, 0),
                (11, 6, 9, TOKEN_DECORATOR, 0),
                (11, 16, 7, TOKEN_DECORATOR, MODIFIER_DEPRECATED),
                (11, 24, 1, TOKEN_ENUM_MEMBER, 0),
                (11, 25, 1, TOKEN_NUMBER, 0),
                (11, 27, 1, TOKEN_OPERATOR, 0),
                (11, 29, 15, TOKEN_COMMENT, 0),
            ]
        );
    }
}
