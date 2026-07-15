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

use abc_parser::FieldValue;
use abc_parser::Line;
use abc_parser::ToAbc;
use abc_parser::parse_recovering;
use std::fs;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;

const KITCHEN_SINK: &str = include_str!("../test_kitchen_sink.abc");
const KITCHEN_SINK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test_kitchen_sink.abc");

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
    assert!(help.contains("--out <FILE>"), "{help}");
    assert!(help.contains("Possible values:"), "{help}");
}

#[test]
fn out_writes_the_transposed_document_to_a_file() {
    let path =
        std::env::temp_dir().join(format!("abc-transpose-output-{}.abc", std::process::id()));
    let output = run(&["--semitones", "1", "--out", path.to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());

    let source = fs::read_to_string(&path).unwrap();
    fs::remove_file(path).unwrap();
    let reparsed = parse_recovering(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert_eq!(reparsed.output.tunes().count(), 2);
}

#[test]
fn zero_semitones_are_a_byte_preserving_no_op() {
    let output = run(&["--semitones", "0"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, KITCHEN_SINK.as_bytes());
}

#[test]
fn octave_accepts_positive_and_negative_values() {
    let upward = run(&["--semitones", "0", "--octave", "1"]);
    let downward = run(&["--semitones", "0", "--octave", "-1"]);
    assert!(upward.status.success(), "{upward:?}");
    assert!(downward.status.success(), "{downward:?}");
    assert_ne!(upward.stdout, KITCHEN_SINK.as_bytes());
    assert_ne!(downward.stdout, KITCHEN_SINK.as_bytes());
    assert_ne!(upward.stdout, downward.stdout);
}

#[test]
fn half_a_step_matches_one_semitone_and_emits_valid_abc() {
    let steps = run(&["--steps", "0.5"]);
    let semitone = run(&["--semitones", "1"]);
    assert!(steps.status.success());
    assert_eq!(steps.stdout, semitone.stdout);

    let source = String::from_utf8(steps.stdout).unwrap();
    let reparsed = parse_recovering(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert_eq!(reparsed.output.tunes().count(), 2);
}

#[test]
fn destination_key_is_applied_independently_to_every_tune() {
    let output = run(&["--key", "Dm"]);
    assert!(output.status.success());
    let source = String::from_utf8(output.stdout).unwrap();
    let reparsed = parse_recovering(&source);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);

    for tune in reparsed.output.tunes() {
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
        let reparsed = parse_recovering(&source);
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
fn malformed_music_reports_the_enclosing_production() {
    let output = run_stdin("X:1\nK:C\n[CEG\nC |\n", &["--semitones", "1"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("<stdin>:3:5: found end of input"), "{error}");
    assert!(error.contains("while parsing chord"), "{error}");
    assert!(error.contains("3 | [CEG"), "{error}");
    assert!(error.contains("|     ^"), "{error}");
}
