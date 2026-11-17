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

//! Core domain modules and XML/MXL writers for `MusicXML` 4.0 generation.

use std::error::Error;
use std::fmt;

use abc_parser::BarDurationPickupPolicy;

pub mod build_info;
pub mod harmony;
pub mod lyrics;
#[cfg(feature = "mxl")]
pub mod mxl;
pub mod pitch;
pub mod score_directive;
pub mod source_map;
pub mod timing;
pub mod writer;

#[cfg(feature = "mxl")]
pub use mxl::create_mxl;
#[cfg(feature = "mxl")]
pub use mxl::create_mxl_seek;
pub use source_map::LineColumn;
pub use source_map::SourceMap;
pub use source_map::SourceProvenance;
pub use source_map::decode_xml_comment;
pub use source_map::sanitize_xml_comment;
pub use writer::ProvenanceEmitter;
pub use writer::XmlWriter;

/// Controls how ABC source provenance is emitted into `MusicXML` output.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnnotationLevel {
    /// Emit no ABC source comments.
    #[default]
    None,
    /// Emit a comment with source filename, 1-based line number, and ABC line text
    /// before the XML elements generated from that line.
    Line,
    /// Emit Level 1 line comments plus a Hybrid (Option D) source-offset reference
    /// on every generated semantic XML element.
    Verbose,
}

/// Configuration options for converting ABC 2.1 to `MusicXML` 4.0.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MusicXmlOptions {
    source_name: Option<String>,
    annotation_level: AnnotationLevel,
    include_doctype: bool,
    indent_spaces: u8,
    pickup_policy: BarDurationPickupPolicy,
    recover: bool,
}

impl Default for MusicXmlOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicXmlOptions {
    /// Creates default `MusicXML` conversion options.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            source_name: None,
            annotation_level: AnnotationLevel::None,
            include_doctype: true,
            indent_spaces: 2,
            pickup_policy: BarDurationPickupPolicy::OpeningBar,
            recover: false,
        }
    }

    /// Sets the display source filename used in Level 1 and Level 2 provenance comments.
    #[must_use]
    pub fn with_source_name(mut self, source_name: impl Into<String>) -> Self {
        self.source_name = Some(source_name.into());
        self
    }

    /// Sets the source provenance annotation level.
    #[must_use]
    pub const fn with_annotation_level(mut self, level: AnnotationLevel) -> Self {
        self.annotation_level = level;
        self
    }

    /// Sets whether to emit the `<!DOCTYPE score-partwise ...>` declaration.
    #[must_use]
    pub const fn with_doctype(mut self, include_doctype: bool) -> Self {
        self.include_doctype = include_doctype;
        self
    }

    /// Sets the number of indentation spaces per XML nesting level.
    #[must_use]
    pub const fn with_indent_spaces(mut self, indent_spaces: u8) -> Self {
        self.indent_spaces = indent_spaces;
        self
    }

    /// Sets the policy for inferring pickup measures (`implicit="yes" number="0"`).
    #[must_use]
    pub const fn with_pickup_policy(mut self, pickup_policy: BarDurationPickupPolicy) -> Self {
        self.pickup_policy = pickup_policy;
        self
    }

    /// Sets whether to attempt best-effort `MusicXML` emission when the input has syntax errors.
    #[must_use]
    pub const fn with_recover(mut self, recover: bool) -> Self {
        self.recover = recover;
        self
    }

    /// Returns the configured display source filename, if any.
    #[must_use]
    pub fn source_name(&self) -> Option<&str> {
        self.source_name.as_deref()
    }

    /// Returns the configured [`AnnotationLevel`].
    #[must_use]
    pub const fn annotation_level(&self) -> AnnotationLevel {
        self.annotation_level
    }

    /// Returns `true` if the `<!DOCTYPE score-partwise ...>` declaration should be emitted.
    #[must_use]
    pub const fn include_doctype(&self) -> bool {
        self.include_doctype
    }

    /// Returns the number of indentation spaces per XML nesting level.
    #[must_use]
    pub const fn indent_spaces(&self) -> u8 {
        self.indent_spaces
    }

    /// Returns the configured [`BarDurationPickupPolicy`].
    #[must_use]
    pub const fn pickup_policy(&self) -> BarDurationPickupPolicy {
        self.pickup_policy
    }

    /// Returns `true` if best-effort error recovery is enabled.
    #[must_use]
    pub const fn recover(&self) -> bool {
        self.recover
    }
}

/// Errors that can occur during ABC to `MusicXML` conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MusicXmlError {
    /// The ABC document contained no tunes.
    EmptyDocument,
    /// The requested `X:` reference number was not found in the document.
    TuneNotFound {
        /// Requested `X:` reference number.
        requested: u32,
        /// Available `X:` reference numbers in the document.
        available: Vec<u32>,
    },
    /// Multiple tunes are present, but neither `--tune` nor batch mode was used.
    MultipleTunes {
        /// Available `X:` reference numbers in the document.
        available: Vec<u32>,
    },
    /// Formatting to the underlying [`fmt::Write`] sink failed.
    FormatError,
}

impl fmt::Display for MusicXmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDocument => f.write_str("ABC document contains no tunes"),
            Self::TuneNotFound {
                requested,
                available,
            } => {
                write!(
                    f,
                    "tune X:{requested} not found (available tunes: {available:?})"
                )
            }
            Self::MultipleTunes { available } => {
                write!(
                    f,
                    "document contains multiple tunes ({available:?}); specify --tune <X> or --out-dir <DIR>"
                )
            }
            Self::FormatError => f.write_str("failed to write MusicXML output"),
        }
    }
}

impl Error for MusicXmlError {}

impl From<fmt::Error> for MusicXmlError {
    fn from(_: fmt::Error) -> Self {
        Self::FormatError
    }
}

/// Generates an `opus.xml` collection document conforming to `musicxml-4/schema/opus.xsd`.
///
/// # Errors
///
/// Returns [`MusicXmlError::FormatError`] if formatting fails.
pub fn generate_opus_xml(
    title: Option<&str>,
    score_hrefs: &[&str],
    indent_spaces: u8,
) -> Result<String, MusicXmlError> {
    let mut writer = XmlWriter::new(String::new(), indent_spaces);
    writer.write_declaration(false)?;
    writer.open_tag_with_attrs(
        "opus",
        &[
            ("version", "4.0"),
            ("xmlns:xlink", "http://www.w3.org/1999/xlink"),
        ],
    )?;
    if let Some(t) = title {
        writer.text_element("title", t)?;
    }
    for href in score_hrefs {
        writer.empty_tag_with_attrs("score", &[("xlink:href", href)])?;
    }
    writer.close_tag("opus")?;
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::generate_opus_xml;

    #[test]
    fn generates_valid_opus_xml_document() {
        let opus = generate_opus_xml(
            Some("Suite & Sonatas"),
            &["tune-1.musicxml", "tune-2.musicxml"],
            2,
        )
        .unwrap();
        assert_eq!(
            opus,
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<opus version=\"4.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n",
                "  <title>Suite &amp; Sonatas</title>\n",
                "  <score xlink:href=\"tune-1.musicxml\"/>\n",
                "  <score xlink:href=\"tune-2.musicxml\"/>\n",
                "</opus>\n",
            )
        );

        let opus_no_title = generate_opus_xml(None, &["tune-1.musicxml"], 2).unwrap();
        assert_eq!(
            opus_no_title,
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<opus version=\"4.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">\n",
                "  <score xlink:href=\"tune-1.musicxml\"/>\n",
                "</opus>\n",
            )
        );
    }
}
