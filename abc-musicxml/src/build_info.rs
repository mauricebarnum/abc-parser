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

//! Build and version metadata reporting.

use std::process::ExitCode;

pub use abc_parser::build_info::BuildInfo;
use abc_parser::build_info::check_version_and_exit_for;
use abc_parser::build_info::print_version_for;
use abc_parser::build_info_from_env;

/// Active build metadata for `abc-musicxml`.
pub const BUILD_INFO: BuildInfo = build_info_from_env!();

/// Intercepts `--version` and `--verbose` from command-line arguments.
///
/// Returns `Some(ExitCode::SUCCESS)` if version information was requested and
/// displayed, or `None` if argument parsing should proceed normally.
#[must_use]
pub fn check_version_and_exit(binary_name: &str) -> Option<ExitCode> {
    check_version_and_exit_for(&BUILD_INFO, binary_name)
}

/// Prints concise or verbose version information to standard output.
pub fn print_version(binary_name: &str, verbose: bool) {
    print_version_for(&BUILD_INFO, binary_name, verbose);
}
