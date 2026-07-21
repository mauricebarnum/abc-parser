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

//! End-to-end tests for the standalone ABC lint command.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use abc_parser::ErrorKind;
use abc_parser::FieldValue;
use abc_parser::Line;
use abc_parser::ParserOptions;
use abc_parser::parse_with_options;

/// Runs the built command with ABC supplied through standard input.
fn run_stdin(source: &str, arguments: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_abc-lint"))
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

/// Returns successful fixed output as UTF-8 text.
fn fix(source: &str) -> String {
    let output = run_stdin(source, &["--fix"]);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// Returns the physical field keys preceding the first music line.
fn header_field_keys(source: &str) -> Vec<char> {
    let report = parse_with_options(source, ParserOptions::new().strict(true));
    assert!(report.is_valid(), "{:#?}", report.errors);
    report
        .output
        .unwrap()
        .tunes()
        .next()
        .unwrap()
        .lines
        .iter()
        .take_while(|line| !matches!(line.value, Line::Music(_)))
        .filter_map(|line| match &line.value {
            Line::Field(field) => Some(field.key),
            _ => None,
        })
        .collect()
}

/// Returns reference values in tune order.
fn references(source: &str) -> Vec<u32> {
    let report = parse_with_options(source, ParserOptions::new().strict(true));
    assert!(report.is_valid(), "{:#?}", report.errors);
    report
        .output
        .unwrap()
        .tunes()
        .filter_map(|tune| {
            tune.lines.iter().find_map(|line| match &line.value {
                Line::Field(field) => match field.value {
                    FieldValue::Reference(value) => Some(value),
                    _ => None,
                },
                _ => None,
            })
        })
        .collect()
}

#[test]
fn help_describes_validation_and_fix_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_abc-lint"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage: abc-lint"), "{help}");
    assert!(help.contains("--fix"), "{help}");
    assert!(help.contains("--out <FILE>"), "{help}");
}

#[test]
fn valid_input_succeeds_without_emitting_a_document() {
    let output = run_stdin("X:1\nT:Valid\nK:C\nCDEF |\n", &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn invalid_input_reports_source_context() {
    let output = run_stdin("X:1\nT:Invalid\nM:6/x\nK:C\nCDEF |\n", &[]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("input is not valid ABC 2.1"), "{error}");
    assert!(
        error.contains("<stdin>:3:5: found 'x' expected integer"),
        "{error}"
    );
    assert!(error.contains("while parsing M: meter"), "{error}");
}

#[test]
fn deprecated_syntax_warns_without_failing_or_rewriting() {
    let source = "X:1\nT:Deprecated\nA:Donegal\nE:1.2\nQ:C=120\nK:C\nCDEF |\n";
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let warnings = String::from_utf8(output.stderr).unwrap();
    assert!(warnings.contains("deprecated A: area field"), "{warnings}");
    assert!(
        warnings.contains("deprecated E: element-spacing field"),
        "{warnings}"
    );
    assert!(
        warnings.contains("deprecated Q: tempo syntax"),
        "{warnings}"
    );
}

#[test]
fn out_of_order_recommended_fields_warn_without_fixing() {
    let source = "X:1\nT:Order\nL:1/8\nM:4/4\nK:C\nCDEF |\n";
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.contains(
            "M: field should appear before L: field in the recommended tune-header order"
        ),
        "{warning}"
    );

    let source = "X:1\nT:Order\nM:4/4\nL:1/8\nC:Composer\nK:C\nCDEF |\n";
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.contains(
            "C: field should appear before M: field in the recommended tune-header order"
        ),
        "{warning}"
    );
}

#[test]
fn overridden_header_fields_warn_without_fixing() {
    let source = "X:1\nT:Overrides\nM:2/4\nM:4/4\nK:C\nCDEF |\n";
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.contains("earlier M: field is overridden by a later definition"),
        "{warning}"
    );
}

#[test]
fn every_kind_of_fixer_change_has_a_default_diagnostic() {
    let source = "X:\nT:Fixable\nH:remove me\nK:C\n+turn+C |\n";
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    let warnings = String::from_utf8(output.stderr).unwrap();
    assert!(warnings.contains("empty X: field"), "{warnings}");
    assert!(warnings.contains("H: field will be removed"), "{warnings}");
    assert!(
        warnings.contains("deprecated +name+ decoration"),
        "{warnings}"
    );

    let noncanonical = run_stdin("X: 1\nT: Spacing\nK: C\nCDEF |\n", &[]);
    assert!(noncanonical.status.success(), "{noncanonical:?}");
    let warnings = String::from_utf8(noncanonical.stderr).unwrap();
    assert!(warnings.contains("<stdin>:1:3:"), "{warnings}");
    assert!(
        warnings.contains(
            "document is not in canonical form; first difference: --fix expects '1', found ' '"
        ),
        "{warnings}"
    );

    let shorthand = run_stdin("X:1\nT:Shorthand\nK:C\nC :|2 D |\n", &[]);
    assert!(shorthand.status.success(), "{shorthand:?}");
    let warnings = String::from_utf8(shorthand.stderr).unwrap();
    assert!(warnings.is_empty(), "{warnings}");
}

#[test]
fn fix_updates_all_deprecated_information_field_forms() {
    let source = concat!(
        "X:1\n",
        "T:Fix fields\n",
        "M:2/4\n",
        "A:Donegal\n",
        "E:1.2\n",
        "+:more spacing\n",
        "% retained between continuations\n",
        "+:still more spacing\n",
        "Q:C=120\n",
        "H:old history\n",
        "this is an implicit history continuation\n",
        "K:C\n",
        "[A:Clare] [E:2] [H:discard] [Q:240] C +turn+D |\n",
    );
    let fixed = fix(source);

    assert!(fixed.contains("O:Donegal\n"), "{fixed}");
    assert!(fixed.contains("Q:1/16=120\n"), "{fixed}");
    assert!(fixed.contains("[O:Clare]"), "{fixed}");
    assert!(fixed.contains("[Q:1/16=240]"), "{fixed}");
    assert!(fixed.contains("!turn!"), "{fixed}");
    assert!(
        fixed.contains("% retained between continuations"),
        "{fixed}"
    );
    for removed in [
        "A:Donegal",
        "E:1.2",
        "more spacing",
        "H:old history",
        "implicit history continuation",
        "[A:Clare]",
        "[E:2]",
        "[H:discard]",
        "+turn+",
    ] {
        assert!(
            !fixed.contains(removed),
            "unexpected {removed:?} in {fixed}"
        );
    }

    let reparsed = parse_with_options(fixed.as_str(), ParserOptions::new().strict(true));
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert!(
        reparsed
            .warnings
            .iter()
            .all(|warning| warning.kind != ErrorKind::DeprecatedSyntax),
        "{:#?}",
        reparsed.warnings
    );
}

#[test]
fn explicit_and_inline_unit_lengths_control_deprecated_tempo_fixes() {
    let source = concat!(
        "X:1\n",
        "T:Explicit length\n",
        "M:2/4\n",
        "L:1/32\n",
        "Q:120\n",
        "K:C\n",
        "[L:1/64][Q:C=240] C |\n",
    );
    let fixed = fix(source);
    assert!(fixed.contains("Q:1/32=120\n"), "{fixed}");
    assert!(fixed.contains("[L:1/64][Q:1/64=240]"), "{fixed}");
}

#[test]
fn fix_stably_applies_the_recommended_header_order() {
    let source = concat!(
        "T:Ordering\n",
        "Q:1/4=90\n",
        "W:Words\n",
        "L:1/8\n",
        "V:1\n",
        "N:Background\n",
        "P:AB\n",
        "I:linebreak $\n",
        "R:reel\n",
        "O:Ireland\n",
        "C:Traditional\n",
        "M:4/4\n",
        "X:9\n",
        "K:C\n",
        "CDEF |\n",
    );
    let fixed = fix(source);
    assert_eq!(
        header_field_keys(&fixed),
        vec![
            'X', 'T', 'C', 'O', 'R', 'N', 'I', 'P', 'V', 'M', 'L', 'Q', 'W', 'K'
        ]
    );
}

#[test]
fn reordered_fields_keep_their_continuations() {
    let source = concat!(
        "X:1\n",
        "T:Continuations\n",
        "M:4/4\n",
        "C:First half\n",
        "+:second half\n",
        "K:C\n",
        "CDEF |\n",
    );
    let fixed = fix(source);
    assert!(
        fixed.contains("C:First half\n+:second half\nM:4/4"),
        "{fixed}"
    );
}

#[test]
fn interspersed_continuation_movement_has_a_diagnostic() {
    let source = concat!(
        "X:1\n",
        "T:Continuation warning\n",
        "C:first half\n",
        "% between field and continuation\n",
        "+:second half\n",
        "K:C\n",
        "CDEF |\n",
    );
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    let warning = String::from_utf8(output.stderr).unwrap();
    assert!(
        warning.contains("continuation will move next to its field"),
        "{warning}"
    );
}

#[test]
fn fix_removes_only_superseded_header_instructions() {
    let source = concat!(
        "X:1\n",
        "T:Primary title\n",
        "T:Alternate title\n",
        "C:First composer\n",
        "C:Second composer\n",
        "I:linebreak $\n",
        "I:linebreak <none>\n",
        "I:decoration !\n",
        "m:~G2={A}G\n",
        "m:~G2={B}G\n",
        "U:T=!trill!\n",
        "U:T=!turn!\n",
        "V:one name=First\n",
        "V:one name=Replacement\n",
        "V:two name=Second\n",
        "P:AB\n",
        "P:BA\n",
        "M:2/4\n",
        "M:4/4\n",
        "L:1/16\n",
        "L:1/8\n",
        "Q:1/4=60\n",
        "Q:1/4=120\n",
        "K:G\n",
        "K:C\n",
        "CDEF |\n",
    );
    let fixed = fix(source);

    for removed in [
        "I:linebreak $",
        "m:~G2={A}G",
        "U:T=!trill!",
        "V:one name=First",
        "P:AB",
        "M:2/4",
        "L:1/16",
        "Q:1/4=60",
        "K:G",
    ] {
        assert!(
            !fixed.contains(removed),
            "unexpected {removed:?} in {fixed}"
        );
    }
    for retained in [
        "T:Primary title",
        "T:Alternate title",
        "C:First composer",
        "C:Second composer",
        "I:linebreak <none>",
        "I:decoration !",
        "m:~G2={B}G",
        "U:T=!turn!",
        "V:one name=Replacement",
        "V:two name=Second",
        "P:BA",
        "M:4/4",
        "L:1/8",
        "Q:1/4=120",
        "K:C",
    ] {
        assert!(fixed.contains(retained), "missing {retained:?} in {fixed}");
    }

    let linted = run_stdin(&fixed, &[]);
    assert!(linted.status.success(), "{linted:?}");
    assert!(linted.stderr.is_empty(), "{linted:?}");
}

#[test]
fn repeated_body_fields_are_preserved() {
    let source = "X:1\nT:Body changes\nK:C\nM:2/4\nCDEF |\nM:4/4\nGABc |\n";
    let fixed = fix(source);
    assert_eq!(fixed.matches("M:").count(), 2, "{fixed}");
}

#[test]
fn superseded_file_header_defaults_are_removed() {
    let source = concat!(
        "M:2/4\n",
        "M:4/4\n",
        "\n",
        "X:1\n",
        "T:File defaults\n",
        "K:C\n",
        "CDEF |\n",
    );
    let output = run_stdin(source, &[]);
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("earlier M: field is overridden")
    );

    let fixed = fix(source);
    assert!(!fixed.contains("M:2/4"), "{fixed}");
    assert!(fixed.starts_with("M:4/4\n\nX:1"), "{fixed}");
}

#[test]
fn fix_supplies_increasing_unique_references() {
    let source = concat!(
        "T:Missing one\nK:C\nCDEF |\n\n",
        "X:2\nT:Existing two\nK:C\nCDEF |\n\n",
        "X:\nT:Empty three\nK:C\nCDEF |\n\n",
        "X:4\nT:Existing four\nK:C\nCDEF |\n",
    );
    let fixed = fix(source);
    assert_eq!(references(&fixed), vec![1, 2, 3, 4]);
    assert_eq!(fixed.matches("\nX:").count(), 3, "{fixed}");
    assert!(fixed.starts_with("X:1\nT:Missing one"), "{fixed}");
}

#[test]
fn fix_moves_a_misplaced_reference_ahead_of_the_header_and_body() {
    let source = "T:Late reference\nK:C\nCDEF |\nX:7\n";
    let fixed = fix(source);
    assert!(fixed.starts_with("X:7\nT:Late reference\nK:C\n"), "{fixed}");
    assert_eq!(fixed.matches("X:7").count(), 1, "{fixed}");
}

#[test]
fn a_line_containing_only_removed_inline_fields_does_not_split_the_tune() {
    let source = "X:1\nT:Inline removal\nK:C\n[E:2][H:discard]\nCDEF |\n";
    let fixed = fix(source);
    assert!(fixed.contains("K:C\nCDEF |\n"), "{fixed}");
}

#[test]
fn a_body_meter_change_does_not_change_the_inferred_unit_length() {
    let source = "X:1\nT:Body meter\nK:C\nM:2/4\nQ:120\nCDEF |\n";
    let fixed = fix(source);
    assert_eq!(
        header_field_keys(&fixed),
        vec!['X', 'T', 'M', 'L', 'Q', 'K']
    );
    assert!(fixed.contains("M:2/4\nL:1/8\n"), "{fixed}");
    assert!(fixed.contains("Q:1/8=120\n"), "{fixed}");
}

#[test]
fn file_header_meter_is_inherited_by_each_tune() {
    let source = concat!(
        "M:2/4\n",
        "\n",
        "X:1\n",
        "T:First\n",
        "Q:120\n",
        "K:C\n",
        "CDEF |\n",
        "\n",
        "X:2\n",
        "T:Second\n",
        "Q:C=90\n",
        "K:C\n",
        "CDEF |\n",
    );
    let fixed = fix(source);
    assert_eq!(fixed.matches("Q:1/16=").count(), 2, "{fixed}");
}

#[test]
fn out_writes_fixed_output_to_a_file() {
    let path = std::env::temp_dir().join(format!("abc-lint-output-{}.abc", std::process::id()));
    let output = run_stdin(
        "X:1\nT:Output\nA:Donegal\nK:C\nCDEF |\n",
        &["--fix", "--out", path.to_str().unwrap()],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());

    let fixed = fs::read_to_string(&path).unwrap();
    fs::remove_file(path).unwrap();
    assert!(fixed.contains("O:Donegal\n"), "{fixed}");
}

#[test]
fn out_requires_fix() {
    let output = run_stdin("X:1\nT:Output\nK:C\nCDEF |\n", &["--out", "unused.abc"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("--fix"), "{error}");
}

#[test]
fn an_unrepresentable_deprecated_tempo_fails_only_when_fixing() {
    let source = concat!(
        "X:1\n",
        "T:Huge tempo\n",
        "Q:999999999999999999999999999999999999999999\n",
        "K:C\n",
        "CDEF |\n",
    );
    let lint = run_stdin(source, &[]);
    assert!(lint.status.success(), "{lint:?}");

    let fixed = run_stdin(source, &["--fix"]);
    assert!(!fixed.status.success());
    let error = String::from_utf8(fixed.stderr).unwrap();
    assert!(
        error.contains("could not resolve deprecated tempo"),
        "{error}"
    );
}
