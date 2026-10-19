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

//! End-to-end tests for the standalone ABC transposition command.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;

use abc_parser::FieldValue;
use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::OwnedDocument;
use abc_parser::ParseReport;
use abc_parser::ToAbc;
use abc_parser::parse;
use chumsky::span::SimpleSpan;

const KITCHEN_SINK: &str = include_str!("../../../test_kitchen_sink.abc");
const KITCHEN_SINK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_kitchen_sink.abc");

fn parse_owned(source: &str) -> ParseReport<OwnedDocument<SimpleSpan<usize>>, SimpleSpan<usize>> {
    let report = parse(source);
    ParseReport {
        output: report
            .output
            .map(|document| document.into_owned(source).unwrap()),
        errors: report.errors,
        warnings: report.warnings,
    }
}

/// Runs the built command with the repository fixture as input.
fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_abc-transpose"))
        .arg(KITCHEN_SINK_PATH)
        .args(arguments)
        .output()
        .unwrap()
}

/// Runs the built command with ABC supplied through standard input.
fn run_stdin(source: &str, arguments: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_abc-transpose"))
        .arg("-")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn help_describes_transposition_and_spelling_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_abc-transpose"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage: abc-transpose"), "{help}");
    assert!(help.contains("--key <KEY>"), "{help}");
    assert!(help.contains("--semitones <N>"), "{help}");
    assert!(help.contains("--steps <N>"), "{help}");
    assert!(help.contains("--octave <OCTAVE>"), "{help}");
    assert!(help.contains("[default: 0]"), "{help}");
    assert!(help.contains("--prefer-flats <BOOL>"), "{help}");
    assert!(help.contains("--explicit-note-lengths"), "{help}");
    assert!(help.contains("--out <FILE>"), "{help}");
    assert!(help.contains("Possible values:"), "{help}");
}

#[test]
fn out_writes_the_transposed_document_to_a_file() {
    let path =
        std::env::temp_dir().join(format!("abc-transpose-output-{}.abc", std::process::id()));
    let output = run(&["--semitones", "1", "--out", path.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"");

    let source = fs::read_to_string(&path).unwrap();
    fs::remove_file(path).unwrap();
    let reparsed = parse_owned(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert_eq!(reparsed.output.as_ref().unwrap().tunes().count(), 2);
}

#[test]
fn zero_semitones_are_a_byte_preserving_no_op() {
    let output = run(&["--semitones", "0"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, KITCHEN_SINK.as_bytes());
}

#[test]
fn output_guarantees_exactly_one_final_newline() {
    for (source, had_trailing_newline) in [("X:1\nK:C\nC |", false), ("X:1\nK:C\nC |\n", true)] {
        for args in [&["--semitones", "0"][..], &["--semitones", "1"][..]] {
            let output = run_stdin(source, args);
            assert!(
                output.status.success(),
                "source (newline={had_trailing_newline}), {args:?}: {output:?}"
            );
            assert!(
                output.stdout.ends_with(b"\n"),
                "missing final newline for {args:?}"
            );
            assert!(
                !output.stdout.ends_with(b"\n\n"),
                "redundant trailing newline for {args:?}"
            );
            if args == ["--semitones", "0"] {
                assert_eq!(output.stdout, b"X:1\nK:C\nC |\n");
            }
        }
    }
}

#[test]
fn trailing_blank_line_is_accepted() {
    let source = "X: 1\nM: 6/8\nK: Bmin\nF/E/|D>EF d2d/e/|f/e/d/c/B/A/ d2\n\n";
    let output = run_stdin(source, &["--steps", "1"]);

    assert!(output.status.success(), "{output:?}");
    let transposed = String::from_utf8(output.stdout).unwrap();
    let reparsed = parse_owned(&transposed);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
}

#[test]
fn transposition_note_length_formatting() {
    let zero_source = "X: 1\nL: 1/8\nK: C\nA/\n";
    let default = run_stdin(zero_source, &["--semitones", "0"]);
    assert!(default.status.success(), "{default:?}");
    assert_eq!(default.stdout, zero_source.as_bytes());

    let explicit_zero = run_stdin(
        zero_source,
        &["--semitones", "0", "--explicit-note-lengths"],
    );
    assert!(explicit_zero.status.success(), "{explicit_zero:?}");
    assert_eq!(
        String::from_utf8(explicit_zero.stdout).unwrap(),
        "X:1\nL:1/8\nK:C\nA/2\n"
    );

    let tune_source = "X:1\nM:6/8\nL:1/8\nK:Bmin\nF/E/|D>EF d2d/e/|\n";
    let shorthand = run_stdin(tune_source, &["--key", "Em"]);
    assert!(shorthand.status.success(), "{shorthand:?}");
    let shorthand_text = String::from_utf8(shorthand.stdout).unwrap();
    assert!(
        shorthand_text.contains("B/A/|G>AB g2g/a/|"),
        "{shorthand_text}"
    );

    let explicit = run_stdin(tune_source, &["--key", "Em", "--explicit-note-lengths"]);
    assert!(explicit.status.success(), "{explicit:?}");
    let explicit_text = String::from_utf8(explicit.stdout).unwrap();
    assert!(
        explicit_text.contains("B/2A/2|G>AB g2g/2a/2|"),
        "{explicit_text}"
    );

    for output in [&shorthand_text, &explicit_text] {
        let reparsed = parse_owned(output);
        assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    }
}

#[test]
fn octave_accepts_positive_and_negative_values() {
    let upward = run(&["--semitones", "0", "--octave", "1"]);
    let downward = run(&["--semitones", "0", "--octave", "-1"]);
    assert!(upward.status.success(), "{upward:?}");
    assert!(downward.status.success(), "{downward:?}");

    let up_source = String::from_utf8(upward.stdout).unwrap();
    let down_source = String::from_utf8(downward.stdout).unwrap();
    let up_reparsed = parse_owned(&up_source);
    assert!(up_reparsed.is_valid(), "{:#?}", up_reparsed.errors);
    let down_reparsed = parse_owned(&down_source);
    assert!(down_reparsed.is_valid(), "{:#?}", down_reparsed.errors);

    let compact_up = run_stdin(
        "X:1\nK:C\nC [EG] {B} |\n",
        &["--semitones", "0", "--octave", "1"],
    );
    assert!(compact_up.status.success());
    let compact_up_text = String::from_utf8(compact_up.stdout).unwrap();
    assert!(
        compact_up_text.contains("c [eg] {b} |"),
        "{compact_up_text}"
    );

    let compact_down = run_stdin(
        "X:1\nK:C\nC [EG] {B} |\n",
        &["--semitones", "0", "--octave", "-1"],
    );
    assert!(compact_down.status.success());
    let compact_down_text = String::from_utf8(compact_down.stdout).unwrap();
    assert!(
        compact_down_text.contains("C, [E,G,] {B,} |"),
        "{compact_down_text}"
    );
}

#[test]
fn half_a_step_matches_one_semitone_and_emits_valid_abc() {
    let steps = run(&["--steps", "0.5"]);
    let semitone = run(&["--semitones", "1"]);
    assert!(steps.status.success());
    assert_eq!(steps.stdout, semitone.stdout);

    let source = String::from_utf8(steps.stdout).unwrap();
    let reparsed = parse_owned(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert_eq!(reparsed.output.as_ref().unwrap().tunes().count(), 2);
}

#[test]
fn destination_key_is_applied_independently_to_every_tune() {
    let output = run(&["--key", "Dm"]);
    assert!(output.status.success());
    let source = String::from_utf8(output.stdout).unwrap();
    let reparsed = parse_owned(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);

    for tune in reparsed.output.as_ref().unwrap().tunes() {
        let key = tune
            .lines
            .iter()
            .find_map(|line| match &line.value {
                Line::Field(field) => match &field.value {
                    FieldValue::Key(key) => Some(key),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        assert_eq!(key.to_abc(), "Dm");
    }
}

#[test]
fn spelling_preference_values_are_accepted_by_the_command() {
    for value in ["true", "false", "auto"] {
        let output = run(&["--semitones", "1", "--prefer-flats", value]);
        assert!(output.status.success(), "{value}: {output:?}");
        let source = String::from_utf8(output.stdout).unwrap();
        let reparsed = parse_owned(&source);
        assert!(reparsed.is_valid(), "{value}: {:#?}", reparsed.errors);
    }
}

#[test]
fn non_half_step_values_are_rejected() {
    let output = run(&["--steps", "0.25"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("exact multiple of 0.5"));
}

#[test]
fn invalid_input_reports_source_context() {
    let output = run_stdin("X:1\nM:6/x\nK:C\nC |\n", &["--semitones", "1"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("<stdin>:2:5: found 'x' expected integer"),
        "{error}"
    );
    assert!(error.contains("while parsing M: meter"), "{error}");
    assert!(error.contains("2 | M:6/x"), "{error}");
    assert!(error.contains("|     ^"), "{error}");
}

#[test]
fn music_like_free_text_warns_without_failing() {
    let source = "% comment\nCDEF |\n";
    let output = run_stdin(source, &["--semitones", "0"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, source.as_bytes());
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.contains("abc-transpose: warning: <stdin>:2:1"),
        "{warning}"
    );
    assert!(warning.contains("treating it as free text"), "{warning}");
    assert!(warning.contains("2 | CDEF |"), "{warning}");
}

#[test]
fn malformed_music_reports_the_enclosing_production() {
    let output = run_stdin("X:1\nK:C\n[CEG\nC |\n", &["--semitones", "1"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("<stdin>:3:5: found end of line"), "{error}");
    assert!(error.contains("while parsing chord"), "{error}");
    assert!(error.contains("3 | [CEG"), "{error}");
    assert!(error.contains("|     ^"), "{error}");
}
