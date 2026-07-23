// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Parsed diagnostics and editor-facing source analysis.

use std::ops::Range;

use abc_parser::BarDurationOptions;
use abc_parser::BarDurationPickupPolicy;
use abc_parser::ErrorKind;
use abc_parser::IntoOwnedAst;
use abc_parser::ParserOptions;
use abc_parser::bar_duration_warnings;
use abc_parser::parse_with_options;
use tower_lsp_server::ls_types::CompletionItem;
use tower_lsp_server::ls_types::CompletionItemKind;
use tower_lsp_server::ls_types::CompletionTextEdit;
use tower_lsp_server::ls_types::Diagnostic;
use tower_lsp_server::ls_types::DiagnosticSeverity;
use tower_lsp_server::ls_types::DiagnosticTag;
use tower_lsp_server::ls_types::DocumentSymbol;
use tower_lsp_server::ls_types::FoldingRange;
use tower_lsp_server::ls_types::FoldingRangeKind;
use tower_lsp_server::ls_types::NumberOrString;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::Range as LspRange;
use tower_lsp_server::ls_types::SelectionRange;
use tower_lsp_server::ls_types::SemanticToken;
use tower_lsp_server::ls_types::SemanticTokens;
use tower_lsp_server::ls_types::SymbolKind;
use tower_lsp_server::ls_types::TextEdit;

use crate::config::Config;
use crate::config::DiagnosticLevel;
use crate::config::NoteLengthStyle;
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

/// Immutable analysis of one synchronized document version.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) has_errors: bool,
}

impl Analysis {
    pub fn new(index: &LineIndex, encoding: &PositionEncodingKind, config: Config) -> Self {
        let report = parse_with_options(
            index.source(),
            ParserOptions::new().strict(config.validation.strict),
        );
        let has_errors = !report.errors.is_empty();
        let abc_parser::ParseReport {
            output,
            errors,
            warnings,
        } = report;
        let bar_duration_warnings = severity(config.validation.bar_duration).and_then(|level| {
            output
                .and_then(|document| document.into_owned(index.source()).ok())
                .map(|document| {
                    (
                        level,
                        bar_duration_warnings(
                            &document,
                            BarDurationOptions::new()
                                .pickup_policy(BarDurationPickupPolicy::OpeningBar)
                                .check_trailing_bar(false),
                        ),
                    )
                })
        });
        let mut diagnostics = errors
            .into_iter()
            .filter_map(|error| {
                diagnostic(
                    index,
                    encoding,
                    error.span.start..error.span.end,
                    DiagnosticSeverity::ERROR,
                    error_kind_code(error.kind),
                    error.message,
                    None,
                )
            })
            .collect::<Vec<_>>();
        diagnostics.extend(warnings.into_iter().filter_map(|warning| {
            let level = if warning.kind == ErrorKind::MissingReference {
                config.validation.ambiguous_music
            } else {
                DiagnosticLevel::Warning
            };
            diagnostic(
                index,
                encoding,
                warning.span.start..warning.span.end,
                severity(level)?,
                error_kind_code(warning.kind),
                warning.message,
                None,
            )
        }));
        if let Some((level, warnings)) = bar_duration_warnings {
            diagnostics.extend(warnings.into_iter().filter_map(|warning| {
                diagnostic(
                    index,
                    encoding,
                    warning.span.start..warning.span.end,
                    level,
                    "bar-duration",
                    lsp_bar_duration_message(warning.message),
                    None,
                )
            }));
        }
        if let Some(level) = severity(config.validation.legacy_decoration) {
            diagnostics.extend(legacy_decorations(index.source()).filter_map(|range| {
                diagnostic(
                    index,
                    encoding,
                    range,
                    level,
                    "legacy-decoration",
                    "legacy +name+ decoration; prefer !name!".to_owned(),
                    Some(vec![DiagnosticTag::DEPRECATED]),
                )
            }));
        }
        Self {
            diagnostics,
            has_errors,
        }
    }
}

fn lsp_bar_duration_message(message: String) -> String {
    if let Some(prefix) = message.strip_suffix(" beats under the effective meter") {
        return prefix.to_owned();
    }
    if let Some(prefix) = message.strip_suffix(" beat under the effective meter") {
        return prefix.to_owned();
    }
    message
}

fn diagnostic(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    range: Range<usize>,
    severity: DiagnosticSeverity,
    code: &'static str,
    message: String,
    tags: Option<Vec<DiagnosticTag>>,
) -> Option<Diagnostic> {
    Some(Diagnostic::new(
        index.lsp_range(range, encoding)?,
        Some(severity),
        Some(NumberOrString::String(code.to_owned())),
        Some("abc-parser".to_owned()),
        message,
        None,
        tags,
    ))
}

const fn severity(level: DiagnosticLevel) -> Option<DiagnosticSeverity> {
    match level {
        DiagnosticLevel::Off => None,
        DiagnosticLevel::Hint => Some(DiagnosticSeverity::HINT),
        DiagnosticLevel::Information => Some(DiagnosticSeverity::INFORMATION),
        DiagnosticLevel::Warning => Some(DiagnosticSeverity::WARNING),
        DiagnosticLevel::Error => Some(DiagnosticSeverity::ERROR),
    }
}

const fn error_kind_code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UnclosedDelimiter => "unclosed-delimiter",
        ErrorKind::InvalidField => "invalid-field",
        ErrorKind::InvalidDirective => "invalid-directive",
        ErrorKind::InvalidMusic => "invalid-music",
        ErrorKind::MissingReference => "missing-reference",
        ErrorKind::InvalidFieldOrder => "invalid-field-order",
        _ => "parser-diagnostic",
    }
}

fn legacy_decorations(source: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    source.match_indices('+').filter_map(|(start, _)| {
        let tail = &source[start + 1..];
        let length = tail
            .find('+')
            .filter(|end| *end > 0 && tail[..*end].chars().all(char::is_alphanumeric))?;
        Some(start..start + length + 2)
    })
}

pub fn hover(index: &LineIndex, offset: usize) -> Option<(Range<usize>, String)> {
    let line = index.line_bounds(offset)?;
    let text = &index.source()[line.clone()];
    if text.as_bytes().get(1) == Some(&b':') {
        let key = text.chars().next()?;
        let explanation = FIELD_COMPLETIONS
            .iter()
            .find_map(|(candidate, description)| (*candidate == key).then_some(*description))
            .unwrap_or("ABC information field");
        return Some((
            line.start..line.start + 1,
            format!("**{key}:** — {explanation}"),
        ));
    }
    let relative = offset.saturating_sub(line.start).min(text.len());
    let (start, character) = text
        .char_indices()
        .take_while(|(position, _)| *position <= relative)
        .last()?;
    let absolute = line.start + start;
    match character {
        'A'..='G' | 'a'..='g' => {
            let suffix = duration_suffix(&text[start + character.len_utf8()..]);
            let detail = suffix.and_then(parse_duration).map_or_else(
                || "unit note length".to_owned(),
                |(numerator, denominator)| {
                    let equivalent = if numerator == 1 && denominator.is_power_of_two() {
                        format!(
                            "; shorthand `{}` = explicit `/{denominator}`",
                            "/".repeat(denominator.ilog2() as usize)
                        )
                    } else {
                        String::new()
                    };
                    format!("duration multiplier {numerator}/{denominator}{equivalent}")
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

pub fn document_symbols(index: &LineIndex, encoding: &PositionEncodingKind) -> Vec<DocumentSymbol> {
    let source = index.source();
    let mut starts = source
        .split_inclusive('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line.trim_end_matches(['\r', '\n'])))
        })
        .filter(|(_, line)| line.starts_with("X:"))
        .collect::<Vec<_>>();
    if starts.is_empty() {
        return Vec::new();
    }
    starts.push((source.len(), ""));
    starts
        .windows(2)
        .filter_map(|window| {
            let (start, reference_line) = window[0];
            let end = window[1].0;
            let body = &source[start..end];
            let title = body
                .lines()
                .find_map(|line| line.strip_prefix("T:").map(str::trim))
                .filter(|title| !title.is_empty());
            let reference = reference_line.trim_start_matches("X:").trim();
            let name = title.map_or_else(|| format!("Tune {reference}"), ToOwned::to_owned);
            let range = index.lsp_range(start..end, encoding)?;
            let selection_range = index.lsp_range(start..start + reference_line.len(), encoding)?;
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name,
                detail: Some(format!("X:{reference}")),
                kind: SymbolKind::MODULE,
                tags: None,
                deprecated: None,
                range,
                selection_range,
                children: voice_symbols(index, encoding, start, body),
            })
        })
        .collect()
}

fn voice_symbols(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    base: usize,
    body: &str,
) -> Option<Vec<DocumentSymbol>> {
    let mut offset = base;
    let voices = body
        .split_inclusive('\n')
        .filter_map(|line| {
            let start = offset;
            offset += line.len();
            let trimmed = line.trim_end_matches(['\r', '\n']);
            let id = trimmed.strip_prefix("V:")?.split_whitespace().next()?;
            let range = index.lsp_range(start..start + trimmed.len(), encoding)?;
            #[allow(deprecated)]
            Some(DocumentSymbol {
                name: id.to_owned(),
                detail: Some("Voice".to_owned()),
                kind: SymbolKind::VARIABLE,
                tags: None,
                deprecated: None,
                range,
                selection_range: index.lsp_range(start + 2..start + 2 + id.len(), encoding)?,
                children: None,
            })
        })
        .collect::<Vec<_>>();
    (!voices.is_empty()).then_some(voices)
}

pub fn folding_ranges(index: &LineIndex) -> Vec<FoldingRange> {
    let source = index.source();
    let mut tune_start = None;
    let mut text_start = None;
    let mut ranges = Vec::new();
    let lines = source.lines().collect::<Vec<_>>();
    for (line_number, line) in lines.iter().enumerate() {
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
    if let Some(start) = tune_start {
        push_fold(&mut ranges, start, lines.len().saturating_sub(1), "tune");
    }
    if let Some(start) = text_start {
        push_fold(&mut ranges, start, lines.len().saturating_sub(1), "text");
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

pub fn semantic_tokens(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    filter: Option<LspRange>,
) -> SemanticTokens {
    let filter = filter.and_then(|range| index.byte_range(range, encoding));
    let mut raw = lexical_tokens(index.source());
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

fn lexical_tokens(source: &str) -> Vec<RawToken> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        if content.starts_with('%') && !content.starts_with("%%") {
            tokens.push(RawToken {
                range: offset..offset + content.len(),
                kind: 0,
                modifiers: 0,
            });
        } else {
            if content.as_bytes().get(1) == Some(&b':') {
                tokens.push(RawToken {
                    range: offset..offset + 1,
                    kind: 1,
                    modifiers: 0,
                });
            } else if content.starts_with("%%") {
                let end = content.find(char::is_whitespace).unwrap_or(content.len());
                tokens.push(RawToken {
                    range: offset..offset + end,
                    kind: 1,
                    modifiers: 0,
                });
            }
            let mut quoted = false;
            for (position, value) in content.char_indices() {
                let kind = match value {
                    '"' => {
                        quoted = !quoted;
                        3
                    }
                    _ if quoted => 3,
                    '0'..='9' => 2,
                    'A'..='G' | 'a'..='g' if content.as_bytes().get(1) != Some(&b':') => 4,
                    '|' | ':' | '/' | '<' | '>' | '(' | ')' | '[' | ']' | '^' | '_' | '=' => 5,
                    '!' | '+' => 6,
                    _ => continue,
                };
                tokens.push(RawToken {
                    range: offset + position..offset + position + value.len_utf8(),
                    kind,
                    modifiers: u32::from(value == '+') << 1,
                });
            }
        }
        offset += line.len();
    }
    tokens
}

pub fn duration_edits(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    style: NoteLengthStyle,
    scope: Range<usize>,
) -> Vec<TextEdit> {
    if style == NoteLengthStyle::Preserve {
        return Vec::new();
    }
    note_duration_ranges(index.source())
        .filter(|(range, _)| range.start >= scope.start && range.end <= scope.end)
        .filter_map(|(range, suffix)| {
            let (numerator, denominator) = parse_duration(suffix)?;
            if numerator != 1 || denominator <= 1 || !denominator.is_power_of_two() {
                return None;
            }
            let replacement = match style {
                NoteLengthStyle::Shorthand => "/".repeat(denominator.ilog2() as usize),
                NoteLengthStyle::Explicit => format!("/{denominator}"),
                NoteLengthStyle::Preserve => return None,
            };
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

fn note_duration_ranges(source: &str) -> impl Iterator<Item = (Range<usize>, &str)> {
    source
        .split_inclusive('\n')
        .scan(0, |base, line| {
            let line_start = *base;
            *base += line.len();
            let content = line.trim_end_matches(['\r', '\n']);
            if content.starts_with('%') || content.as_bytes().get(1) == Some(&b':') {
                return Some(Vec::new());
            }
            let mut found = Vec::new();
            let bytes = content.as_bytes();
            let mut cursor = 0;
            while cursor < bytes.len() {
                if matches!(bytes[cursor], b'A'..=b'G' | b'a'..=b'g' | b'z' | b'x') {
                    cursor += 1;
                    while cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b',') {
                        cursor += 1;
                    }
                    let start = cursor;
                    while cursor < bytes.len()
                        && (bytes[cursor].is_ascii_digit() || bytes[cursor] == b'/')
                    {
                        cursor += 1;
                    }
                    if cursor > start {
                        found.push((
                            line_start + start..line_start + cursor,
                            &content[start..cursor],
                        ));
                    }
                } else {
                    cursor += 1;
                }
            }
            Some(found)
        })
        .flatten()
}

fn duration_suffix(text: &str) -> Option<&str> {
    let end = text
        .find(|value: char| !value.is_ascii_digit() && value != '/')
        .unwrap_or(text.len());
    (end > 0).then_some(&text[..end])
}

fn parse_duration(value: &str) -> Option<(u32, u32)> {
    if value.is_empty() {
        return Some((1, 1));
    }
    if value.bytes().all(|byte| byte == b'/') {
        return Some((1, 2_u32.checked_pow(u32::try_from(value.len()).ok()?)?));
    }
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = if numerator.is_empty() {
            1
        } else {
            numerator.parse().ok()?
        };
        let denominator = if denominator.is_empty() {
            2
        } else if denominator.bytes().all(|byte| byte == b'/') {
            2_u32.checked_pow(u32::try_from(denominator.len() + 1).ok()?)?
        } else {
            denominator.parse().ok()?
        };
        return Some((numerator, denominator));
    }
    Some((value.parse().ok()?, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_spellings_are_semantically_distinct_and_rewritable() {
        assert_eq!(parse_duration("/2"), Some((1, 2)));
        assert_eq!(parse_duration("/"), Some((1, 2)));
        assert_eq!(parse_duration("//"), Some((1, 4)));
        assert_eq!(parse_duration("/4"), Some((1, 4)));
        assert_eq!(parse_duration("3/"), Some((3, 2)));
        assert_eq!(parse_duration("3//"), Some((3, 4)));
    }

    #[test]
    fn duration_edits_only_rewrite_equivalent_music_suffixes() {
        let index = LineIndex::new("X:1\nT:A/2 stays text\nK:C\nA/2 B/ C// D/4 E3/\n".to_owned());
        let encoding = PositionEncodingKind::UTF16;
        let shorthand = duration_edits(
            &index,
            &encoding,
            NoteLengthStyle::Shorthand,
            0..index.source().len(),
        );
        assert_eq!(
            shorthand
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            ["/", "//"]
        );
        let explicit = duration_edits(
            &index,
            &encoding,
            NoteLengthStyle::Explicit,
            0..index.source().len(),
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
    fn analysis_honors_optional_diagnostic_levels() {
        let index = LineIndex::new("X:1\nK:C\n+trill+A\n".to_owned());
        let encoding = PositionEncodingKind::UTF16;
        let mut config = Config::default();
        config.validation.legacy_decoration = DiagnosticLevel::Off;
        let disabled = Analysis::new(&index, &encoding, config);
        assert!(disabled.diagnostics.iter().all(|diagnostic| diagnostic.code
            != Some(NumberOrString::String("legacy-decoration".to_owned()))));

        config.validation.legacy_decoration = DiagnosticLevel::Hint;
        let enabled = Analysis::new(&index, &encoding, config);
        assert!(enabled.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == Some(NumberOrString::String("legacy-decoration".to_owned()))
                && diagnostic.severity == Some(DiagnosticSeverity::HINT)
        }));
    }

    #[test]
    fn bar_duration_diagnostics_skip_only_the_trailing_open_bar() {
        let source = "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C | CDEFG | CCCCC\n";
        let index = LineIndex::new(source.to_owned());
        let analysis = Analysis::new(&index, &PositionEncodingKind::UTF16, Config::default());
        let diagnostics = analysis
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
        let open = LineIndex::new("X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C\n".to_owned());
        let closed = LineIndex::new("X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C |\n".to_owned());
        let encoding = PositionEncodingKind::UTF16;
        let code = Some(NumberOrString::String("bar-duration".to_owned()));
        assert!(
            Analysis::new(&open, &encoding, Config::default())
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != code)
        );
        assert!(
            Analysis::new(&closed, &encoding, Config::default())
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code)
        );
    }

    #[test]
    fn bar_duration_level_and_opening_pickup_are_editor_specific() {
        let index =
            LineIndex::new("X:1\nT:Cairo Waltz\nM:4/4\nL:1/8\nK:D\nabcd | a2b2c2d|\n".to_owned());
        let encoding = PositionEncodingKind::UTF16;
        let mut config = Config::default();
        config.validation.bar_duration = DiagnosticLevel::Information;
        let analysis = Analysis::new(&index, &encoding, config);
        let diagnostics = analysis
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
            Analysis::new(&index, &encoding, config)
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code
                    != Some(NumberOrString::String("bar-duration".to_owned())))
        );
    }

    #[test]
    fn analysis_preserves_variant_ending_parser_diagnostics() {
        let index = LineIndex::new("X:1\nK:C\n:| 2 c\n".to_owned());
        let analysis = Analysis::new(&index, &PositionEncodingKind::UTF16, Config::default());
        assert_eq!(analysis.diagnostics.len(), 1);
        let diagnostic = &analysis.diagnostics[0];
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
        let index = LineIndex::new(
            "%%begintext\nnotes\n%%endtext\nX:7\nT:Example\nV:top\nK:C\nC\n".to_owned(),
        );
        let symbols = document_symbols(&index, &PositionEncodingKind::UTF16);
        assert_eq!(symbols[0].name, "Example");
        assert_eq!(symbols[0].children.as_ref().map(Vec::len), Some(1));
        let folds = folding_ranges(&index);
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
}
