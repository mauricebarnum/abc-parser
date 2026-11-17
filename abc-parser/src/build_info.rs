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

use std::env;
use std::fs;
use std::io;
use std::io::Read as _;
use std::path::Path;
use std::process::ExitCode;

/// Version and compilation metadata collected at build time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildInfo {
    /// Full version string including short git hash and dirty status.
    pub version: &'static str,
    /// Base version from the VERSION text file.
    pub base_version: &'static str,
    /// Git commit SHA-1 at build time.
    pub commit_hash: &'static str,
    /// Git commit date at build time.
    pub commit_date: &'static str,
    /// UTC timestamp of the build.
    pub build_date: &'static str,
    /// Compiler version string.
    pub rustc_version: &'static str,
    /// Build host platform triple.
    pub host: &'static str,
    /// Compilation target platform triple.
    pub target: &'static str,
    /// Cargo build profile.
    pub profile: &'static str,
}

/// Captures compile-time build and version metadata from the invoking crate's
/// `build.rs` environment variables.
#[macro_export]
macro_rules! build_info_from_env {
    () => {
        $crate::build_info::BuildInfo {
            version: env!("ABC_VERSION_STRING"),
            base_version: env!("ABC_VERSION_BASE"),
            commit_hash: env!("ABC_GIT_COMMIT_HASH"),
            commit_date: env!("ABC_GIT_COMMIT_DATE"),
            build_date: env!("ABC_BUILD_DATE"),
            rustc_version: env!("ABC_BUILD_RUSTC"),
            host: env!("ABC_BUILD_HOST"),
            target: env!("ABC_BUILD_TARGET"),
            profile: env!("ABC_BUILD_PROFILE"),
        }
    };
}

/// Active build metadata for this crate.
pub const BUILD_INFO: BuildInfo = build_info_from_env!();

/// Intercepts `--version` and `--verbose` from command-line arguments using
/// the caller-supplied [`BuildInfo`].
///
/// Returns `Some(ExitCode::SUCCESS)` if version information was requested and
/// displayed, or `None` if argument parsing should proceed normally.
#[must_use]
pub fn check_version_and_exit_for(info: &BuildInfo, binary_name: &str) -> Option<ExitCode> {
    let mut version_requested = false;
    let mut verbose_requested = false;

    for arg in env::args().skip(1) {
        if arg == "--" {
            break;
        }
        if arg == "-V" || arg == "--version" {
            version_requested = true;
        } else if arg == "-v" || arg == "--verbose" {
            verbose_requested = true;
        } else if let Some(stripped) = arg.strip_prefix('-')
            && !stripped.starts_with('-')
        {
            if stripped.contains('V') {
                version_requested = true;
            }
            if stripped.contains('v') {
                verbose_requested = true;
            }
        }
    }

    if version_requested {
        print_version_for(info, binary_name, verbose_requested);
        Some(ExitCode::SUCCESS)
    } else {
        None
    }
}

/// Intercepts `--version` and `--verbose` from command-line arguments.
///
/// Returns `Some(ExitCode::SUCCESS)` if version information was requested and
/// displayed, or `None` if argument parsing should proceed normally.
#[must_use]
pub fn check_version_and_exit(binary_name: &str) -> Option<ExitCode> {
    check_version_and_exit_for(&BUILD_INFO, binary_name)
}

/// Prints concise or verbose version information to standard output for the
/// given [`BuildInfo`].
pub fn print_version_for(info: &BuildInfo, binary_name: &str, verbose: bool) {
    println!("{binary_name} {}", info.version);
    if verbose {
        println!("release: {}", info.base_version);
        println!("commit-hash: {}", info.commit_hash);
        println!("commit-date: {}", info.commit_date);
        println!("build-date: {}", info.build_date);
        println!("compiler: {}", info.rustc_version);
        println!("host: {}", info.host);
        println!("target: {}", info.target);
        println!("profile: {}", info.profile);
    }
}

/// Prints concise or verbose version information to standard output.
pub fn print_version(binary_name: &str, verbose: bool) {
    print_version_for(&BUILD_INFO, binary_name, verbose);
}

/// Returns a display name for diagnostics concerning one input path.
///
/// Maps `-` to `"<stdin>"` and formats any other path with [`Path::display`].
#[must_use]
pub fn input_name(path: &Path) -> String {
    if path.as_os_str() == "-" {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

/// Reads the named file, or standard input when `path` is `-`.
///
/// # Errors
///
/// Returns a diagnostic message if standard input or `path` cannot be read.
pub fn read_source(path: &Path) -> Result<String, String> {
    if path.as_os_str() == "-" {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("could not read standard input: {error}"))?;
        Ok(source)
    } else {
        fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
    }
}
