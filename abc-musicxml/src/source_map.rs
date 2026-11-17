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

//! Source line indexing, provenance spans, and XML 1.0 comment sanitization.

use std::fmt;

use abc_parser::SimpleSpan;

/// 1-based line and 1-based Unicode scalar (`char`) column in an ABC source file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineColumn {
    /// 1-based physical line number.
    pub line: usize,
    /// 1-based Unicode scalar (`char`) column offset within the line.
    pub column: usize,
}

impl LineColumn {
    /// Creates a new 1-based line and column coordinate.
    #[must_use]
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// Source provenance attached to an IR node for Level 1 and Level 2 XML comments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceProvenance {
    /// Byte span of the originating physical ABC line (excluding line terminators).
    line_span: SimpleSpan<usize>,
    /// Additional physical line spans associated with this construct (such as
    /// `+:` continuations on header fields or `w:` / `s:` lines on a measure).
    extra_line_spans: Option<Box<[SimpleSpan<usize>]>>,
    /// Exact byte span of the originating AST token or element.
    element_span: SimpleSpan<usize>,
    /// Optional secondary byte span (for example, when an element combines two tokens).
    secondary_span: Option<SimpleSpan<usize>>,
    /// Whether the Level 2 verbose comment should include the `: <snippet>` suffix.
    include_snippet: bool,
}

impl SourceProvenance {
    /// Creates provenance with a physical line span and an element token span.
    #[must_use]
    pub const fn new(line_span: SimpleSpan<usize>, element_span: SimpleSpan<usize>) -> Self {
        Self {
            line_span,
            extra_line_spans: None,
            element_span,
            secondary_span: None,
            include_snippet: true,
        }
    }

    /// Creates provenance for a multi-token container element (such as `<measure>`)
    /// that omits the trailing `: <token_snippet>` in Level 2 verbose comments.
    #[must_use]
    pub const fn container(line_span: SimpleSpan<usize>, element_span: SimpleSpan<usize>) -> Self {
        Self {
            line_span,
            extra_line_spans: None,
            element_span,
            secondary_span: None,
            include_snippet: false,
        }
    }

    /// Creates provenance where the element span is the entire line span.
    #[must_use]
    pub const fn from_line(line_span: SimpleSpan<usize>) -> Self {
        Self::new(line_span, line_span)
    }

    /// Attaches a secondary byte span to this provenance record.
    #[must_use]
    pub const fn with_secondary_span(mut self, span: SimpleSpan<usize>) -> Self {
        self.secondary_span = Some(span);
        self
    }

    /// Attaches additional associated physical line spans (such as `+:` or `w:` lines).
    #[must_use]
    pub fn with_extra_line_spans(mut self, spans: Vec<SimpleSpan<usize>>) -> Self {
        self.extra_line_spans = (!spans.is_empty()).then(|| spans.into_boxed_slice());
        self
    }

    /// Adds an additional physical line span if not already present.
    pub fn add_extra_line_span(&mut self, span: SimpleSpan<usize>) {
        let current = self.extra_line_spans();
        if current.contains(&span) {
            return;
        }
        let mut expanded = Vec::with_capacity(current.len() + 1);
        expanded.extend_from_slice(current);
        expanded.push(span);
        self.extra_line_spans = Some(expanded.into_boxed_slice());
    }

    /// Configures whether the Level 2 comment includes the `: <snippet>` suffix.
    #[must_use]
    pub const fn with_snippet(mut self, include_snippet: bool) -> Self {
        self.include_snippet = include_snippet;
        self
    }

    /// Returns the primary physical line byte span.
    #[must_use]
    pub const fn line_span(&self) -> SimpleSpan<usize> {
        self.line_span
    }

    /// Returns any additional associated physical line spans.
    #[must_use]
    pub fn extra_line_spans(&self) -> &[SimpleSpan<usize>] {
        self.extra_line_spans.as_deref().unwrap_or(&[])
    }

    /// Returns the element token byte span.
    #[must_use]
    pub const fn element_span(&self) -> SimpleSpan<usize> {
        self.element_span
    }

    /// Returns the optional secondary byte span.
    #[must_use]
    pub const fn secondary_span(&self) -> Option<SimpleSpan<usize>> {
        self.secondary_span
    }

    /// Returns `true` if Level 2 comments should include the trailing `: <snippet>`.
    #[must_use]
    pub const fn include_snippet(&self) -> bool {
        self.include_snippet
    }
}

/// Indexes line starts across LF (`\n`), CRLF (`\r\n`), and bare CR (`\r`) line
/// endings and formats Level 1 and Level 2 source provenance comments.
#[derive(Clone, Debug)]
pub struct SourceMap<'src> {
    source: &'src str,
    source_name: &'src str,
    line_starts: Vec<usize>,
}

impl<'src> SourceMap<'src> {
    /// Builds a line index over `source` with `source_name` for comment prefixes.
    #[must_use]
    pub fn new(source: &'src str, source_name: &'src str) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = vec![0];
        let mut index = 0;

        while index < bytes.len() {
            match bytes[index] {
                b'\n' => {
                    line_starts.push(index + 1);
                    index += 1;
                }
                b'\r' => {
                    if index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
                        line_starts.push(index + 2);
                        index += 2;
                    } else {
                        line_starts.push(index + 1);
                        index += 1;
                    }
                }
                _ => {
                    index += 1;
                }
            }
        }

        Self {
            source,
            source_name,
            line_starts,
        }
    }

    /// Returns the full underlying ABC source text.
    #[must_use]
    pub const fn source(&self) -> &'src str {
        self.source
    }

    /// Returns the display name of the source file (e.g. `"example.abc"` or `"<stdin>"`).
    #[must_use]
    pub const fn source_name(&self) -> &'src str {
        self.source_name
    }

    /// Returns the number of indexed lines in the source.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Returns the 0-based line index containing `byte_offset`.
    #[must_use]
    pub fn line_index(&self, byte_offset: usize) -> usize {
        let clamped = byte_offset.min(self.source.len());
        self.line_starts
            .partition_point(|&start| start <= clamped)
            .saturating_sub(1)
    }

    /// Returns the 0-based line index containing `byte_offset`, starting from `hint`
    /// to optimize sequential forward lookups.
    #[must_use]
    pub fn line_index_with_hint(&self, byte_offset: usize, hint: usize) -> usize {
        let clamped = byte_offset.min(self.source.len());
        if let Some(&start) = self.line_starts.get(hint) {
            let next_start = self
                .line_starts
                .get(hint + 1)
                .copied()
                .unwrap_or(usize::MAX);
            if clamped >= start && clamped < next_start {
                return hint;
            }
            if let Some(&next) = self.line_starts.get(hint + 1) {
                let next_next = self
                    .line_starts
                    .get(hint + 2)
                    .copied()
                    .unwrap_or(usize::MAX);
                if clamped >= next && clamped < next_next {
                    return hint + 1;
                }
            }
        }
        self.line_index(byte_offset)
    }

    /// Returns the 1-based physical line number containing `byte_offset`.
    #[must_use]
    pub fn line_number(&self, byte_offset: usize) -> usize {
        self.line_index(byte_offset) + 1
    }

    /// Returns the byte span of `line_index` (0-based), excluding any trailing
    /// `\r\n`, `\n`, or `\r` line terminator.
    #[must_use]
    pub fn line_span(&self, line_index: usize) -> SimpleSpan<usize> {
        let Some(&start) = self.line_starts.get(line_index) else {
            let end = self.source.len();
            return SimpleSpan::from(end..end);
        };
        let raw_end = self
            .line_starts
            .get(line_index + 1)
            .copied()
            .unwrap_or(self.source.len());
        let bytes = self.source.as_bytes();
        let mut end = raw_end;
        if end > start && bytes[end - 1] == b'\n' {
            end -= 1;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
        } else if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        SimpleSpan::from(start..end)
    }

    /// Returns the raw text of `line_index` (0-based), excluding any trailing
    /// `\r\n`, `\n`, or `\r` line terminator.
    #[must_use]
    pub fn line_text(&self, line_index: usize) -> &'src str {
        let span = self.line_span(line_index);
        &self.source[span.start..span.end]
    }

    /// Converts a UTF-8 byte offset into a 1-based [`LineColumn`] using `hint` as a starting line index.
    #[must_use]
    pub fn line_column_with_hint(&self, byte_offset: usize, hint: usize) -> LineColumn {
        let clamped = self.clamp_to_char_boundary(byte_offset);
        let line_idx = self.line_index_with_hint(clamped, hint);
        let line_start = self.line_starts[line_idx];
        let prefix = &self.source[line_start..clamped];
        let column = prefix.chars().count() + 1;
        LineColumn::new(line_idx + 1, column)
    }

    /// Converts a UTF-8 byte offset into a 1-based [`LineColumn`].
    #[must_use]
    pub fn line_column(&self, byte_offset: usize) -> LineColumn {
        self.line_column_with_hint(byte_offset, 0)
    }

    /// Extracts the source slice for `span`, clamping boundaries to valid UTF-8
    /// character boundaries and stripping any trailing line terminators.
    #[must_use]
    pub fn slice(&self, span: SimpleSpan<usize>) -> &'src str {
        let start = self.clamp_to_char_boundary(span.start);
        let end = self.clamp_to_char_boundary(span.end).max(start);
        let raw = &self.source[start..end];
        raw.trim_end_matches(['\r', '\n'])
    }

    /// Writes a Level 1 (`AnnotationLevel::Line`) comment body for `line_index` (0-based)
    /// directly to `output`, avoiding intermediate string allocations.
    pub fn write_line_comment<W: fmt::Write>(
        &self,
        output: &mut W,
        line_index: usize,
    ) -> fmt::Result {
        let line_number = line_index + 1;
        let text = self.line_text(line_index);
        if text.chars().any(is_valid_xml_char) {
            write!(output, "{}:{line_number}: ", self.source_name)?;
            write_sanitized_xml_comment(output, text)?;
        } else {
            write!(output, "{}:{line_number}:", self.source_name)?;
        }
        Ok(())
    }

    /// Formats a Level 1 (`AnnotationLevel::Line`) comment body for `line_index` (0-based).
    ///
    /// Format: `<source_name>:<line_number>: <sanitized_line_text>`.
    #[must_use]
    pub fn format_line_comment(&self, line_index: usize) -> String {
        let mut out = String::new();
        let _ = self.write_line_comment(&mut out, line_index);
        out
    }

    /// Formats a Level 1 (`AnnotationLevel::Line`) comment body for the physical
    /// line containing `line_span.start`.
    #[must_use]
    pub fn format_line_span_comment(&self, line_span: SimpleSpan<usize>) -> String {
        let line_index = self.line_index(line_span.start);
        self.format_line_comment(line_index)
    }

    /// Writes a Level 2 (`AnnotationLevel::Verbose`) comment body directly to `output`,
    /// avoiding intermediate string allocations.
    pub fn write_verbose_comment<W: fmt::Write>(
        &self,
        output: &mut W,
        provenance: &SourceProvenance,
    ) -> fmt::Result {
        self.write_verbose_comment_with_hint(output, provenance, None)
    }

    /// Writes a Level 2 (`AnnotationLevel::Verbose`) comment body directly to `output`
    /// using an optional line hint to optimize sequential lookups.
    pub fn write_verbose_comment_with_hint<W: fmt::Write>(
        &self,
        output: &mut W,
        provenance: &SourceProvenance,
        hint: Option<usize>,
    ) -> fmt::Result {
        let span = self.trimmed_span(provenance.element_span);
        let start = match hint {
            Some(h) => self.line_column_with_hint(span.start, h),
            None => self.line_column(span.start),
        };
        let end = self.line_column_with_hint(span.end, start.line.saturating_sub(1));

        write!(
            output,
            "@{}:{}..{}:{} [{}..{}]",
            start.line, start.column, end.line, end.column, span.start, span.end
        )?;

        if let Some(secondary) = provenance.secondary_span {
            let sec = self.trimmed_span(secondary);
            let sec_start = self.line_column_with_hint(sec.start, end.line.saturating_sub(1));
            let sec_end = self.line_column_with_hint(sec.end, sec_start.line.saturating_sub(1));
            write!(
                output,
                " + @{}:{}..{}:{} [{}..{}]",
                sec_start.line, sec_start.column, sec_end.line, sec_end.column, sec.start, sec.end
            )?;
        }

        if provenance.include_snippet {
            let slice = self.slice(span);
            if slice.chars().any(is_valid_xml_char) {
                output.write_str(": ")?;
                write_sanitized_xml_comment(output, slice)?;
            }
        }

        Ok(())
    }

    /// Formats a Level 2 (`AnnotationLevel::Verbose`) Hybrid Option D comment body
    /// for `provenance`.
    ///
    /// Format: `@<start_line>:<start_col>..<end_line>:<end_col> [<byte_start>..<byte_end>]: <snippet>`
    /// (or without `: <snippet>` when `provenance.include_snippet()` is `false`).
    #[must_use]
    pub fn format_verbose_comment(&self, provenance: &SourceProvenance) -> String {
        let mut out = String::new();
        let _ = self.write_verbose_comment(&mut out, provenance);
        out
    }

    fn trimmed_span(&self, span: SimpleSpan<usize>) -> SimpleSpan<usize> {
        let start = self.clamp_to_char_boundary(span.start);
        let mut end = self.clamp_to_char_boundary(span.end).max(start);
        let bytes = self.source.as_bytes();
        while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
            end -= 1;
        }
        SimpleSpan::from(start..end)
    }

    fn clamp_to_char_boundary(&self, offset: usize) -> usize {
        let mut idx = offset.min(self.source.len());
        while idx > 0 && !self.source.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }
}

/// Sanitizes arbitrary text for embedding inside an XML 1.0 comment (`<!-- ... -->`)
/// directly into a [`fmt::Write`] sink, avoiding intermediate allocations.
///
/// Follows the lossless percent-encoding rules specified in §3.4:
/// 1. Strips illegal XML 1.0 control characters (`U+0000`–`U+0008`, `U+000B`,
///    `U+000C`, `U+000E`–`U+001F` other than `\t`, `\n`, `\r`).
/// 2. Encodes every literal `%` character as `%25`.
/// 3. For any run of consecutive `-` characters of length `> 1`, keeps the
///    leading `-` literal and encodes every subsequent `-` in the run as `%2D`.
/// 4. If the content ends in a single `-`, encodes that trailing `-` as `%2D`.
pub fn write_sanitized_xml_comment<W: fmt::Write>(output: &mut W, raw: &str) -> fmt::Result {
    write_sanitized_comment_dispatch(output, raw, true)
}

/// Sanitizes arbitrary text for embedding inside an XML 1.0 comment (`<!-- ... -->`)
/// directly into a [`fmt::Write`] sink, preserving literal `%` characters.
///
/// Follows XML 1.0 §2.5 rules without percent-encoding `%`:
/// 1. Strips illegal XML 1.0 control characters (`U+0000`–`U+0008`, `U+000B`,
///    `U+000C`, `U+000E`–`U+001F` other than `\t`, `\n`, `\r`).
/// 2. For any run of consecutive `-` characters of length `> 1`, keeps the
///    leading `-` literal and encodes every subsequent `-` in the run as `%2D`.
/// 3. If the content ends in a single `-`, encodes that trailing `-` as `%2D`.
///
/// # Errors
///
/// Returns [`fmt::Error`] if writing to `output` fails.
pub fn write_sanitized_xml_comment_text<W: fmt::Write>(output: &mut W, raw: &str) -> fmt::Result {
    write_sanitized_comment_dispatch(output, raw, false)
}

fn write_hyphen_run<W: fmt::Write>(output: &mut W, count: usize) -> fmt::Result {
    if count == 0 {
        return Ok(());
    }
    output.write_char('-')?;
    for _ in 1..count {
        output.write_str("%2D")?;
    }
    Ok(())
}

fn write_sanitized_comment_dispatch<W: fmt::Write>(
    output: &mut W,
    raw: &str,
    encode_percent: bool,
) -> fmt::Result {
    let mut pending_hyphens = 0_usize;
    let mut chunk_start = 0_usize;

    for (idx, ch) in raw.char_indices() {
        let special = !is_valid_xml_char(ch) || ch == '-' || (ch == '%' && encode_percent);
        if !special && pending_hyphens == 0 {
            continue;
        }
        if chunk_start < idx {
            output.write_str(&raw[chunk_start..idx])?;
        }
        if !is_valid_xml_char(ch) {
            chunk_start = idx + ch.len_utf8();
            continue;
        }
        if ch == '-' {
            pending_hyphens += 1;
            chunk_start = idx + 1;
            continue;
        }
        if pending_hyphens > 0 {
            write_hyphen_run(output, pending_hyphens)?;
            pending_hyphens = 0;
        }
        if ch == '%' && encode_percent {
            output.write_str("%25")?;
            chunk_start = idx + 1;
        } else {
            chunk_start = idx;
        }
    }

    if chunk_start < raw.len() {
        output.write_str(&raw[chunk_start..])?;
    }
    if pending_hyphens == 1 {
        output.write_str("%2D")?;
    } else {
        write_hyphen_run(output, pending_hyphens)?;
    }
    Ok(())
}

/// Sanitizes arbitrary text for embedding inside an XML 1.0 comment (`<!-- ... -->`).
///
/// Follows the lossless percent-encoding rules specified in §3.4:
/// 1. Strips illegal XML 1.0 control characters (`U+0000`–`U+0008`, `U+000B`,
///    `U+000C`, `U+000E`–`U+001F` other than `\t`, `\n`, `\r`).
/// 2. Encodes every literal `%` character as `%25`.
/// 3. For any run of consecutive `-` characters of length `> 1`, keeps the
///    leading `-` literal and encodes every subsequent `-` in the run as `%2D`.
/// 4. If the content ends in a single `-`, encodes that trailing `-` as `%2D`.
#[must_use]
pub fn sanitize_xml_comment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let _ = write_sanitized_xml_comment(&mut out, raw);
    out
}

/// Decodes a comment body previously encoded by [`sanitize_xml_comment`].
#[must_use]
pub fn decode_xml_comment(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut out = String::with_capacity(encoded.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &bytes[index + 1..index + 3];
            if hex.eq_ignore_ascii_case(b"25") {
                out.push('%');
                index += 3;
                continue;
            }
            if hex.eq_ignore_ascii_case(b"2D") {
                out.push('-');
                index += 3;
                continue;
            }
        }
        let ch = encoded[index..].chars().next().unwrap_or('\0');
        out.push(ch);
        index += ch.len_utf8();
    }

    out
}

const fn is_valid_xml_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000D}'
            | '\u{0020}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
    )
}

#[cfg(test)]
mod tests {
    use abc_parser::SimpleSpan;

    use super::LineColumn;
    use super::SourceMap;
    use super::SourceProvenance;
    use super::decode_xml_comment;
    use super::sanitize_xml_comment;

    #[test]
    fn indexes_lf_crlf_and_bare_cr_line_endings() {
        let src = "X:1\nT:Test\r\nM:4/4\rK:C";
        let map = SourceMap::new(src, "mixed.abc");

        assert_eq!(map.line_count(), 4);
        assert_eq!(map.line_text(0), "X:1");
        assert_eq!(map.line_text(1), "T:Test");
        assert_eq!(map.line_text(2), "M:4/4");
        assert_eq!(map.line_text(3), "K:C");

        assert_eq!(map.line_span(0), SimpleSpan::from(0..3));
        assert_eq!(map.line_span(1), SimpleSpan::from(4..10));
        assert_eq!(map.line_span(2), SimpleSpan::from(12..17));
        assert_eq!(map.line_span(3), SimpleSpan::from(18..21));

        assert_eq!(map.line_column(0), LineColumn::new(1, 1));
        assert_eq!(map.line_column(3), LineColumn::new(1, 4));
        assert_eq!(map.line_column(4), LineColumn::new(2, 1));
        assert_eq!(map.line_column(12), LineColumn::new(3, 1));
        assert_eq!(map.line_column(18), LineColumn::new(4, 1));
    }

    #[test]
    fn clamps_mid_codepoint_and_out_of_bounds_spans() {
        // 'é' is 2 bytes (5..7), '—' is 3 bytes (8..11), followed by "\r\n" (11..13)
        let src = "T:Café —\r\nK:C";
        let map = SourceMap::new(src, "utf8.abc");

        // Byte 6 is inside 'é' (5..7); byte 9 is inside '—' (8..11).
        assert_eq!(map.line_column(6), LineColumn::new(1, 6));
        assert_eq!(map.line_column(9), LineColumn::new(1, 8));
        assert_eq!(map.line_column(11), LineColumn::new(1, 9));

        // Slice clamping backs up to char boundary and strips trailing "\r\n".
        assert_eq!(map.slice(SimpleSpan::from(6..999)), "é —\r\nK:C");
        assert_eq!(map.slice(SimpleSpan::from(0..13)), "T:Café —");

        // Out-of-bounds line index returns an empty span at EOF.
        let len = src.len();
        assert_eq!(map.line_span(99), SimpleSpan::from(len..len));
        assert_eq!(map.line_text(99), "");
    }

    #[test]
    fn sanitizes_and_roundtrips_xml_comments() {
        let cases = [
            ("plain text", "plain text"),
            ("100% pure", "100%25 pure"),
            ("single-hyphen", "single-hyphen"),
            ("double--hyphen", "double-%2Dhyphen"),
            ("triple---hyphen", "triple-%2D%2Dhyphen"),
            ("trailing-", "trailing%2D"),
            ("trailing--", "trailing-%2D"),
            ("trailing---", "trailing-%2D%2D"),
            ("-", "%2D"),
            ("--", "-%2D"),
            ("---", "-%2D%2D"),
            (
                "% --- Begin Voice 1 ---",
                "%25 -%2D%2D Begin Voice 1 -%2D%2D",
            ),
            ("literal %2D and %25", "literal %252D and %2525"),
        ];

        for (raw, expected) in cases {
            let encoded = sanitize_xml_comment(raw);
            assert_eq!(encoded, expected, "failed encoding for {raw:?}");
            assert!(
                !encoded.contains("--"),
                "encoded comment must never contain `--`: {encoded:?}"
            );
            assert!(
                !encoded.ends_with('-'),
                "encoded comment must never end with `-`: {encoded:?}"
            );
            let decoded = decode_xml_comment(&encoded);
            assert_eq!(decoded, raw, "failed round-trip for {raw:?}");
        }

        // Illegal XML 1.0 control characters (other than \t, \n, \r) are stripped.
        assert_eq!(
            sanitize_xml_comment("A\x00B\x07C\x08D\tE\x0BF\x1FG"),
            "ABCD\tEFG"
        );
    }

    #[test]
    fn formats_line_and_verbose_provenance_branches() {
        let src = "% -- hdr\n\nC- D |\r\n";
        let map = SourceMap::new(src, "t.abc");

        // Sanitized non-empty line vs. empty line (omits trailing space).
        assert_eq!(map.format_line_comment(0), "t.abc:1: %25 -%2D hdr");
        assert_eq!(map.format_line_comment(1), "t.abc:2:");

        // Container provenance omits `: <snippet>`, and trailing `\r\n` is trimmed from span end.
        let container =
            SourceProvenance::container(SimpleSpan::from(10..16), SimpleSpan::from(10..18));
        assert_eq!(map.format_verbose_comment(&container), "@3:1..3:7 [10..16]");

        // Token provenance with secondary span and sanitized trailing `-` in snippet.
        let tied = SourceProvenance::new(SimpleSpan::from(10..16), SimpleSpan::from(10..12))
            .with_secondary_span(SimpleSpan::from(13..14));
        assert_eq!(
            map.format_verbose_comment(&tied),
            "@3:1..3:3 [10..12] + @3:4..3:5 [13..14]: C%2D"
        );
    }

    #[test]
    fn line_index_with_hint_matches_line_index_exhaustively() {
        let src = "X:1\nT:Test\nM:4/4\nK:C\nC D E F | G A B c |\n";
        let map = SourceMap::new(src, "t.abc");
        for offset in 0..=src.len() + 5 {
            let expected = map.line_index(offset);
            for hint in 0..=map.line_count() + 2 {
                assert_eq!(
                    map.line_index_with_hint(offset, hint),
                    expected,
                    "mismatch for offset {offset} with hint {hint}"
                );
            }
        }
    }
}
