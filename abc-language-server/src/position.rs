// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Conversion between UTF-8 byte spans and negotiated LSP positions.

use std::ops::Range;

use tower_lsp_server::ls_types::Position;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::Range as LspRange;

/// Indexed source used for all protocol position conversions.
#[derive(Clone, Debug)]
pub struct LineIndex {
    source: String,
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: String) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = vec![0];
        let mut cursor = 0;
        while cursor < bytes.len() {
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
        Self {
            source,
            line_starts,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn byte_range(
        &self,
        range: LspRange,
        encoding: &PositionEncodingKind,
    ) -> Option<Range<usize>> {
        Some(self.byte_offset(range.start, encoding)?..self.byte_offset(range.end, encoding)?)
    }

    pub fn lsp_range(
        &self,
        range: Range<usize>,
        encoding: &PositionEncodingKind,
    ) -> Option<LspRange> {
        Some(LspRange::new(
            self.position(range.start, encoding)?,
            self.position(range.end, encoding)?,
        ))
    }

    pub fn whole_range(&self, encoding: &PositionEncodingKind) -> LspRange {
        self.lsp_range(0..self.source.len(), encoding)
            .expect("complete source is always on character boundaries")
    }

    pub fn position(&self, offset: usize, encoding: &PositionEncodingKind) -> Option<Position> {
        if offset > self.source.len() || !self.source.is_char_boundary(offset) {
            return None;
        }
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.line_starts[line];
        let content_end = self.line_content_end(line);
        let text = &self.source[line_start..offset.min(content_end)];
        let character = if *encoding == PositionEncodingKind::UTF8 {
            text.len()
        } else {
            text.encode_utf16().count()
        };
        Some(Position::new(
            u32::try_from(line).ok()?,
            u32::try_from(character).ok()?,
        ))
    }

    pub(crate) fn line_bounds(&self, offset: usize) -> Option<Range<usize>> {
        if offset > self.source.len() {
            return None;
        }
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let start = self.line_starts[line];
        let end = self.line_content_end(line);
        Some(start..end)
    }

    /// Iterates over physical lines with their byte-offset start.
    ///
    /// Each yielded `&str` is the line content without any trailing line
    /// terminator (`\n`, `\r`, or `\r\n`), matching the column
    /// accounting used by [`Self::position`]. Supports LF, CRLF, and
    /// bare CR line endings per ABC 2.1 §8.1.
    pub fn line_iter(&self) -> impl Iterator<Item = (usize, &str)> {
        let source = self.source.as_str();
        let mut line_starts = self.line_starts.iter().copied().peekable();
        std::iter::from_fn(move || {
            let start = line_starts.next()?;
            let end = line_starts.peek().copied().unwrap_or(source.len());
            let content_end = match source.as_bytes()[start..end] {
                [.., b'\r', b'\n'] => end - 2,
                [.., b'\n'] | [.., b'\r'] => end - 1,
                _ => end,
            };
            Some((start, &source[start..content_end]))
        })
    }

    fn byte_offset(&self, position: Position, encoding: &PositionEncodingKind) -> Option<usize> {
        let line = usize::try_from(position.line).ok()?;
        let start = *self.line_starts.get(line)?;
        let end = self.line_content_end(line);
        let line_text = &self.source[start..end];
        let character = usize::try_from(position.character).ok()?;
        if *encoding == PositionEncodingKind::UTF8 {
            return line_text
                .is_char_boundary(character)
                .then_some(start + character)
                .filter(|offset| *offset <= end);
        }
        if character == 0 {
            return Some(start);
        }
        let mut units = 0;
        for (offset, value) in line_text.char_indices() {
            if units == character {
                return Some(start + offset);
            }
            units += value.len_utf16();
            if units > character {
                return None;
            }
        }
        (units == character).then_some(end)
    }

    fn line_content_end(&self, line: usize) -> usize {
        let Some(next) = self.line_starts.get(line + 1).copied() else {
            return self.source.len();
        };
        let before_newline = next.saturating_sub(1);
        before_newline
            .checked_sub(1)
            .filter(|offset| self.source.as_bytes().get(*offset) == Some(&b'\r'))
            .unwrap_or(before_newline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_utf8_and_utf16_positions() {
        let index = LineIndex::new("a𐐀b\né\n".to_owned());
        assert_eq!(
            index.position(5, &PositionEncodingKind::UTF8),
            Some(Position::new(0, 5))
        );
        assert_eq!(
            index.position(5, &PositionEncodingKind::UTF16),
            Some(Position::new(0, 3))
        );
        assert_eq!(
            index.byte_offset(Position::new(0, 3), &PositionEncodingKind::UTF16),
            Some(5)
        );
        assert_eq!(
            index.byte_offset(Position::new(0, 2), &PositionEncodingKind::UTF16),
            None
        );
    }

    #[test]
    fn excludes_line_endings_from_positionable_line_content() {
        let index = LineIndex::new("A\r\nB".to_owned());
        assert_eq!(
            index.byte_range(
                LspRange::new(Position::new(1, 0), Position::new(1, 1)),
                &PositionEncodingKind::UTF16,
            ),
            Some(3..4)
        );
        assert_eq!(
            index.byte_range(
                LspRange::new(Position::new(0, 2), Position::new(0, 2)),
                &PositionEncodingKind::UTF16,
            ),
            None
        );
    }

    #[test]
    fn line_starts_handle_all_three_line_endings() {
        let lf = LineIndex::new("X:1\nK:C\n".to_owned());
        assert_eq!(lf.line_starts, vec![0, 4, 8]);

        let crlf = LineIndex::new("X:1\r\nK:C\r\n".to_owned());
        assert_eq!(crlf.line_starts, vec![0, 5, 10]);

        let cr = LineIndex::new("X:1\rK:C\r".to_owned());
        assert_eq!(cr.line_starts, vec![0, 4, 8]);

        let crlf_at_end = LineIndex::new("X:1\r\nK:C".to_owned());
        assert_eq!(crlf_at_end.line_starts, vec![0, 5]);

        let cr_at_end = LineIndex::new("X:1\rK:C".to_owned());
        assert_eq!(cr_at_end.line_starts, vec![0, 4]);
    }

    #[test]
    fn line_iter_skips_all_three_line_terminators() {
        let index = LineIndex::new("a\r\nb\nc\rd".to_owned());
        let collected: Vec<&str> = index.line_iter().map(|(_, line)| line).collect();
        assert_eq!(collected, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn position_round_trip_on_cr_only_document() {
        let cr = LineIndex::new("X:1\rK:C\rCDEF |\r".to_owned());
        assert_eq!(cr.line_starts, vec![0, 4, 8, 15]);
        assert_eq!(
            cr.position(8, &PositionEncodingKind::UTF16),
            Some(Position::new(2, 0))
        );
        assert_eq!(
            cr.position(8, &PositionEncodingKind::UTF16)
                .and_then(|pos| cr.byte_offset(pos, &PositionEncodingKind::UTF16)),
            Some(8)
        );
        assert_eq!(
            cr.position(15, &PositionEncodingKind::UTF16),
            Some(Position::new(3, 0))
        );
        assert_eq!(
            cr.byte_offset(Position::new(2, 0), &PositionEncodingKind::UTF16),
            Some(8)
        );
        assert_eq!(
            cr.byte_offset(Position::new(3, 0), &PositionEncodingKind::UTF16),
            Some(15)
        );
    }
}
