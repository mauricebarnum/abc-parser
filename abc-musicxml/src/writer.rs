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

//! Zero-dependency `MusicXML` 4.0 XML writer and provenance comment emitter.

use std::fmt;

use crate::AnnotationLevel;
use crate::source_map::SourceMap;
use crate::source_map::SourceProvenance;
use crate::source_map::write_sanitized_xml_comment_text;

/// Standard XML 1.0 UTF-8 declaration.
pub const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>";

/// Standard `MusicXML` 4.0 Partwise DOCTYPE declaration.
pub const MUSICXML_PARTWISE_DOCTYPE: &str = "<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \"http://www.musicxml.org/dtds/partwise.dtd\">";

/// Precomputed 128-byte whitespace buffer for constant-time slice writes during indentation.
const SPACES: &str = concat!(
    "                                ",
    "                                ",
    "                                ",
    "                                ",
);

/// Deterministic XML writer over [`fmt::Write`] with configurable indentation.
#[derive(Debug)]
pub struct XmlWriter<W> {
    output: W,
    indent_spaces: u8,
    depth: usize,
}

impl<W: fmt::Write> XmlWriter<W> {
    /// Creates a new `XmlWriter` writing to `output` with `indent_spaces` spaces per level.
    #[must_use]
    pub const fn new(output: W, indent_spaces: u8) -> Self {
        Self {
            output,
            indent_spaces,
            depth: 0,
        }
    }

    /// Consumes the writer and returns the underlying [`fmt::Write`] sink.
    #[must_use]
    pub fn into_inner(self) -> W {
        self.output
    }

    /// Returns the current tag nesting depth.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Writes the XML 1.0 declaration and optional `MusicXML` 4.0 Partwise DOCTYPE.
    pub fn write_declaration(&mut self, include_doctype: bool) -> fmt::Result {
        self.output.write_str(XML_DECLARATION)?;
        self.output.write_char('\n')?;
        if include_doctype {
            self.output.write_str(MUSICXML_PARTWISE_DOCTYPE)?;
            self.output.write_char('\n')?;
        }
        Ok(())
    }

    /// Writes an already-sanitized XML comment (`<!-- {comment_body} -->`) at the current indent.
    pub fn write_raw_comment(&mut self, comment_body: &str) -> fmt::Result {
        self.write_raw_comment_with(|out| out.write_str(comment_body))
    }

    /// Streams an already-sanitized XML comment (`<!-- {f()} -->`) at the current indent
    /// using a closure, avoiding any intermediate `String` allocation.
    pub fn write_raw_comment_with<F>(&mut self, f: F) -> fmt::Result
    where
        F: FnOnce(&mut W) -> fmt::Result,
    {
        self.write_indent()?;
        self.output.write_str("<!-- ")?;
        f(&mut self.output)?;
        self.output.write_str(" -->\n")
    }

    /// Sanitizes `raw_comment` per XML 1.0 §2.5 rules and writes it as an XML comment.
    pub fn write_comment(&mut self, raw_comment: &str) -> fmt::Result {
        self.write_raw_comment_with(|out| write_sanitized_xml_comment_text(out, raw_comment))
    }

    /// Opens a paired XML element `<{name}>` and increments the indentation depth.
    pub fn open_tag(&mut self, name: &str) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")?;
        self.depth += 1;
        Ok(())
    }

    /// Opens a paired XML element `<{name} attr="val">` and increments the indentation depth.
    pub fn open_tag_with_attrs(&mut self, name: &str, attrs: &[(&str, &str)]) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.write_attrs(attrs)?;
        self.output.write_str(">\n")?;
        self.depth += 1;
        Ok(())
    }

    /// Closes a paired XML element `</{name}>` after decrementing the indentation depth.
    pub fn close_tag(&mut self, name: &str) -> fmt::Result {
        self.depth = self.depth.saturating_sub(1);
        self.write_indent()?;
        self.output.write_str("</")?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")
    }

    /// Writes a self-closing empty element `<{name}/>` at the current indentation depth.
    pub fn empty_tag(&mut self, name: &str) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.output.write_str("/>\n")
    }

    /// Writes a self-closing empty element `<{name} attr="val"/>` at the current indentation depth.
    pub fn empty_tag_with_attrs(&mut self, name: &str, attrs: &[(&str, &str)]) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.write_attrs(attrs)?;
        self.output.write_str("/>\n")
    }

    /// Writes a single-line leaf element `<{name}>{escaped_text}</{name}>`.
    pub fn text_element(&mut self, name: &str, text: &str) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.output.write_char('>')?;
        write_escaped_text(&mut self.output, text)?;
        self.output.write_str("</")?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")
    }

    /// Writes a single-line leaf element `<{name} attr="val">{escaped_text}</{name}>`.
    pub fn text_element_with_attrs(
        &mut self,
        name: &str,
        attrs: &[(&str, &str)],
        text: &str,
    ) -> fmt::Result {
        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.write_attrs(attrs)?;
        self.output.write_char('>')?;
        write_escaped_text(&mut self.output, text)?;
        self.output.write_str("</")?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")
    }

    /// Writes a single-line leaf element `<{name}>{value}</{name}>` without
    /// allocating an intermediate [`String`].
    pub fn display_element<T: fmt::Display>(&mut self, name: &str, value: T) -> fmt::Result {
        use fmt::Write as _;

        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.output.write_char('>')?;
        write!(EscapedTextWriter(&mut self.output), "{value}")?;
        self.output.write_str("</")?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")
    }

    /// Writes a single-line leaf element `<{name} attr="val">{value}</{name}>`
    /// without allocating an intermediate [`String`].
    pub fn display_element_with_attrs<T: fmt::Display>(
        &mut self,
        name: &str,
        attrs: &[(&str, &str)],
        value: T,
    ) -> fmt::Result {
        use fmt::Write as _;

        self.write_indent()?;
        self.output.write_char('<')?;
        self.output.write_str(name)?;
        self.write_attrs(attrs)?;
        self.output.write_char('>')?;
        write!(EscapedTextWriter(&mut self.output), "{value}")?;
        self.output.write_str("</")?;
        self.output.write_str(name)?;
        self.output.write_str(">\n")
    }

    /// Formats a non-negative integer into a caller-owned stack buffer without allocating.
    #[must_use]
    pub fn format_u128(mut value: u128, buf: &mut [u8; 40]) -> &str {
        if value == 0 {
            buf[39] = b'0';
            return std::str::from_utf8(&buf[39..]).unwrap_or("0");
        }
        let mut idx = buf.len();
        while value > 0 {
            idx -= 1;
            buf[idx] = b'0' + (value % 10) as u8;
            value /= 10;
        }
        std::str::from_utf8(&buf[idx..]).unwrap_or("")
    }

    fn write_indent(&mut self) -> fmt::Result {
        let mut remaining = usize::from(self.indent_spaces) * self.depth;
        while remaining > 0 {
            let chunk = remaining.min(SPACES.len());
            self.output.write_str(&SPACES[..chunk])?;
            remaining -= chunk;
        }
        Ok(())
    }

    fn write_attrs(&mut self, attrs: &[(&str, &str)]) -> fmt::Result {
        for &(key, value) in attrs {
            self.output.write_char(' ')?;
            self.output.write_str(key)?;
            self.output.write_str("=\"")?;
            write_escaped_attr(&mut self.output, value)?;
            self.output.write_char('"')?;
        }
        Ok(())
    }
}

/// Tracks line cursors and emits Level 1 (`AnnotationLevel::Line`) and Level 2
/// (`AnnotationLevel::Verbose`) source provenance comments into an [`XmlWriter`].
#[derive(Clone, Debug)]
pub struct ProvenanceEmitter<'src> {
    source_map: Option<SourceMap<'src>>,
    level: AnnotationLevel,
    last_music_line: Option<usize>,
    last_header_line: Option<usize>,
}

impl<'src> ProvenanceEmitter<'src> {
    /// Creates a new provenance emitter for `source_map` and `level`.
    #[must_use]
    pub const fn new(source_map: Option<SourceMap<'src>>, level: AnnotationLevel) -> Self {
        Self {
            source_map,
            level,
            last_music_line: None,
            last_header_line: None,
        }
    }

    /// Returns the configured [`AnnotationLevel`].
    #[must_use]
    pub const fn level(&self) -> AnnotationLevel {
        self.level
    }

    /// Resets the per-part music-line and header-attribute cursors when starting a new `<part>`.
    pub const fn reset_part_cursors(&mut self) {
        self.last_music_line = None;
        self.last_header_line = None;
    }

    /// Resets only the header/attribute line cursor (for example when entering `<attributes>`).
    pub const fn reset_header_cursor(&mut self) {
        self.last_header_line = None;
    }

    /// Emits provenance comments for a `<score-header>` or `<attributes>` element.
    ///
    /// Uses the separate `last_header_line` cursor so header-derived attributes in
    /// Measure 1 do not overwrite the `<part>`'s music-line cursor.
    pub fn emit_header_provenance<W: fmt::Write>(
        &mut self,
        writer: &mut XmlWriter<W>,
        provenance: Option<&SourceProvenance>,
    ) -> fmt::Result {
        emit_provenance_for_cursor(
            self.source_map.as_ref(),
            self.level,
            &mut self.last_header_line,
            writer,
            provenance,
        )
    }

    /// Emits provenance comments for a `<measure>` or measure-level music element
    /// (`<note>`, `<harmony>`, `<direction>`, `<backup>`, `<forward>`, `<barline>`).
    ///
    /// Uses the per-part `last_music_line` cursor so consecutive elements or measures
    /// originating from the same physical ABC line emit the Level 1 line comment once.
    pub fn emit_music_provenance<W: fmt::Write>(
        &mut self,
        writer: &mut XmlWriter<W>,
        provenance: Option<&SourceProvenance>,
    ) -> fmt::Result {
        emit_provenance_for_cursor(
            self.source_map.as_ref(),
            self.level,
            &mut self.last_music_line,
            writer,
            provenance,
        )
    }

    /// Emits provenance comments for a sub-element inside `<note>` that originates
    /// from a distinct ABC token or span (such as `<tie>`, `<tied>`, `<slur>`,
    /// `<tuplet>`, `<ornaments>`, `<articulations>`, `<technical>`, or `<lyric>`).
    ///
    /// Sub-elements only emit Level 2 (`AnnotationLevel::Verbose`) span comments.
    pub fn emit_subelement_provenance<W: fmt::Write>(
        &self,
        writer: &mut XmlWriter<W>,
        provenance: Option<&SourceProvenance>,
    ) -> fmt::Result {
        if self.level != AnnotationLevel::Verbose {
            return Ok(());
        }
        let (Some(map), Some(prov)) = (&self.source_map, provenance) else {
            return Ok(());
        };
        writer.write_raw_comment_with(|out| {
            map.write_verbose_comment_with_hint(out, prov, self.last_music_line)
        })
    }
}

fn emit_provenance_for_cursor<W: fmt::Write>(
    source_map: Option<&SourceMap<'_>>,
    level: AnnotationLevel,
    cursor: &mut Option<usize>,
    writer: &mut XmlWriter<W>,
    provenance: Option<&SourceProvenance>,
) -> fmt::Result {
    let (Some(map), Some(prov)) = (source_map, provenance) else {
        return Ok(());
    };
    if level == AnnotationLevel::None {
        return Ok(());
    }

    let hint = cursor.unwrap_or(0);
    let line_idx = map.line_index_with_hint(prov.line_span().start, hint);
    if *cursor != Some(line_idx) {
        *cursor = Some(line_idx);
        writer.write_raw_comment_with(|out| map.write_line_comment(out, line_idx))?;
        for &extra_span in prov.extra_line_spans() {
            let extra_idx = map.line_index_with_hint(extra_span.start, line_idx);
            writer.write_raw_comment_with(|out| map.write_line_comment(out, extra_idx))?;
        }
    }

    if level == AnnotationLevel::Verbose {
        writer.write_raw_comment_with(|out| {
            map.write_verbose_comment_with_hint(out, prov, *cursor)
        })?;
    }

    Ok(())
}

struct EscapedTextWriter<'a, W>(&'a mut W);

impl<W: fmt::Write> fmt::Write for EscapedTextWriter<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_escaped_text(self.0, s)
    }
}

fn write_escaped_text<W: fmt::Write>(output: &mut W, text: &str) -> fmt::Result {
    let mut last = 0;
    for (i, b) in text.bytes().enumerate() {
        let escape = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            _ => continue,
        };
        if last < i {
            output.write_str(&text[last..i])?;
        }
        output.write_str(escape)?;
        last = i + 1;
    }
    if last < text.len() {
        output.write_str(&text[last..])?;
    }
    Ok(())
}

fn write_escaped_attr<W: fmt::Write>(output: &mut W, value: &str) -> fmt::Result {
    let mut last = 0;
    for (i, b) in value.bytes().enumerate() {
        let escape = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\t' => "&#9;",
            b'\n' => "&#10;",
            b'\r' => "&#13;",
            _ => continue,
        };
        if last < i {
            output.write_str(&value[last..i])?;
        }
        output.write_str(escape)?;
        last = i + 1;
    }
    if last < value.len() {
        output.write_str(&value[last..])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use abc_parser::SimpleSpan;

    use super::ProvenanceEmitter;
    use super::XmlWriter;
    use crate::AnnotationLevel;
    use crate::source_map::SourceMap;
    use crate::source_map::SourceProvenance;

    #[test]
    fn writes_escaped_attributes_and_text() {
        let mut writer = XmlWriter::new(String::new(), 2);
        writer.write_declaration(true).unwrap();
        writer
            .open_tag_with_attrs("score-partwise", &[("version", "4.0")])
            .unwrap();
        writer.write_comment("hdr -- end-").unwrap();
        writer
            .text_element_with_attrs(
                "words",
                &[("justify", "left & \"center\"\t\r\n")],
                "Rock & Roll <1> > 0",
            )
            .unwrap();
        writer
            .empty_tag_with_attrs("tie", &[("type", "start")])
            .unwrap();
        writer.close_tag("score-partwise").unwrap();

        let xml = writer.into_inner();
        assert_eq!(
            xml,
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \"http://www.musicxml.org/dtds/partwise.dtd\">\n",
                "<score-partwise version=\"4.0\">\n",
                "  <!-- hdr -%2D end%2D -->\n",
                "  <words justify=\"left &amp; &quot;center&quot;&#9;&#13;&#10;\">Rock &amp; Roll &lt;1&gt; &gt; 0</words>\n",
                "  <tie type=\"start\"/>\n",
                "</score-partwise>\n",
            )
        );
    }

    #[test]
    fn tracks_provenance_cursors_across_header_music_and_part_transitions() {
        // Line 1 (0..4):   "T:A\n" (0..3)
        // Line 2 (4..8):   "+:B\n" (4..7)
        // Line 3 (8..12):  "K:C\n" (8..11)
        // Line 4 (12..18): "C D |\n" (12..17)
        // Line 5 (18..24): "E F |\n" (18..23)
        let src = "T:A\n+:B\nK:C\nC D |\nE F |\n";
        let map = SourceMap::new(src, "t.abc");

        let emit_sequence = |level: AnnotationLevel| {
            let mut emitter = ProvenanceEmitter::new(Some(map.clone()), level);
            let mut writer = XmlWriter::new(String::new(), 0);

            // 1. Header field with `+:B` continuation emits both lines once, then deduplicates line 1.
            let title_prov = SourceProvenance::from_line(SimpleSpan::from(0..3))
                .with_extra_line_spans(vec![SimpleSpan::from(4..7)]);
            emitter
                .emit_header_provenance(&mut writer, Some(&title_prov))
                .unwrap();
            writer.text_element("work-title", "A B").unwrap();
            emitter
                .emit_header_provenance(&mut writer, Some(&title_prov))
                .unwrap();

            // 2. Music line 4 ("C D |"), interrupted by header-attribute line 3 ("K:C"),
            //    followed by another music element on line 4 (must NOT re-emit line 4),
            //    a sub-element on line 4 (Verbose-only), and a transition to line 5 ("E F |").
            let line4_elem1 =
                SourceProvenance::new(SimpleSpan::from(12..17), SimpleSpan::from(12..13));
            let key_prov = SourceProvenance::from_line(SimpleSpan::from(8..11));
            let line4_elem2 =
                SourceProvenance::new(SimpleSpan::from(12..17), SimpleSpan::from(14..15));
            let line4_sub =
                SourceProvenance::new(SimpleSpan::from(12..17), SimpleSpan::from(14..15));
            let line5_elem1 =
                SourceProvenance::new(SimpleSpan::from(18..23), SimpleSpan::from(18..19));

            emitter
                .emit_music_provenance(&mut writer, Some(&line4_elem1))
                .unwrap();
            emitter
                .emit_header_provenance(&mut writer, Some(&key_prov))
                .unwrap();
            writer.text_element("fifths", "0").unwrap();
            emitter
                .emit_music_provenance(&mut writer, Some(&line4_elem2))
                .unwrap();
            emitter
                .emit_subelement_provenance(&mut writer, Some(&line4_sub))
                .unwrap();
            writer.empty_tag("fermata").unwrap();
            emitter
                .emit_music_provenance(&mut writer, Some(&line5_elem1))
                .unwrap();

            // 3. Resetting part cursors causes line 5 to be emitted again for the next part.
            emitter.reset_part_cursors();
            emitter
                .emit_music_provenance(&mut writer, Some(&line5_elem1))
                .unwrap();
            writer.into_inner()
        };

        let none_xml = emit_sequence(AnnotationLevel::None);
        let line_xml = emit_sequence(AnnotationLevel::Line);
        let verbose_xml = emit_sequence(AnnotationLevel::Verbose);

        assert_eq!(
            line_xml,
            concat!(
                "<!-- t.abc:1: T:A -->\n",
                "<!-- t.abc:2: +:B -->\n",
                "<work-title>A B</work-title>\n",
                "<!-- t.abc:4: C D | -->\n",
                "<!-- t.abc:3: K:C -->\n",
                "<fifths>0</fifths>\n",
                "<fermata/>\n",
                "<!-- t.abc:5: E F | -->\n",
                "<!-- t.abc:5: E F | -->\n",
            )
        );
        assert!(verbose_xml.contains("<!-- @4:3..4:4 [14..15]: D -->\n<fermata/>\n"));

        // Stripping Level 2 (`<!-- @... -->`) comments from Verbose yields Line;
        // stripping all comments yields None.
        let verbose_without_span_comments: String = verbose_xml
            .lines()
            .filter(|line| !line.trim_start().starts_with("<!-- @"))
            .flat_map(|line| [line, "\n"])
            .collect();
        assert_eq!(verbose_without_span_comments, line_xml);

        let line_without_comments: String = line_xml
            .lines()
            .filter(|line| !line.trim_start().starts_with("<!--"))
            .flat_map(|line| [line, "\n"])
            .collect();
        assert_eq!(line_without_comments, none_xml);
    }
}
