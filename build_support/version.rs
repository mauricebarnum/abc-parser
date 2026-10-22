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

//! Extracts version and build metadata for workspace binaries at compile time.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

/// Discovers repository version information and emits `cargo:rustc-env` instructions.
pub fn generate_version_info(workspace_root: &Path) {
    let version_file_path = workspace_root.join("VERSION");
    println!("cargo:rerun-if-changed={}", version_file_path.display());
    println!("cargo:rerun-if-env-changed=ABC_VERSION_OVERRIDE");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    track_git_state(workspace_root);

    let base_version = fs::read_to_string(&version_file_path)
        .map_or_else(|_| "0.1.0".to_owned(), |content| content.trim().to_owned());

    let (full_version, commit_hash, commit_date) =
        if let Ok(override_ver) = env::var("ABC_VERSION_OVERRIDE") {
            (override_ver, "unknown".to_owned(), "unknown".to_owned())
        } else {
            compute_git_version(workspace_root, &base_version)
        };

    let build_date = compute_build_date();
    let rustc_version = compute_rustc_version();
    let host = env::var("HOST").unwrap_or_else(|_| "unknown".to_owned());
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());

    println!("cargo:rustc-env=ABC_VERSION_STRING={full_version}");
    println!("cargo:rustc-env=ABC_VERSION_BASE={base_version}");
    println!("cargo:rustc-env=ABC_GIT_COMMIT_HASH={commit_hash}");
    println!("cargo:rustc-env=ABC_GIT_COMMIT_DATE={commit_date}");
    println!("cargo:rustc-env=ABC_BUILD_DATE={build_date}");
    println!("cargo:rustc-env=ABC_BUILD_RUSTC={rustc_version}");
    println!("cargo:rustc-env=ABC_BUILD_HOST={host}");
    println!("cargo:rustc-env=ABC_BUILD_TARGET={target}");
    println!("cargo:rustc-env=ABC_BUILD_PROFILE={profile}");
}

/// Emits `rerun-if-changed` for Git refs and index so Cargo rebuilds on commits.
fn track_git_state(workspace_root: &Path) {
    let Ok(output) = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(workspace_root)
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let git_dir_str = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let git_dir = workspace_root.join(git_dir_str);

    let head_path = git_dir.join("HEAD");
    if head_path.exists() {
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head_content) = fs::read_to_string(&head_path)
            && let Some(ref_path_str) = head_content.strip_prefix("ref: ")
        {
            let ref_path = git_dir.join(ref_path_str.trim());
            if ref_path.exists() {
                println!("cargo:rerun-if-changed={}", ref_path.display());
            }
        }
    }

    let packed_refs_path = git_dir.join("packed-refs");
    if packed_refs_path.exists() {
        println!("cargo:rerun-if-changed={}", packed_refs_path.display());
    }

    let index_path = git_dir.join("index");
    if index_path.exists() {
        println!("cargo:rerun-if-changed={}", index_path.display());
    }
}

/// Evaluates commits touching `VERSION` vs `HEAD` and checks dirty status.
fn compute_git_version(workspace_root: &Path, base_version: &str) -> (String, String, String) {
    let Ok(head_out) = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
    else {
        return (
            base_version.to_owned(),
            "unknown".to_owned(),
            "unknown".to_owned(),
        );
    };

    if !head_out.status.success() {
        return (
            base_version.to_owned(),
            "unknown".to_owned(),
            "unknown".to_owned(),
        );
    }

    let head_hash = String::from_utf8_lossy(&head_out.stdout).trim().to_owned();
    let short_hash = if head_hash.len() >= 7 {
        &head_hash[..7]
    } else {
        &head_hash
    };

    let version_commit_output = Command::new("git")
        .args(["log", "-1", "--format=%H", "--", "VERSION"])
        .current_dir(workspace_root)
        .output();

    let version_commit = version_commit_output
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(String::new, |output| {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        });

    let commit_date = Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unknown".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        );

    let is_dirty = Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .current_dir(workspace_root)
        .status()
        .is_ok_and(|status| !status.success());

    let has_commits_since_file = version_commit.is_empty() || version_commit != head_hash;

    let mut version = base_version.to_owned();
    if has_commits_since_file {
        version.push('-');
        version.push_str(short_hash);
    }
    if is_dirty {
        version.push_str("-dirty");
    }

    (version, head_hash, commit_date)
}

/// Generates a build timestamp, honoring `SOURCE_DATE_EPOCH` for reproducibility.
fn compute_build_date() -> String {
    if let Ok(epoch_str) = env::var("SOURCE_DATE_EPOCH")
        && let Ok(epoch) = epoch_str.trim().parse::<u64>()
    {
        return format_utc_timestamp(epoch);
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format_utc_timestamp(now)
}

/// Formats a UNIX timestamp as a UTC RFC 3339 string without external dependencies.
fn format_utc_timestamp(seconds: u64) -> String {
    let days = seconds / 86400;
    let rem_seconds = seconds % 86400;
    let hours = rem_seconds / 3600;
    let rem_seconds = rem_seconds % 3600;
    let minutes = rem_seconds / 60;
    let secs = rem_seconds % 60;

    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{secs:02}Z")
}

/// Converts elapsed days since 1970-01-01 into a (year, month, day) tuple.
fn days_to_date(mut days: u64) -> (u32, u32, u32) {
    let mut year = 1970_u32;
    loop {
        let leap = is_leap_year(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap_year(year);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1_u32;
    for &m_days in &month_days {
        if days < m_days {
            break;
        }
        days -= m_days;
        month += 1;
    }
    (year, month, (days + 1) as u32)
}

/// Returns whether the given year is a leap year in the Gregorian calendar.
const fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Extracts `rustc --version`.
fn compute_rustc_version() -> String {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or_else(
            || "unknown".to_owned(),
            |output| String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        )
}
