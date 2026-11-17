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

//! End-to-end verification and schema validation suite for `abc-musicxml`.
//!
//! Validates generated `MusicXML` documents against the normative W3C `MusicXML` 4.0
//! XSD schema (`musicxml-4/schema/musicxml.xsd` and `opus.xsd`) via `xmllint` when
//! available in the test environment, and performs comprehensive in-Rust structural
//! XML child-element ordering, hierarchy, and semantic assertions across all
//! annotation levels.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use abc_musicxml::AnnotationLevel;
use abc_musicxml::MusicXmlOptions;
use abc_musicxml::tune_to_musicxml;
use abc_parser::IntoOwnedAst;
use abc_parser::OwnedDocument;
use abc_parser::SimpleSpan;
use abc_parser::parse;

const KITCHEN_SINK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_kitchen_sink.abc");
const CATALOG_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../musicxml-4/schema/catalog.xml"
);
const MUSICXML_SCHEMA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../musicxml-4/schema/musicxml.xsd"
);
const OPUS_SCHEMA_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../musicxml-4/schema/opus.xsd");

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct XmlNode {
    tag: String,
    children: Vec<Self>,
}

fn parse_xml(s: &str) -> Vec<XmlNode> {
    let mut stack: Vec<XmlNode> = vec![XmlNode {
        tag: "ROOT".to_string(),
        children: Vec::new(),
    }];
    let mut chars = s.char_indices();

    while let Some((i, c)) = chars.next() {
        if c == '<' {
            let rest = &s[i..];
            if rest.starts_with("<!--") {
                if let Some(end) = rest.find("-->") {
                    for _ in 0..end + 2 {
                        chars.next();
                    }
                }
            } else if rest.starts_with("<?") {
                if let Some(end) = rest.find("?>") {
                    for _ in 0..=end {
                        chars.next();
                    }
                }
            } else if rest.starts_with("<!") {
                if let Some(end) = rest.find('>') {
                    for _ in 0..end {
                        chars.next();
                    }
                }
            } else if rest.starts_with("</") {
                let end = rest.find('>').expect("closing >");
                let tag_name = rest[2..end].trim();
                let top = stack.pop().expect("stack underflow");
                assert_eq!(
                    top.tag, tag_name,
                    "mismatched closing tag: expected </{}>, found </{}>",
                    top.tag, tag_name
                );
                stack.last_mut().expect("parent").children.push(top);
                for _ in 0..end {
                    chars.next();
                }
            } else {
                let end = rest.find('>').expect("closing >");
                let tag_body = &rest[1..end];
                let is_self_closing = tag_body.ends_with('/');
                let body_trimmed = if is_self_closing {
                    tag_body[..tag_body.len() - 1].trim()
                } else {
                    tag_body.trim()
                };
                let tag_name = body_trimmed.split_whitespace().next().unwrap_or("");
                let node = XmlNode {
                    tag: tag_name.to_string(),
                    children: Vec::new(),
                };
                for _ in 0..end {
                    chars.next();
                }
                if is_self_closing {
                    stack.last_mut().expect("parent").children.push(node);
                } else {
                    stack.push(node);
                }
            }
        }
    }
    assert_eq!(stack.len(), 1, "unbalanced XML tags: open tags remain");
    stack.pop().unwrap().children
}

fn assert_child_order(node: &XmlNode, target_tag: &str, allowed_order: &[&str]) {
    if node.tag == target_tag {
        let mut last_idx = 0;
        for child in &node.children {
            let child_pos = allowed_order
                .iter()
                .position(|&item| item == child.tag)
                .unwrap_or_else(|| {
                    panic!(
                        "Unexpected child tag <{}> inside <{}>. Allowed tags: {:?}",
                        child.tag, target_tag, allowed_order
                    )
                });
            assert!(
                child_pos >= last_idx,
                "Tag order violation inside <{}>: <{}> (pos {}) appeared after pos {}",
                target_tag,
                child.tag,
                child_pos,
                last_idx
            );
            last_idx = child_pos;
        }
    }
    for child in &node.children {
        assert_child_order(child, target_tag, allowed_order);
    }
}

#[allow(clippy::too_many_lines)]
fn assert_strict_element_ordering(xml: &str) {
    let roots = parse_xml(xml);
    assert_eq!(roots.len(), 1, "expected exactly one root element");
    let root = &roots[0];
    assert_eq!(root.tag, "score-partwise");

    assert_child_order(
        root,
        "score-partwise",
        &[
            "work",
            "movement-number",
            "movement-title",
            "identification",
            "defaults",
            "credit",
            "part-list",
            "part",
        ],
    );

    assert_child_order(
        root,
        "identification",
        &[
            "creator",
            "rights",
            "encoding",
            "source",
            "relation",
            "miscellaneous",
        ],
    );

    assert_child_order(
        root,
        "attributes",
        &[
            "divisions",
            "key",
            "time",
            "staves",
            "part-symbol",
            "instruments",
            "clef",
            "staff-details",
            "transpose",
            "for-part",
            "directive",
            "measure-style",
        ],
    );

    assert_child_order(
        root,
        "note",
        &[
            "grace",
            "cue",
            "chord",
            "pitch",
            "rest",
            "unpitched",
            "duration",
            "tie",
            "voice",
            "type",
            "dot",
            "accidental",
            "time-modification",
            "stem",
            "notehead",
            "notehead-text",
            "staff",
            "beam",
            "notations",
            "lyric",
            "play",
            "listen",
        ],
    );

    assert_child_order(
        root,
        "notations",
        &[
            "tied",
            "slur",
            "tuplet",
            "glissando",
            "slide",
            "ornaments",
            "technical",
            "articulations",
            "dynamics",
            "fermata",
            "arpeggiate",
            "non-arpeggiate",
            "accidental-mark",
            "other-notation",
        ],
    );

    assert_child_order(
        root,
        "direction",
        &[
            "direction-type",
            "offset",
            "voice",
            "staff",
            "sound",
            "listening",
        ],
    );

    assert_child_order(
        root,
        "barline",
        &[
            "bar-style",
            "wavy-line",
            "segno",
            "coda",
            "fermata",
            "ending",
            "repeat",
        ],
    );
}

fn assert_no_illegal_hyphens_in_comments(xml: &str) {
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        let after_start = &rest[start + 4..];
        let end = after_start.find("-->").expect("unterminated comment");
        let comment_body = &after_start[..end];
        assert!(
            !comment_body.contains("--"),
            "XML comment contains illegal '--': {comment_body}"
        );
        rest = &after_start[end + 3..];
    }
}

fn validate_with_xmllint(schema_path: &str, xml_content: &str) {
    let xmllint_available = Command::new("xmllint")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());

    if !xmllint_available {
        eprintln!("xmllint not available; skipping external XSD validation");
        return;
    }

    if !Path::new(schema_path).exists() || !Path::new(CATALOG_PATH).exists() {
        eprintln!("Schema or catalog not found; skipping external XSD validation");
        return;
    }

    let count = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = std::env::temp_dir().join(format!(
        "abc-musicxml-val-{}-{count}.xml",
        std::process::id()
    ));
    fs::write(&temp_path, xml_content).expect("writing temporary validation file");

    let output = Command::new("xmllint")
        .env("XML_CATALOG_FILES", CATALOG_PATH)
        .arg("--noout")
        .arg("--schema")
        .arg(schema_path)
        .arg(&temp_path)
        .output()
        .expect("xmllint execution failed");

    fs::remove_file(&temp_path).ok();

    assert!(
        output.status.success(),
        "xmllint validation failed against {}:\nstdout:\n{}\nstderr:\n{}",
        schema_path,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn parse_abc_doc(source: &str) -> OwnedDocument<SimpleSpan<usize>> {
    let parsed = parse(source);
    let doc = parsed.output.expect("document produced");
    doc.into_owned(source).expect("owned ast")
}

fn run_cli(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_abc-musicxml"))
        .args(arguments)
        .output()
        .expect("abc-musicxml should execute")
}

#[test]
#[allow(clippy::too_many_lines)]
fn converts_kitchen_sink_tune_1_all_annotation_levels() {
    let source = fs::read_to_string(KITCHEN_SINK_PATH).expect("reading kitchen sink file");
    let document = parse_abc_doc(&source);
    let tunes: Vec<_> = document.tunes().collect();
    let tune = tunes[0];

    for level in [
        AnnotationLevel::None,
        AnnotationLevel::Line,
        AnnotationLevel::Verbose,
    ] {
        let options = MusicXmlOptions::new()
            .with_source_name("test_kitchen_sink.abc")
            .with_annotation_level(level);

        let output = tune_to_musicxml(tune, &document.header, Some(&source), &options)
            .expect("tune 1 lowering should succeed");

        validate_with_xmllint(MUSICXML_SCHEMA_PATH, &output.xml);
        assert_strict_element_ordering(&output.xml);
        assert_no_illegal_hyphens_in_comments(&output.xml);

        // Musical semantics
        assert!(output.xml.contains("<movement-number>1</movement-number>"));
        assert!(
            output
                .xml
                .contains("<movement-title>Test Tune 1: File Separation Boundary</movement-title>")
        );
        assert!(
            output
                .xml
                .contains("<creator type=\"composer\">Traditional</creator>")
        );
        assert!(output.xml.contains("<fifths>0</fifths>"));
        assert!(output.xml.contains("<mode>major</mode>"));
        assert!(output.xml.contains("<beats>4</beats>"));
        assert!(output.xml.contains("<beat-type>4</beat-type>"));
        assert!(output.xml.contains("<per-minute>120</per-minute>"));
        assert!(output.xml.contains("<repeat direction=\"forward\"/>"));
        assert!(output.xml.contains("<repeat direction=\"backward\"/>"));
        assert!(output.xml.contains("<beam number=\"1\">begin</beam>"));
        assert!(output.xml.contains("<beam number=\"1\">end</beam>"));

        // Provenance annotations
        match level {
            AnnotationLevel::None => {
                assert!(
                    !output.xml.contains("<!-- test_kitchen_sink.abc:")
                        && !output.xml.contains("<!-- @"),
                    "Level 0 should emit no source provenance comments"
                );
            }
            AnnotationLevel::Line => {
                assert!(
                    output
                        .xml
                        .contains("<!-- test_kitchen_sink.abc:12: |: CDEF GABc | cBAG FEDC :| -->"),
                    "Level 1 should emit source line comments"
                );
                assert!(
                    !output.xml.contains("<!-- @12:"),
                    "Level 1 should not emit element span comments"
                );
            }
            AnnotationLevel::Verbose => {
                assert!(
                    output
                        .xml
                        .contains("<!-- test_kitchen_sink.abc:12: |: CDEF GABc | cBAG FEDC :| -->"),
                    "Level 2 should emit source line comments"
                );
                assert!(
                    output.xml.contains("<!-- @12:"),
                    "Level 2 should emit element span comments"
                );
            }
        }
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn converts_kitchen_sink_tune_2_all_annotation_levels() {
    let source = fs::read_to_string(KITCHEN_SINK_PATH).expect("reading kitchen sink file");
    let document = parse_abc_doc(&source);
    let tunes: Vec<_> = document.tunes().collect();
    let tune = tunes[1];

    for level in [
        AnnotationLevel::None,
        AnnotationLevel::Line,
        AnnotationLevel::Verbose,
    ] {
        let options = MusicXmlOptions::new()
            .with_source_name("test_kitchen_sink.abc")
            .with_annotation_level(level);

        let output = tune_to_musicxml(tune, &document.header, Some(&source), &options)
            .expect("tune 2 lowering should succeed");

        validate_with_xmllint(MUSICXML_SCHEMA_PATH, &output.xml);
        assert_strict_element_ordering(&output.xml);
        assert_no_illegal_hyphens_in_comments(&output.xml);

        // Musical semantics
        assert!(output.xml.contains("<movement-number>2</movement-number>"));
        assert!(
            output
                .xml
                .contains("<movement-title>Test Tune 2: The ABC 2.1 Kitchen Sink</movement-title>")
        );
        assert!(
            output
                .xml
                .contains("<creator type=\"composer\">Maurice Barnum &amp; AI</creator>")
        );
        assert!(output.xml.contains("<part-name>Soprano / Bass</part-name>"));
        assert!(
            output
                .xml
                .contains("<part-abbreviation>S / B</part-abbreviation>")
        );
        assert!(output.xml.contains("<mode>mixolydian</mode>"));
        assert!(output.xml.contains("<per-minute>100</per-minute>"));
        assert!(output.xml.contains("<words>Majestic</words>"));

        // Polyphonic merging & backup
        assert!(output.xml.contains("<voice>1</voice>"));
        assert!(output.xml.contains("<voice>2</voice>"));
        assert!(output.xml.contains("<backup>"));

        // Accidentals
        assert!(output.xml.contains("<accidental>flat</accidental>"));
        assert!(output.xml.contains("<accidental>natural</accidental>"));
        assert!(output.xml.contains("<accidental>sharp</accidental>"));

        // Lyrics
        assert!(output.xml.contains("<syllabic>begin</syllabic>"));
        assert!(output.xml.contains("<syllabic>end</syllabic>"));
        assert!(output.xml.contains("<syllabic>single</syllabic>"));
        assert!(output.xml.contains("<text>Ho</text>"));
        assert!(output.xml.contains("<text>Mo</text>"));
        assert!(output.xml.contains("<text>ly!</text>"));

        // Chords
        assert!(output.xml.contains("<chord/>"));

        // Variant endings
        assert!(output.xml.contains("<ending number=\"1\" type=\"start\""));
        assert!(output.xml.contains("<ending number=\"2\" type=\"start\""));

        // Grace notes
        assert!(output.xml.contains("<grace/>"));

        // Tuplets
        assert!(
            output
                .xml
                .contains("<tuplet type=\"start\" number=\"1\" bracket=\"yes\"/>")
        );
        assert!(output.xml.contains("<actual-notes>3</actual-notes>"));

        // Slurs
        assert!(output.xml.contains("<slur type=\"start\" number=\"1\"/>"));
        assert!(output.xml.contains("<slur type=\"start\" number=\"2\"/>"));

        // Ornaments, articulations, dynamics
        assert!(output.xml.contains("<trill-mark/>"));
        assert!(output.xml.contains("<turn/>"));
        assert!(output.xml.contains("<accent/>"));
        assert!(output.xml.contains("<staccato/>"));
        assert!(output.xml.contains("<ppp/>"));

        // Ties
        assert!(output.xml.contains("<tie type=\"start\"/>"));
        assert!(output.xml.contains("<tie type=\"stop\"/>"));
        assert!(output.xml.contains("<tied type=\"start\"/>"));
        assert!(output.xml.contains("<tied type=\"stop\"/>"));

        // Unaligned trailing lyrics in miscellaneous
        assert!(
            output
                .xml
                .contains("<miscellaneous-field name=\"words\">This is an unaligned lyric line at the very end of the tune</miscellaneous-field>")
        );

        // Provenance annotations
        match level {
            AnnotationLevel::None => {
                assert!(
                    !output.xml.contains("<!-- test_kitchen_sink.abc:")
                        && !output.xml.contains("<!-- @"),
                    "Level 0 should emit no source provenance comments"
                );
            }
            AnnotationLevel::Line => {
                assert!(
                    output.xml.contains("<!-- test_kitchen_sink.abc:29: "),
                    "Level 1 should emit source line comments"
                );
                assert!(
                    !output.xml.contains("<!-- @29:"),
                    "Level 1 should not emit element span comments"
                );
            }
            AnnotationLevel::Verbose => {
                assert!(
                    output.xml.contains("<!-- test_kitchen_sink.abc:29: "),
                    "Level 2 should emit source line comments"
                );
                assert!(
                    output.xml.contains("<!-- @29:"),
                    "Level 2 should emit element span comments"
                );
            }
        }
    }
}

#[test]
fn batch_out_dir_validates_opus_and_tune_schemas() {
    let dir = std::env::temp_dir().join(format!("abc-batch-test-{}", std::process::id()));
    let output = run_cli(&[
        KITCHEN_SINK_PATH,
        "--out-dir",
        dir.to_str().unwrap(),
        "-a",
        "verbose",
    ]);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let opus_path = dir.join("opus.xml");
    assert!(opus_path.exists(), "opus.xml should be generated");
    let opus_xml = fs::read_to_string(&opus_path).expect("reading opus.xml");
    validate_with_xmllint(OPUS_SCHEMA_PATH, &opus_xml);
    assert_no_illegal_hyphens_in_comments(&opus_xml);
    assert!(
        opus_xml.contains("<opus version=\"4.0\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">")
    );
    assert!(opus_xml.contains("<score xlink:href=\"test_kitchen_sink-1.musicxml\"/>"));
    assert!(opus_xml.contains("<score xlink:href=\"test_kitchen_sink-2.musicxml\"/>"));

    let tune1_path = dir.join("test_kitchen_sink-1.musicxml");
    assert!(tune1_path.exists());
    let tune1_xml = fs::read_to_string(&tune1_path).expect("reading test_kitchen_sink-1.musicxml");
    assert!(tune1_xml.contains("<movement-number>1</movement-number>"));
    validate_with_xmllint(MUSICXML_SCHEMA_PATH, &tune1_xml);
    assert_strict_element_ordering(&tune1_xml);

    let tune2_path = dir.join("test_kitchen_sink-2.musicxml");
    assert!(tune2_path.exists());
    let tune2_xml = fs::read_to_string(&tune2_path).expect("reading test_kitchen_sink-2.musicxml");
    assert!(tune2_xml.contains("<movement-number>2</movement-number>"));
    validate_with_xmllint(MUSICXML_SCHEMA_PATH, &tune2_xml);
    assert_strict_element_ordering(&tune2_xml);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn modal_keys_bagpipe_and_microtones_schema_validation() {
    let abc_src = "X:1\nT:Modal and Bagpipe\nM:6/8\nL:1/8\nK:HP\n{g}A2 d e2 f | g2 e d3 |]\n\
                   X:2\nT:Microtones and Modes\nM:4/4\nL:1/4\nK:D dorian\n^/c _3/2d =e F | G A B c |]\n";

    let document = parse_abc_doc(abc_src);
    for tune in document.tunes() {
        for level in [AnnotationLevel::None, AnnotationLevel::Verbose] {
            let options = MusicXmlOptions::new().with_annotation_level(level);
            let output = tune_to_musicxml(tune, &document.header, Some(abc_src), &options)
                .expect("lowering should succeed");

            validate_with_xmllint(MUSICXML_SCHEMA_PATH, &output.xml);
            assert_strict_element_ordering(&output.xml);
            assert_no_illegal_hyphens_in_comments(&output.xml);
        }
    }
}

#[test]
fn multi_measure_rests_and_clef_variations_schema_validation() {
    let abc_src = "X:1\nT:Multi Measure Rest and Clefs\nM:3/4\nL:1/4\nV:1 clef=alto\nV:2 clef=bass+8\n\
                   [V:1] Z4 | c d e |]\n\
                   [V:2] X | A,, B,, C, |]\n";

    let document = parse_abc_doc(abc_src);
    let tune = document.tunes().next().expect("tune 1");
    let options = MusicXmlOptions::new().with_annotation_level(AnnotationLevel::Verbose);
    let output = tune_to_musicxml(tune, &document.header, Some(abc_src), &options)
        .expect("lowering should succeed");

    validate_with_xmllint(MUSICXML_SCHEMA_PATH, &output.xml);
    assert_strict_element_ordering(&output.xml);
    assert_no_illegal_hyphens_in_comments(&output.xml);

    // Verify multiple parts with alto and bass+8 clefs
    assert!(output.xml.contains("<sign>C</sign>"));
    assert!(
        output
            .xml
            .contains("<clef-octave-change>1</clef-octave-change>")
    );
    assert!(output.xml.contains("<multiple-rest>4</multiple-rest>"));
}
