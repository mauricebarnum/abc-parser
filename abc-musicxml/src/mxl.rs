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

//! `MusicXML` 4.0 compressed `.mxl` archive container generation.
//!
//! An `.mxl` package conforms to the W3C `container.xsd` specification:
//! - Contains a `META-INF/container.xml` manifest pointing to the root score file.
//! - Stores the root `MusicXML` score XML payload using standard DEFLATE compression.

use std::io::Write as _;

use crate::writer::XmlWriter;

/// Creates a compressed `.mxl` ZIP container writing directly to a seekable writer.
///
/// Contains `META-INF/container.xml` stored uncompressed (method 0), and
/// compresses `score_xml` as that root file using DEFLATE.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if writing or finishing the ZIP archive fails.
pub fn create_mxl_seek<W: std::io::Write + std::io::Seek>(
    writer: W,
    rootfile_name: &str,
    score_xml: &[u8],
) -> std::io::Result<()> {
    let mut zip = zip::ZipWriter::new(writer);
    let container_xml = format_container_xml(rootfile_name).map_err(std::io::Error::other)?;

    let stored_options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("META-INF/container.xml", stored_options)?;
    zip.write_all(container_xml.as_bytes())?;

    let deflated_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(rootfile_name, deflated_options)?;
    zip.write_all(score_xml)?;

    zip.finish()?;
    Ok(())
}

fn format_container_xml(rootfile_name: &str) -> Result<String, std::fmt::Error> {
    let mut container_xml = String::with_capacity(256);
    let mut xml = XmlWriter::new(&mut container_xml, 2);
    xml.write_declaration(false)?;
    xml.open_tag("container")?;
    xml.open_tag("rootfiles")?;
    xml.empty_tag_with_attrs(
        "rootfile",
        &[
            ("full-path", rootfile_name),
            ("media-type", "application/vnd.recordare.musicxml+xml"),
        ],
    )?;
    xml.close_tag("rootfiles")?;
    xml.close_tag("container")?;
    Ok(container_xml)
}

/// Creates a compressed `.mxl` ZIP container writing to `writer`.
///
/// Contains `META-INF/container.xml` pointing to `rootfile_name`, and
/// compresses `score_xml` as that root file. If `writer` does not implement
/// [`std::io::Seek`], an in-memory buffer is used before writing.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if writing or finishing the ZIP archive fails.
pub fn create_mxl<W: std::io::Write>(
    mut writer: W,
    rootfile_name: &str,
    score_xml: &[u8],
) -> std::io::Result<()> {
    let mut buffer = std::io::Cursor::new(Vec::with_capacity(score_xml.len() / 2 + 512));
    create_mxl_seek(&mut buffer, rootfile_name, score_xml)?;
    writer.write_all(buffer.get_ref())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::io::Read as _;

    use super::create_mxl;
    use super::create_mxl_seek;

    #[test]
    fn creates_valid_mxl_container() {
        let xml_payload =
            b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<score-partwise version=\"4.0\"/>\n";
        let mut buffer = Cursor::new(Vec::new());
        create_mxl_seek(&mut buffer, "score.musicxml", xml_payload).unwrap();

        let zip_bytes = buffer.into_inner();
        let mut archive = zip::ZipArchive::new(Cursor::new(zip_bytes)).unwrap();

        // 1. Verify META-INF/container.xml is Stored
        {
            let mut container_file = archive.by_name("META-INF/container.xml").unwrap();
            assert_eq!(container_file.compression(), zip::CompressionMethod::Stored);
            let mut container_content = String::new();
            container_file
                .read_to_string(&mut container_content)
                .unwrap();
            assert!(container_content.contains("full-path=\"score.musicxml\""));
            assert!(
                container_content.contains("media-type=\"application/vnd.recordare.musicxml+xml\"")
            );
        }

        // 2. Verify root score file is Deflated
        {
            let mut score_file = archive.by_name("score.musicxml").unwrap();
            assert_eq!(score_file.compression(), zip::CompressionMethod::Deflated);
            let mut score_content = Vec::new();
            score_file.read_to_end(&mut score_content).unwrap();
            assert_eq!(score_content, xml_payload);
        }

        // 3. Verify create_mxl unseekable path produces valid archive
        let mut out = Vec::new();
        create_mxl(&mut out, "score.musicxml", xml_payload).unwrap();
        let mut archive2 = zip::ZipArchive::new(Cursor::new(out)).unwrap();
        assert!(archive2.by_name("META-INF/container.xml").is_ok());
        assert!(archive2.by_name("score.musicxml").is_ok());
    }

    #[test]
    fn escapes_rootfile_name_attributes_in_container_xml() {
        let xml_payload = b"<score-partwise version=\"4.0\"/>\n";
        let special_name = "a&b\"c<d>.musicxml";
        let mut buffer = Cursor::new(Vec::new());
        create_mxl_seek(&mut buffer, special_name, xml_payload).unwrap();

        let mut archive = zip::ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let mut container_file = archive.by_name("META-INF/container.xml").unwrap();
        let mut container_content = String::new();
        container_file
            .read_to_string(&mut container_content)
            .unwrap();
        assert!(
            container_content.contains("full-path=\"a&amp;b&quot;c&lt;d&gt;.musicxml\""),
            "unexpected container.xml: {container_content}"
        );
    }
}
