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

//! Prints a source-spanned AST for an ABC file.

use abc_parser::parse;
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("usage: kitchen_sink FILE.abc");
        return ExitCode::FAILURE;
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("could not read input: {error}");
            return ExitCode::FAILURE;
        }
    };
    let report = parse(source.as_str());
    println!("{:#?}", report.output);
    for error in &report.errors {
        eprintln!("{error}");
    }
    if report.is_valid() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
