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

//! Integration tests for `MusicXML` 4.0 Phase 3 semantic features:
//! - Multi-staff braced keyboards with lagging-staff padding
//! - Compressed `.mxl` archive export and container manifest
//! - MIDI channel, program, volume, and pan directives
//! - Layout line breaks, page breaks, and typesetting comments
//! - Unpitched percussion notation mapping

use std::fs;
#[cfg(feature = "mxl")]
use std::io::Read as _;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use abc_musicxml::MusicXmlOptions;
use abc_musicxml::tune_to_musicxml;
use abc_parser::IntoOwnedAst;
use abc_parser::parse;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

const MUSICXML_SCHEMA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../musicxml-4/schema/musicxml.xsd"
);
const CATALOG_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../musicxml-4/schema/catalog.xml"
);

fn validate_xml(xml: &str) {
    let xmllint_available = Command::new("xmllint")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !xmllint_available || !Path::new(MUSICXML_SCHEMA_PATH).exists() {
        return;
    }

    let uid = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path =
        std::env::temp_dir().join(format!("phase3-val-{}-{}.xml", std::process::id(), uid));
    fs::write(&temp_path, xml).expect("write temp validation file");

    let output = Command::new("xmllint")
        .env("XML_CATALOG_FILES", CATALOG_PATH)
        .arg("--noout")
        .arg("--schema")
        .arg(MUSICXML_SCHEMA_PATH)
        .arg(&temp_path)
        .output()
        .expect("xmllint execution");

    fs::remove_file(&temp_path).ok();

    assert!(
        output.status.success(),
        "xmllint validation failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn keyboard_grand_staff_lowering_and_padding() {
    let abc = concat!(
        "X:1\n",
        "T:Keyboard Grand Staff\n",
        "%%score {RH | LH}\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:RH clef=treble\n",
        "c d e f | g4 |\n",
        "V:LH clef=bass\n",
        "C E G | C4 |\n",
    );
    let parsed = parse(abc);
    let doc = parsed.output.unwrap().into_owned(abc).unwrap();
    let tune = doc.tunes().next().unwrap();
    let options = MusicXmlOptions::new().with_doctype(false);

    let output = tune_to_musicxml(tune, &doc.header, Some(abc), &options).unwrap();
    let xml = &output.xml;

    // Schema validation
    validate_xml(xml);

    // Single score-part representing the piano
    assert!(xml.contains("<score-part id=\"P1\">"));
    assert!(!xml.contains("<score-part id=\"P2\">"));

    // Multi-staff attributes on Measure 1
    assert!(xml.contains("<staves>2</staves>"));
    assert!(xml.contains("<part-symbol>brace</part-symbol>"));
    assert!(xml.contains(
        "<clef number=\"1\">\n          <sign>G</sign>\n          <line>2</line>\n        </clef>"
    ));
    assert!(xml.contains(
        "<clef number=\"2\">\n          <sign>F</sign>\n          <line>4</line>\n        </clef>"
    ));

    // Measure 1: RH has 4 quarter notes on staff 1
    assert!(xml.contains("<staff>1</staff>"));
    // Rewind backup
    assert!(xml.contains("<backup>\n        <duration>4</duration>\n      </backup>"));
    // LH has 3 quarter notes on staff 2
    assert!(xml.contains("<staff>2</staff>"));

    // Mismatched duration in Measure 1 (4 vs 3 beats) was padded with invisible rest
    assert!(xml.contains("<note print-object=\"no\">\n        <rest/>"));
    assert_eq!(output.warnings.len(), 1);
    assert!(
        output.warnings[0]
            .message
            .contains("duration mismatch on staff 2")
    );
}

#[test]
fn midi_playback_directives_lowering() {
    let abc = concat!(
        "X:1\n",
        "T:MIDI Directives\n",
        "M:4/4\n",
        "L:1/4\n",
        "%%MIDI channel 3\n",
        "%%MIDI program 41\n",
        "%%MIDI control 7 100\n",
        "%%MIDI pan 30\n",
        "K:C\n",
        "c d e f |\n",
    );
    let parsed = parse(abc);
    let doc = parsed.output.unwrap().into_owned(abc).unwrap();
    let tune = doc.tunes().next().unwrap();
    let options = MusicXmlOptions::new().with_doctype(false);

    let output = tune_to_musicxml(tune, &doc.header, Some(abc), &options).unwrap();
    let xml = &output.xml;

    validate_xml(xml);

    assert!(xml.contains("<score-instrument id=\"P1-I1\">"));
    assert!(xml.contains("<midi-instrument id=\"P1-I1\">"));
    assert!(xml.contains("<midi-channel>3</midi-channel>"));
    assert!(xml.contains("<midi-program>41</midi-program>"));
    // 100 / 127 scaled to percent: (100 * 100 + 63) / 127 = 79%
    assert!(xml.contains("<volume>79</volume>"));
    assert!(xml.contains("<pan>30</pan>"));
}

#[test]
fn layout_directives_and_typesetting_comments() {
    let abc = concat!(
        "X:1\n",
        "T:Layout Directives\n",
        "%%scale 0.8\n",
        "%%topmargin 2.0cm\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f | $ g a b c' | !line-break! c' b a g | !newpage! f e d c |\n",
    );
    let parsed = parse(abc);
    let doc = parsed.output.unwrap().into_owned(abc).unwrap();
    let tune = doc.tunes().next().unwrap();
    let options = MusicXmlOptions::new().with_doctype(false);

    let output = tune_to_musicxml(tune, &doc.header, Some(abc), &options).unwrap();
    let xml = &output.xml;

    validate_xml(xml);

    // Typesetting comments emitted as XML comments
    assert!(xml.contains("<!-- %%scale 0.8 -->"));
    assert!(xml.contains("<!-- %%topmargin 2.0cm -->"));

    // Measure 2 has new-system from $
    assert!(xml.contains("<print new-system=\"yes\"/>"));
    // Measure 4 has new-page from !newpage!
    assert!(xml.contains("<print new-page=\"yes\"/>"));
}

#[test]
fn unpitched_percussion_lowering() {
    let abc = concat!(
        "X:1\n",
        "T:Percussion\n",
        "M:4/4\n",
        "L:1/4\n",
        "V:1 clef=perc\n",
        "K:C\n",
        "^c =d _e f |\n",
    );
    let parsed = parse(abc);
    let doc = parsed.output.unwrap().into_owned(abc).unwrap();
    let tune = doc.tunes().next().unwrap();
    let options = MusicXmlOptions::new().with_doctype(false);

    let output = tune_to_musicxml(tune, &doc.header, Some(abc), &options).unwrap();
    let xml = &output.xml;

    validate_xml(xml);

    // Percussion clef
    assert!(xml.contains("<sign>percussion</sign>"));

    // Notes must use <unpitched> with <display-step> and <display-octave>, without <pitch> or <accidental>
    assert!(xml.contains("<unpitched>\n          <display-step>C</display-step>\n          <display-octave>5</display-octave>\n        </unpitched>"));
    assert!(xml.contains("<unpitched>\n          <display-step>D</display-step>\n          <display-octave>5</display-octave>\n        </unpitched>"));
    assert!(!xml.contains("<pitch>"));
    assert!(!xml.contains("<accidental>"));
}

#[test]
#[cfg(feature = "mxl")]
fn compressed_mxl_cli_export_and_extraction() {
    let temp_dir = std::env::temp_dir().join(format!("mxl-test-{}", std::process::id()));
    fs::create_dir_all(&temp_dir).unwrap();
    let abc_path = temp_dir.join("test.abc");
    let mxl_path = temp_dir.join("test.mxl");

    fs::write(
        &abc_path,
        concat!(
            "X:1\n",
            "T:Compressed MXL Test\n",
            "M:4/4\n",
            "L:1/4\n",
            "K:C\n",
            "c d e f |\n",
        ),
    )
    .unwrap();

    let exe = env!("CARGO_BIN_EXE_abc-musicxml");
    let status = Command::new(exe)
        .arg(&abc_path)
        .arg("-o")
        .arg(&mxl_path)
        .status()
        .expect("execute abc-musicxml");
    assert!(status.success());
    assert!(mxl_path.exists());

    // Verify ZIP archive structure
    let file = fs::File::open(&mxl_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();

    // 1. Verify META-INF/container.xml
    let mut container_file = archive.by_name("META-INF/container.xml").unwrap();
    let mut container_content = String::new();
    container_file
        .read_to_string(&mut container_content)
        .unwrap();
    assert!(container_content.contains("full-path=\"score.musicxml\""));
    assert!(container_content.contains("media-type=\"application/vnd.recordare.musicxml+xml\""));
    drop(container_file);

    // 2. Verify score.musicxml extraction and content
    let mut score_file = archive.by_name("score.musicxml").unwrap();
    let mut score_xml = String::new();
    score_file.read_to_string(&mut score_xml).unwrap();
    assert!(score_xml.contains("<movement-title>Compressed MXL Test</movement-title>"));
    drop(score_file);

    // 3. Schema validate extracted MusicXML
    validate_xml(&score_xml);

    fs::remove_dir_all(&temp_dir).ok();
}
