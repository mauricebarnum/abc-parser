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

//! Parser diagnostics, source-context rendering, and parse reports.

use std::fmt;
use std::fmt::Write as _;

use chumsky::error::Rich;
use chumsky::span::SimpleSpan;
use chumsky::span::Span as ChumskySpan;

use super::SourceResolver;
use super::Span;

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
    /// Recognized syntax is accepted for compatibility but is deprecated.
    DeprecatedSyntax,
    /// Valid input has a deterministic repair available from a consumer.
    FixableSyntax,
    /// A free text block appears outside any tune.
    FreeTextOutsideTune,
    /// An information field letter is not part of the §3 standard table.
    UnrecognizedField,
    /// A field restricted to the file or tune header appears in the tune body.
    HeaderFieldInBody,
    /// An `I:linebreak` instruction appears in the tune body.
    InstructionInBody,
    /// A `\` continuation at the end of a music line is followed by an empty line.
    MusicLineContinuationBeforeEmpty,
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

/// Reuses resolved source text and a progressive line index across diagnostics.
///
/// Constructing a renderer does not resolve its source or allocate an index.
/// Rendering the first diagnostic caches the complete source, when available,
/// and indexes only as far as that diagnostic requires. Later diagnostics at
/// greater offsets extend the index, while diagnostics at earlier offsets use
/// the line starts already discovered.
///
/// # Examples
///
/// ```
/// use abc_parser::DiagnosticRenderer;
/// use abc_parser::ErrorKind;
/// use abc_parser::ParseError;
///
/// let source = "X:1\nM:nope\nK:C\n";
/// let errors = [
///     ParseError {
///         kind: ErrorKind::InvalidField,
///         message: "invalid M: field value".to_owned(),
///         span: 4..10,
///     },
///     ParseError {
///         kind: ErrorKind::InvalidField,
///         message: "invalid K: field value".to_owned(),
///         span: 11..14,
///     },
/// ];
/// let mut renderer = DiagnosticRenderer::new(source);
/// let diagnostics = errors
///     .iter()
///     .map(|error| renderer.render_error(error))
///     .collect::<Vec<_>>();
/// assert!(diagnostics[0].starts_with("2:1:"));
/// assert!(diagnostics[1].starts_with("3:1:"));
/// ```
#[derive(Debug)]
pub struct DiagnosticRenderer<'source, R: ?Sized, S = Span> {
    resolver: &'source R,
    source: DiagnosticSource<'source>,
    line_starts: Vec<usize>,
    indexed_to: usize,
    span: std::marker::PhantomData<fn(&S)>,
}

/// Lazily resolved complete source retained by a diagnostic renderer.
#[derive(Debug)]
enum DiagnosticSource<'source> {
    Unresolved,
    Unavailable,
    Resolved(std::borrow::Cow<'source, str>),
}

impl<'source, R, S> DiagnosticRenderer<'source, R, S>
where
    R: ?Sized,
{
    /// Creates an allocation-free renderer for `resolver`.
    pub const fn new(resolver: &'source R) -> Self {
        Self {
            resolver,
            source: DiagnosticSource::Unresolved,
            line_starts: Vec::new(),
            indexed_to: 0,
            span: std::marker::PhantomData,
        }
    }

    /// Renders one error, reusing source and line information from prior calls.
    pub fn render_error(&mut self, error: &ParseError<S>) -> String
    where
        R: SourceResolver<S>,
        S: ChumskySpan,
        S::Offset: fmt::Display,
    {
        self.render(&error.message, &error.span)
            .unwrap_or_else(|| error.to_string())
    }

    /// Renders one warning, reusing source and line information from prior calls.
    pub fn render_warning(&mut self, warning: &ParseWarning<S>) -> String
    where
        R: SourceResolver<S>,
        S: ChumskySpan,
        S::Offset: fmt::Display,
    {
        self.render_with_related(&warning.message, &warning.span, &warning.related)
            .unwrap_or_else(|| warning.to_string())
    }

    /// Resolves and renders one native source span.
    fn render(&mut self, message: &str, span: &S) -> Option<String>
    where
        R: SourceResolver<S>,
    {
        self.render_with_related(message, span, &[])
    }

    /// Resolves one primary span and any related spans into one diagnostic.
    fn render_with_related(
        &mut self,
        message: &str,
        span: &S,
        related: &[RelatedSpan<S>],
    ) -> Option<String>
    where
        R: SourceResolver<S>,
    {
        let range = self.resolver.diagnostic_range(span)?;
        if matches!(self.source, DiagnosticSource::Unresolved) {
            let resolver: &'source R = self.resolver;
            self.source = <R as SourceResolver<S>>::full_source(resolver)
                .map_or(DiagnosticSource::Unavailable, DiagnosticSource::Resolved);
        }
        let resolver = self.resolver;
        let DiagnosticSource::Resolved(source) = &self.source else {
            return None;
        };
        let line_starts = &mut self.line_starts;
        let indexed_to = &mut self.indexed_to;
        let primary = diagnostic_lines(&range, source, line_starts, indexed_to)?;
        let mut related_lines = Vec::with_capacity(related.len());
        for related_span in related {
            let related_range = resolver.diagnostic_range(&related_span.span)?;
            let lines = diagnostic_lines(&related_range, source, line_starts, indexed_to)?;
            related_lines.push((related_span, lines));
        }
        let mut gutter_width = primary.line_number.to_string().len();
        for (_, lines) in &related_lines {
            gutter_width = gutter_width.max(lines.line_number.to_string().len());
        }
        let mut rendered = render_primary_diagnostic(message, &primary, source, gutter_width);
        for (related_span, lines) in related_lines {
            rendered.push('\n');
            rendered.push_str(&render_related_diagnostic(
                &related_span.message,
                &lines,
                source,
                gutter_width,
            ));
        }
        Some(rendered)
    }
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
    /// Use [`DiagnosticRenderer`] to reuse source and line information when
    /// rendering more than one diagnostic.
    pub fn diagnostic<R>(&self, resolver: &R) -> String
    where
        R: SourceResolver<S> + ?Sized,
    {
        DiagnosticRenderer::new(resolver).render_error(self)
    }
}

/// A secondary source location attached to a parser advisory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelatedSpan<S = Span> {
    /// Half-open native span in the original input.
    pub span: S,
    /// Human-readable note explaining the significance of this location.
    pub message: String,
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
    /// Additional source locations that give context for this advisory.
    pub related: Vec<RelatedSpan<S>>,
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
    ///
    /// Use [`DiagnosticRenderer`] to reuse source and line information when
    /// rendering more than one diagnostic.
    pub fn diagnostic<R>(&self, resolver: &R) -> String
    where
        R: SourceResolver<S> + ?Sized,
    {
        DiagnosticRenderer::new(resolver).render_warning(self)
    }
}

/// Source line, caret, and gutter information for one byte span.
struct DiagnosticLines {
    line_start: usize,
    line_end: usize,
    line_number: usize,
    column: usize,
    prefix: String,
    marker: String,
}

/// Computes source line and caret information for one byte span.
fn diagnostic_lines(
    span: &Span,
    source: &str,
    line_starts: &mut Vec<usize>,
    indexed_to: &mut usize,
) -> Option<DiagnosticLines> {
    let start = span.start;
    let end = span.end;
    if start > end
        || end > source.len()
        || !source.is_char_boundary(start)
        || !source.is_char_boundary(end)
    {
        return None;
    }

    if line_starts.is_empty() {
        line_starts.push(0);
    }
    if start > *indexed_to {
        let bytes = source.as_bytes();
        let mut cursor = *indexed_to;
        while cursor < start {
            match bytes[cursor] {
                b'\r' => {
                    cursor += 1;
                    if cursor < bytes.len() && bytes[cursor] == b'\n' {
                        cursor += 1;
                    }
                    line_starts.push(cursor);
                }
                b'\n' => {
                    cursor += 1;
                    line_starts.push(cursor);
                }
                _ => cursor += 1,
            }
        }
        *indexed_to = cursor;
    }
    let line_index = line_starts.partition_point(|line_start| *line_start <= start) - 1;
    let line_start = line_starts[line_index];
    let line_end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| start + offset);
    let line_number = line_index + 1;
    let column = source[line_start..start].chars().count() + 1;
    let prefix = source[line_start..start]
        .chars()
        .map(|character| if character == '\t' { '\t' } else { ' ' })
        .collect::<String>();
    let highlight_end = end.min(line_end);
    let highlight_width = source[start..highlight_end].chars().count().max(1);
    let marker = "^".repeat(highlight_width);
    Some(DiagnosticLines {
        line_start,
        line_end,
        line_number,
        column,
        prefix,
        marker,
    })
}

/// Formats one primary diagnostic block.
fn render_primary_diagnostic(
    message: &str,
    lines: &DiagnosticLines,
    source: &str,
    gutter_width: usize,
) -> String {
    let DiagnosticLines {
        line_start,
        line_end,
        line_number,
        column,
        prefix,
        marker,
    } = lines;
    format!(
        "{line_number}:{column}: {message}\n{empty:>gutter_width$} |\n{line_number:>gutter_width$} | {}\n{empty:>gutter_width$} | {prefix}{marker}",
        &source[*line_start..*line_end],
        empty = "",
    )
}

/// Formats one related-span block attached to a diagnostic.
fn render_related_diagnostic(
    message: &str,
    lines: &DiagnosticLines,
    source: &str,
    gutter_width: usize,
) -> String {
    let DiagnosticLines {
        line_start,
        line_end,
        line_number,
        column,
        prefix,
        marker,
    } = lines;
    let source_line = &source[*line_start..*line_end];
    format!(
        "{empty:>gutter_width$} |\n{empty:>gutter_width$} = note: {message} at {line_number}:{column}\n{line_number}:{column}: {source_line}\n{empty:>gutter_width$} |\n{line_number:>gutter_width$} | {source_line}\n{empty:>gutter_width$} | {prefix}{marker}",
        empty = "",
    )
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

/// Converts a Chumsky `Rich` error into a [`ParseError`] preserving its native span.
pub fn chumsky_error<S>(error: &Rich<'_, char, S>) -> ParseError<S>
where
    S: Clone,
{
    ParseError {
        kind: ErrorKind::InvalidMusic,
        message: rich_error_message(error),
        span: error.span().clone(),
    }
}

/// Converts a line-local Chumsky error to a classified parse error.
pub fn chumsky_error_with_kind_and_offset(
    error: &Rich<'_, char, SimpleSpan<usize>>,
    kind: ErrorKind,
    offset: usize,
) -> ParseError {
    let span = error.span();
    ParseError {
        kind,
        message: rich_error_message(error),
        span: span.start.saturating_add(offset)..span.end.saturating_add(offset),
    }
}

/// Selects the first classified parser error or synthesizes a fallback.
pub fn first_parser_error(
    errors: &[Rich<'_, char, SimpleSpan<usize>>],
    kind: ErrorKind,
    fallback_message: &str,
    source_len: usize,
) -> ParseError {
    errors.first().map_or_else(
        || error(kind, fallback_message, 0, source_len),
        |error| chumsky_error_with_kind_and_offset(error, kind, 0),
    )
}

/// Formats Chumsky's reason and production contexts without internal spans.
pub fn rich_error_message<S>(error: &Rich<'_, char, S>) -> String {
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

/// Constructs a [`ParseError`] with byte-offset range `start..end`.
pub fn error(kind: ErrorKind, message: &str, start: usize, end: usize) -> ParseError {
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
    fn diagnostic_renderer_preserves_unicode_tabs_and_line_endings() {
        let source = "α\tb\r\nnext\n";
        let mut renderer = DiagnosticRenderer::new(source);

        let tabbed = error(ErrorKind::InvalidMusic, "tabbed", 3, 4);
        let diagnostic = renderer.render_error(&tabbed);
        assert!(diagnostic.starts_with("1:3: tabbed"), "{diagnostic}");
        assert!(diagnostic.contains("1 | α\tb"), "{diagnostic}");
        assert!(diagnostic.contains("|  \t^"), "{diagnostic}");

        let multiline = error(ErrorKind::InvalidMusic, "multiline", 0, 8);
        let diagnostic = renderer.render_error(&multiline);
        assert!(diagnostic.starts_with("1:1: multiline"), "{diagnostic}");
        assert!(diagnostic.ends_with("| ^^^"), "{diagnostic}");

        let end = error(ErrorKind::InvalidMusic, "end", source.len(), source.len());
        let diagnostic = renderer.render_error(&end);
        assert!(diagnostic.starts_with("3:1: end"), "{diagnostic}");
        assert!(diagnostic.contains("3 | \n"), "{diagnostic}");
        assert!(diagnostic.ends_with("| ^"), "{diagnostic}");

        let invalid_boundary = error(ErrorKind::InvalidMusic, "invalid", 1, 2);
        assert_eq!(
            renderer.render_error(&invalid_boundary),
            invalid_boundary.to_string()
        );
    }

    #[test]
    fn diagnostic_renderer_line_numbers_match_across_line_endings() {
        let lf = "X:1\nK:C\nCDEF |\n";
        let crlf = "X:1\r\nK:C\r\nCDEF |\r\n";
        let cr = "X:1\rK:C\rCDEF |\r";

        let mut lf_renderer = DiagnosticRenderer::new(lf);
        let mut crlf_renderer = DiagnosticRenderer::new(crlf);
        let mut cr_renderer = DiagnosticRenderer::new(cr);

        let lf_err = lf_renderer.render_error(&error(ErrorKind::InvalidMusic, "x", 15, 15));
        let crlf_err = crlf_renderer.render_error(&error(ErrorKind::InvalidMusic, "x", 18, 18));
        let cr_err = cr_renderer.render_error(&error(ErrorKind::InvalidMusic, "x", 15, 15));
        assert!(lf_err.starts_with("4:1: x"), "{lf_err}");
        assert!(crlf_err.starts_with("4:1: x"), "{crlf_err}");
        assert!(cr_err.starts_with("4:1: x"), "{cr_err}");
        assert_eq!(lf_renderer.line_starts, vec![0, 4, 8, 15]);
        assert_eq!(crlf_renderer.line_starts, vec![0, 5, 10, 18]);
        assert_eq!(cr_renderer.line_starts, vec![0, 4, 8, 15]);

        let mut renderer = DiagnosticRenderer::new(cr);
        let second_line = error(ErrorKind::InvalidMusic, "second", 4, 5);
        let diagnostic = renderer.render_error(&second_line);
        assert!(diagnostic.starts_with("2:1: second"), "{diagnostic}");
    }

    #[test]
    fn diagnostic_renderer_does_not_double_count_crlf_when_span_starts_at_lf() {
        let crlf = "X:1\r\nK:C\r\nCDEF |\r\n";
        let mut renderer = DiagnosticRenderer::new(crlf);

        // Diagnostic starting at \n of line 1 (byte 4).
        let first = error(ErrorKind::InvalidMusic, "eol", 4, 4);
        let diag1 = renderer.render_error(&first);
        assert!(diag1.starts_with("1:5: eol"), "{diag1}");

        // Subsequent diagnostic on line 3 (byte 10).
        let third_line = error(ErrorKind::InvalidMusic, "third", 10, 11);
        let diag2 = renderer.render_error(&third_line);
        assert!(diag2.starts_with("3:1: third"), "{diag2}");

        assert_eq!(renderer.line_starts, vec![0, 5, 10]);
    }
}
