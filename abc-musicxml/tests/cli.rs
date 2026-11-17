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

//! End-to-end integration tests for `abc-musicxml` CLI.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

const KITCHEN_SINK_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_kitchen_sink.abc");

fn run_file(file_path: &str, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_abc-musicxml"))
        .arg(file_path)
        .args(arguments)
        .output()
        .expect("abc-musicxml should execute")
}

fn run_stdin(source: &str, arguments: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_abc-musicxml"))
        .arg("-")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("abc-musicxml should spawn");

    child
        .stdin
        .as_mut()
        .expect("stdin handle should be open")
        .write_all(source.as_bytes())
        .expect("writing ABC to stdin should succeed");

    child
        .wait_with_output()
        .expect("abc-musicxml child process should complete")
}

#[test]
fn converts_single_tune_to_stdout() {
    let src = "X:1\nT:Single Tune\nM:4/4\nL:1/4\nK:C\nc d e f |\n";
    let output = run_stdin(src, &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("<score-partwise version=\"4.0\">"));
    assert!(stdout.contains("<movement-title>Single Tune</movement-title>"));
    assert!(stdout.contains("<step>C</step>"));
}

#[test]
fn converts_single_tune_to_file() {
    let path =
        std::env::temp_dir().join(format!("abc-musicxml-out-{}.musicxml", std::process::id()));
    let src = "X:1\nT:Output File\nM:4/4\nL:1/4\nK:C\nc d e f |\n";
    let output = run_stdin(src, &["--out", path.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("<movement-title>Output File</movement-title>"));
    fs::remove_file(&path).ok();
}

#[test]
fn multi_tune_without_flags_fails_listing_available_tunes() {
    let output = run_file(KITCHEN_SINK_PATH, &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("document contains multiple tunes"));
    assert!(stderr.contains("--tune <X> or --out-dir <DIR>"));
    assert!(stderr.contains("[1, 2]"));
}

#[test]
fn multi_tune_selects_specific_tune() {
    let output1 = run_file(KITCHEN_SINK_PATH, &["--tune", "1"]);
    assert!(output1.status.success());
    let stdout1 = String::from_utf8(output1.stdout).unwrap();
    assert!(stdout1.contains("<movement-number>1</movement-number>"));
    assert!(
        stdout1.contains("<movement-title>Test Tune 1: File Separation Boundary</movement-title>")
    );

    let output2 = run_file(KITCHEN_SINK_PATH, &["-x", "2"]);
    assert!(output2.status.success());
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert!(stdout2.contains("<movement-number>2</movement-number>"));
    assert!(
        stdout2.contains("<movement-title>Test Tune 2: The ABC 2.1 Kitchen Sink</movement-title>")
    );
}

#[test]
fn multi_tune_unknown_tune_fails() {
    let output = run_file(KITCHEN_SINK_PATH, &["--tune", "99"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("tune X:99 not found"));
    assert!(stderr.contains("[1, 2]"));
}

#[test]
fn stdin_out_dir_uses_tune_stem() {
    let dir = std::env::temp_dir().join(format!("abc-musicxml-stdin-dir-{}", std::process::id()));
    let src = concat!(
        "X:1\nT:First\nM:4/4\nL:1/4\nK:C\nc4 |\n\n",
        "X:2\nT:Second\nM:4/4\nL:1/4\nK:C\ne4 |\n",
    );
    let output = run_stdin(src, &["--out-dir", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let tune1_path = dir.join("tune-1.musicxml");
    let tune2_path = dir.join("tune-2.musicxml");
    let opus_path = dir.join("opus.xml");

    assert!(tune1_path.exists());
    assert!(tune2_path.exists());
    assert!(opus_path.exists());

    let opus_content = fs::read_to_string(&opus_path).unwrap();
    assert!(opus_content.contains("<score xlink:href=\"tune-1.musicxml\"/>"));
    assert!(opus_content.contains("<score xlink:href=\"tune-2.musicxml\"/>"));

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_doctype_flag_omits_declaration() {
    let src = "X:1\nT:No Doctype\nM:4/4\nL:1/4\nK:C\nc4 |\n";
    let with_doctype = run_stdin(src, &[]);
    let without_doctype = run_stdin(src, &["--no-doctype"]);

    assert!(with_doctype.status.success());
    assert!(without_doctype.status.success());

    let with_str = String::from_utf8(with_doctype.stdout).unwrap();
    let without_str = String::from_utf8(without_doctype.stdout).unwrap();

    assert!(with_str.contains("<!DOCTYPE score-partwise"));
    assert!(!without_str.contains("<!DOCTYPE score-partwise"));
}

#[test]
fn annotate_flags_emit_source_provenance_comments() {
    let src = "X:1\nT:Annotate\nM:4/4\nL:1/4\nK:C\nc d e f |\n";
    let none = run_stdin(src, &[]);
    let line = run_stdin(src, &["--annotate", "line"]);
    let verbose = run_stdin(src, &["--annotate", "verbose"]);

    assert!(none.status.success());
    assert!(line.status.success());
    assert!(verbose.status.success());

    let none_str = String::from_utf8(none.stdout).unwrap();
    let line_str = String::from_utf8(line.stdout).unwrap();
    let verbose_str = String::from_utf8(verbose.stdout).unwrap();

    assert!(!none_str.contains("<!-- <stdin>:"));
    assert!(line_str.contains("<!-- <stdin>:6: c d e f | -->"));
    assert!(verbose_str.contains("<!-- <stdin>:6: c d e f | -->"));
    assert!(verbose_str.contains(": c -->\n      <note>"));
}

#[test]
fn recover_flag_converts_recoverable_syntax_errors() {
    let invalid_src = "X:1\nT:Invalid\nM:4/4\nL:1/4\nK:C\nc d [invalid syntax! e f |\n";

    let strict = run_stdin(invalid_src, &[]);
    assert!(!strict.status.success());
    let stderr = String::from_utf8(strict.stderr).unwrap();
    assert!(stderr.contains("input is not valid ABC:"));

    let recovered = run_stdin(invalid_src, &["--recover"]);
    assert!(
        recovered.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    let stdout = String::from_utf8(recovered.stdout).unwrap();
    assert!(stdout.contains("<score-partwise version=\"4.0\">"));
}

#[test]
fn conflicting_flags_are_rejected_by_clap() {
    let output1 = run_file(KITCHEN_SINK_PATH, &["--tune", "1", "--out-dir", "/tmp"]);
    assert!(!output1.status.success());
    let stderr1 = String::from_utf8(output1.stderr).unwrap();
    assert!(stderr1.contains("cannot be used with"));

    let output2 = run_file(
        KITCHEN_SINK_PATH,
        &["--out", "file.xml", "--out-dir", "/tmp"],
    );
    assert!(!output2.status.success());
    let stderr2 = String::from_utf8(output2.stderr).unwrap();
    assert!(stderr2.contains("cannot be used with"));
}

#[test]
fn out_dir_disambiguates_duplicate_tune_filenames() {
    let dir = std::env::temp_dir().join(format!("abc-musicxml-dup-dir-{}", std::process::id()));
    let src = concat!(
        "X:2\nT:Numbered Two\nM:4/4\nL:1/4\nK:C\nc4 |\n\n",
        "X:2\nT:Also Two\nM:4/4\nL:1/4\nK:C\ne4 |\n",
    );
    let output = run_stdin(src, &["--out-dir", dir.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let file1 = dir.join("tune-2.musicxml");
    let file2 = dir.join("tune-2-2.musicxml");
    assert!(file1.exists(), "first tune file must exist");
    assert!(file2.exists(), "disambiguated second tune file must exist");

    let text1 = fs::read_to_string(&file1).unwrap();
    let text2 = fs::read_to_string(&file2).unwrap();
    assert!(text1.contains("<movement-title>Numbered Two</movement-title>"));
    assert!(text2.contains("<movement-title>Also Two</movement-title>"));

    let opus = fs::read_to_string(dir.join("opus.xml")).unwrap();
    assert!(opus.contains("<score xlink:href=\"tune-2.musicxml\"/>"));
    assert!(opus.contains("<score xlink:href=\"tune-2-2.musicxml\"/>"));

    fs::remove_dir_all(&dir).ok();
}
