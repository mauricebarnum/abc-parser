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
    /// Creates a new line index for `source`.
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

    /// Returns a reference to the indexed document source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the byte offsets where each line starts.
    pub fn line_starts(&self) -> &[usize] {
        &self.line_starts
    }

    /// Converts an LSP range into a UTF-8 byte offset range.
    pub fn byte_range(
        &self,
        range: LspRange,
        encoding: &PositionEncodingKind,
    ) -> Option<Range<usize>> {
        Some(self.byte_offset(range.start, encoding)?..self.byte_offset(range.end, encoding)?)
    }

    /// Converts a UTF-8 byte offset range into an LSP range.
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

    /// Returns the full LSP range spanning the entire document source.
    ///
    /// # Panics
    ///
    /// Panics if the entire document text range cannot be converted into an
    /// LSP range.
    pub fn whole_range(&self, encoding: &PositionEncodingKind) -> LspRange {
        self.lsp_range(0..self.source.len(), encoding)
            .expect("complete source is always on character boundaries")
    }

    /// Translates a UTF-8 byte offset into an LSP position.
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
                [.., b'\n' | b'\r'] => end - 1,
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
        let start = self.line_starts[line];
        let Some(next) = self.line_starts.get(line + 1).copied() else {
            return self.source.len();
        };
        match self.source.as_bytes()[start..next] {
            [.., b'\r', b'\n'] => next - 2,
            [.., b'\n' | b'\r'] => next - 1,
            _ => next,
        }
    }
}

/// Error returned when an LSP-ranged text edit cannot be applied to the
/// server's text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEditError {
    /// The range did not translate to a valid byte span for the indexed
    /// source (e.g. out-of-bounds line or character).
    InvalidRange,
}

/// Applies a single ranged text edit to `text`, returning the new text.
///
/// The LSP [`LspRange`] is converted to a UTF-8 byte range via `index`
/// using the negotiated `encoding`. When `range` refers to positions that
/// do not exist in `text` (the client is out of sync), the function
/// returns [`TextEditError::InvalidRange`] and leaves `text` untouched.
///
/// Insertions are expressed as a range with equal start and end and a
/// non-empty `new_text`; deletions are the opposite.
pub fn apply_text_edit(
    text: &str,
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    range: LspRange,
    new_text: &str,
) -> Result<String, TextEditError> {
    let byte_range = index
        .byte_range(range, encoding)
        .ok_or(TextEditError::InvalidRange)?;
    let mut result = text.to_owned();
    result.replace_range(byte_range, new_text);
    Ok(result)
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

    #[test]
    fn apply_text_edit_inserts_at_point() {
        let index = LineIndex::new("X:1\nK:C\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(0, 3), Position::new(0, 3)),
            " T:Title",
        )
        .expect("insertion at end of first line");
        assert_eq!(result, "X:1 T:Title\nK:C\n");
    }

    #[test]
    fn apply_text_edit_deletes_a_range() {
        let index = LineIndex::new("X:99\nK:C\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(0, 2), Position::new(0, 4)),
            "",
        )
        .expect("deletion");
        assert_eq!(result, "X:\nK:C\n");
    }

    #[test]
    fn apply_text_edit_replaces_across_multiple_lines() {
        let index = LineIndex::new("X:1\nold text\nK:C\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(1, 0), Position::new(2, 0)),
            "T:Replacement\n",
        )
        .expect("multi-line replacement");
        assert_eq!(result, "X:1\nT:Replacement\nK:C\n");
    }

    #[test]
    fn apply_text_edit_edits_at_end_of_file() {
        let index = LineIndex::new("X:1\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(1, 0), Position::new(1, 0)),
            "K:C\nC |\n",
        )
        .expect("edit at EOF");
        assert_eq!(result, "X:1\nK:C\nC |\n");
    }

    #[test]
    fn apply_text_edit_handles_crlf_line_endings() {
        // LSP ranges span the line terminator that separates the end and
        // start positions, so the trailing CRLF is replaced together
        // with the inserted text.
        let index = LineIndex::new("X:1\r\nK:C\r\nC |\r\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(0, 3), Position::new(1, 0)),
            " T:Inserted",
        )
        .expect("edit spanning CRLF terminator");
        assert_eq!(result, "X:1 T:InsertedK:C\r\nC |\r\n");
    }

    #[test]
    fn apply_text_edit_handles_cr_only_line_endings() {
        let index = LineIndex::new("X:1\rK:C\rC |\r".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(0, 3), Position::new(1, 0)),
            " T:Inserted",
        )
        .expect("edit spanning CR terminator");
        assert_eq!(result, "X:1 T:InsertedK:C\rC |\r");
    }

    #[test]
    fn apply_text_edit_handles_utf8_positions() {
        let index = LineIndex::new("X:1\néé\nK:C\n".to_owned());
        // Replace the second 'é' (UTF-8 bytes 6..8) with 'E'. UTF-8
        // positions must land on Unicode char boundaries, so the range
        // spans two 'character' units of the multibyte text.
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF8,
            LspRange::new(Position::new(1, 2), Position::new(1, 4)),
            "E",
        )
        .expect("UTF-8 position round-trip");
        assert_eq!(result, "X:1\néE\nK:C\n");
    }

    #[test]
    fn apply_text_edit_handles_astral_utf16_surrogate_pair() {
        // '𝄞' (U+1D11E) encodes as a UTF-16 surrogate pair (0xD834 0xDD1E).
        let source = "X:1\n𝄞\nK:C\n";
        let index = LineIndex::new(source.to_owned());
        // Replace the astral character (UTF-16 columns 0..2 of line 1).
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(1, 0), Position::new(1, 2)),
            "G clef",
        )
        .expect("astral UTF-16 surrogate pair");
        assert_eq!(result, "X:1\nG clef\nK:C\n");
    }

    #[test]
    fn apply_text_edit_handles_empty_document() {
        let index = LineIndex::new(String::new());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(0, 0), Position::new(0, 0)),
            "X:1\n",
        )
        .expect("insertion into empty document");
        assert_eq!(result, "X:1\n");
    }

    #[test]
    fn apply_text_edit_rejects_invalid_range() {
        let index = LineIndex::new("X:1\nK:C\n".to_owned());
        let result = apply_text_edit(
            index.source(),
            &index,
            &PositionEncodingKind::UTF16,
            LspRange::new(Position::new(5, 0), Position::new(6, 0)),
            "irrelevant",
        );
        assert_eq!(result, Err(TextEditError::InvalidRange));
    }
}
