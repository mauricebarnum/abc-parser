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

//! Integration tests for `--version` and `--verbose` flags on `abc-language-server`.

use std::process::Command;

#[test]
fn language_server_version_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_abc-language-server"))
        .arg("--version")
        .output()
        .expect("abc-language-server should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is valid utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr is valid utf-8");

    assert!(stderr.is_empty(), "stderr should be empty: {stderr}");
    assert!(
        stdout.starts_with("abc-language-server 0.1.0"),
        "stdout should start with binary name and version: {stdout}"
    );
    assert_eq!(stdout.lines().count(), 1);
}

#[test]
fn language_server_short_version_flag() {
    let long_output = Command::new(env!("CARGO_BIN_EXE_abc-language-server"))
        .arg("--version")
        .output()
        .expect("abc-language-server should execute");
    let short_output = Command::new(env!("CARGO_BIN_EXE_abc-language-server"))
        .arg("-V")
        .output()
        .expect("abc-language-server should execute");

    assert!(long_output.status.success());
    assert!(short_output.status.success());
    assert_eq!(long_output.stdout, short_output.stdout);
}

#[test]
fn language_server_version_verbose() {
    let output = Command::new(env!("CARGO_BIN_EXE_abc-language-server"))
        .args(["--version", "--verbose"])
        .output()
        .expect("abc-language-server should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout is valid utf-8");

    assert!(stdout.starts_with("abc-language-server 0.1.0"));
    assert!(stdout.contains("release: 0.1.0"));
    assert!(stdout.contains("commit-hash: "));
    assert!(stdout.contains("commit-date: "));
    assert!(stdout.contains("build-date: "));
    assert!(stdout.contains("compiler: "));
    assert!(stdout.contains("host: "));
    assert!(stdout.contains("target: "));
    assert!(stdout.contains("profile: "));
}
