# Plan: Add `--version` and Build Information to Workspace Binaries

## 1. Overview and Goals

This document specifies the design, technical architecture, and implementation plan for adding a comprehensive `--version` flag to all executable binaries in the `abc-parser` workspace:

- `abc-transpose` (`abc-parser/src/bin/abc-transpose.rs`)
- `abc-lint` (`abc-parser/src/bin/abc-lint.rs`)
- `abc-language-server` (`abc-language-server/src/main.rs`)

### Key Objectives

1. **Simple, Human-Editable Version File**: Maintain the base semantic version in a single plain-text file (`VERSION`) at the workspace root (e.g., `0.1.0`). Bumping a version requires editing only this text file.
2. **Automated Commit-Derived Suffix**: If commits have been made since the `VERSION` file was committed, automatically append `-XXXX`, where `XXXX` is the short Git revision ID (`HEAD`).
3. **Dirty Working Tree Indicator**: If the working branch contains uncommitted modifications to tracked files, append `-dirty`.
4. **Detailed Build Information (`--version --verbose`)**: Provide extended build metadata when `--verbose` is combined with `--version` (or `-Vv` / `-vV`), patterned after the standard `rustc -vV` / `cargo -vV` format. This includes:
   - Build timestamp (UTC ISO 8601, respecting `SOURCE_DATE_EPOCH` for reproducible builds)
   - Compiler version (`rustc --version`)
   - Host platform triple (`$HOST`)
   - Target architecture and OS triple (`$TARGET`)
   - Git commit hash, commit timestamp, and current branch
   - Cargo build profile (`debug` or `release`)
5. **Zero External Build Dependencies**: Implement the build script logic using pure Rust standard library capabilities (`std::process::Command`, `std::env`, `std::fs`, `std::path`), avoiding third-party dependency bloat in `[build-dependencies]`.
6. **Graceful Fallbacks & Hermetic Build Support**: Ensure clean behavior in environments without Git (e.g., release tarballs, hermetic packaging builds, or Nix/Debian builders) and support environment overrides (`ABC_VERSION_OVERRIDE`).
7. **Strict Project Compliance**: Adhere strictly to the workspace guidelines defined in `AGENTS.md` (Rust edition 2024, unmerged imports, `const fn` where appropriate, strict Clippy settings, and complete doc comments).

---

## 2. Version String Specification & State Matrix

### 2.1 Format Definition

The version string produced by `--version` follows this structure:

```text
<base_version>[-<short_hash>][-dirty]
```

- `<base_version>`: The trimmed contents of `VERSION` (e.g., `0.1.0`).
- `-<short_hash>`: Present **only if** there are changes/commits in `HEAD` since the commit that last modified `VERSION`. `<short_hash>` is the 7-character hexadecimal prefix of the current `HEAD` commit (e.g., `771e828`).
- `-dirty`: Present **only if** the working tree contains uncommitted changes to tracked files (staged or unstaged).

### 2.2 Git Detection Logic

At build time (`build.rs`), the build script executes the following checks:

1. **Locate the Version File's Last Commit**:
   ```bash
   git log -1 --format=%H -- VERSION
   ```
   Let this commit hash be `COMMIT_VERSION`.
2. **Locate Current HEAD Commit**:
   ```bash
   git rev-parse HEAD
   ```
   Let this commit hash be `COMMIT_HEAD`.
3. **Determine if Changes Have Occurred Since `VERSION` was Committed**:
   - If `COMMIT_VERSION` is empty (e.g., `VERSION` is not yet committed in Git): treat as changed; use current HEAD's short hash.
   - If `COMMIT_VERSION == COMMIT_HEAD`: exactly 0 commits have occurred since `VERSION` was committed. Do **not** append `-<short_hash>`.
   - If `COMMIT_VERSION != COMMIT_HEAD`: one or more commits exist since `VERSION` was committed. Append `-<short_hash>` (`git rev-parse --short HEAD`).
4. **Determine Working Tree Dirty Status**:
   ```bash
   git diff-index --quiet HEAD --
   ```
   - Exit code `0`: Working tree is clean.
   - Non-zero exit code: Tracked files have unstaged or staged modifications. Append `-dirty`.
   *(Note: Untracked files like scratch buffers, local tests, or `.envrc` are ignored so developer sandbox workflows do not perpetually brand builds as dirty, matching `git describe --dirty` semantics).*
5. **Non-Git Fallback**:
   If Git is not installed or the directory is not inside a Git repository (e.g., an extracted release archive):
   - Check for the `ABC_VERSION_OVERRIDE` environment variable.
   - If not set, use `<base_version>` directly without suffix.

### 2.3 Version State Matrix

| State | Commits Since `VERSION` Committed | Working Tree Tracked Changes | Output of `--version` | Example |
| :--- | :--- | :--- | :--- | :--- |
| **Tagged / Release Commit** | 0 (`COMMIT_VERSION == COMMIT_HEAD`) | Clean | `<base>` | `0.1.0` |
| **Release Commit + Uncommitted Edits** | 0 (`COMMIT_VERSION == COMMIT_HEAD`) | Dirty | `<base>-dirty` *(or `<base>-<hash>-dirty`)* | `0.1.0-dirty` |
| **Subsequent Commit** | $\ge 1$ (`COMMIT_VERSION != COMMIT_HEAD`) | Clean | `<base>-<short_hash>` | `0.1.0-771e828` |
| **Subsequent Commit + Uncommitted Edits** | $\ge 1$ (`COMMIT_VERSION != COMMIT_HEAD`) | Dirty | `<base>-<short_hash>-dirty` | `0.1.0-771e828-dirty` |
| **Non-Git Source Archive** | N/A (no `.git` directory) | N/A | `<base>` | `0.1.0` |
| **Explicit Override** | Any (`ABC_VERSION_OVERRIDE="0.1.0-custom"`) | Any | `<override>` | `0.1.0-custom` |

---

## 3. Build Information Specification (`--version --verbose`)

When invoked with `--version --verbose` (or `-Vv` / `-vV` / `--verbose --version`), each binary will print an extended metadata block modeled after standard Rust tooling:

```text
abc-transpose 0.1.0-771e828-dirty
release: 0.1.0
commit-hash: 771e828185187df4cfd2092014166dec9814410a
commit-date: 2026-10-21T18:45:00+00:00
build-date: 2026-10-21T20:26:20Z
compiler: rustc 1.95.0 (xxxx 2026-xx-xx)
host: x86_64-unknown-linux-gnu
target: x86_64-unknown-linux-gnu
profile: release
```

### Metadata Fields

1. **Binary Name & Full Version Line**: First line identical to standard `--version`.
2. **`release`**: Base version extracted from the root `VERSION` file.
3. **`commit-hash`**: Full 40-character hexadecimal SHA-1 commit hash of `HEAD` (or `unknown` if non-Git).
4. **`commit-date`**: ISO 8601 commit author date from `git log -1 --format=%cI` (or `unknown`).
5. **`build-date`**: Timestamp in UTC RFC 3339 format when the crate was compiled.
   - *Reproducible Builds*: If `SOURCE_DATE_EPOCH` is set in the environment, use that UNIX epoch timestamp instead of the current system clock.
6. **`compiler`**: Output of `rustc --version` (or compiler binary path queried via Cargo's `$RUSTC`).
7. **`host`**: Host platform triple provided by Cargo (`$HOST`, e.g., `x86_64-unknown-linux-gnu`).
8. **`target`**: Target platform triple provided by Cargo (`$TARGET`, e.g., `aarch64-apple-darwin`).
9. **`profile`**: Cargo build profile (`$PROFILE`, e.g., `debug` or `release`).

---

## 4. Architecture & Cargo Integration

### 4.1 Component Diagram

```
workspace root/
├── VERSION                        <-- Single source of truth for base version
├── build_support/
│   └── version.rs                 <-- Shared build-time logic (zero external crates)
├── abc-parser/
│   ├── build.rs                   <-- Runs build_support/version.rs + architecture.rs doc gen
│   └── src/
│       ├── bin/
│       │   ├── abc-transpose.rs   <-- Includes version & verbose handling
│       │   └── abc-lint.rs        <-- Includes version & verbose handling
│       └── build_info.rs          <-- Internal helper formatting compile-time env vars
└── abc-language-server/
    ├── build.rs                   <-- Runs build_support/version.rs
    └── src/
        └── main.rs                <-- Includes version & verbose handling
```

### 4.2 Cargo Rerun Invalidation (`cargo:rerun-if-changed`)

To guarantee that Cargo rebuilds the binaries when and only when relevant inputs change, the build script emits specific rerun triggers:

1. `cargo:rerun-if-changed=../VERSION` (re-runs whenever `VERSION` is modified).
2. `cargo:rerun-if-env-changed=ABC_VERSION_OVERRIDE` (allows manual version injection).
3. `cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH` (supports reproducible build systems).
4. **Git State Tracking**:
   - `.git/HEAD`: Detects branch switches, checkouts, and commits in detached HEAD state.
   - `.git/refs/heads/<branch>` (or `.git/packed-refs`): Detects new commits on the active branch.
   - `.git/index`: Detects staged changes (dirty tree state updates).
   - *Worktrees/Submodules*: The script runs `git rev-parse --git-dir` to locate the true Git metadata directory, supporting linked worktrees.

When none of these change, Cargo skips re-running `build.rs`, preserving sub-second incremental builds.

### 4.3 Communication via `cargo:rustc-env`

Rather than generating `.rs` source files on disk for the version data, `build.rs` communicates directly with the compiler using Cargo's native `cargo:rustc-env=KEY=VALUE` directives:

- `cargo:rustc-env=ABC_VERSION_STRING=<computed_version>`
- `cargo:rustc-env=ABC_VERSION_BASE=<base_version>`
- `cargo:rustc-env=ABC_GIT_COMMIT_HASH=<commit_hash>`
- `cargo:rustc-env=ABC_GIT_COMMIT_DATE=<commit_date>`
- `cargo:rustc-env=ABC_BUILD_DATE=<build_date>`
- `cargo:rustc-env=ABC_BUILD_RUSTC=<rustc_version>`
- `cargo:rustc-env=ABC_BUILD_HOST=<host_triple>`
- `cargo:rustc-env=ABC_BUILD_TARGET=<target_triple>`
- `cargo:rustc-env=ABC_BUILD_PROFILE=<profile>`

The binaries reference these values at compile time via the standard `env!("ABC_...")` macro.

---

## 5. Command-Line Interface (Clap) Integration

### 5.1 The Required Argument Conflict Problem

Both `abc-transpose` and `abc-lint` currently specify required positional arguments and mutually exclusive argument groups:
- In `abc-transpose`: `<INPUT>` is required, and exactly one of `--key`, `--semitones`, or `--steps` is required via `ArgGroup::new("transposition").required(true)`.
- In `abc-lint`: `<INPUT>` is required.

If `--version` is implemented solely as an optional boolean flag parsed by `Arguments::parse()`, Clap will reject invocations such as `abc-transpose --version` with an error:
```text
error: the following required arguments were not provided:
  <INPUT>
  <--key <KEY>|--semitones <N>|--steps <N>>
```

Furthermore, Clap's default automatic `ArgAction::Version` exits the process immediately upon encountering `--version`, preventing the application from inspecting whether `--verbose` was also supplied.

### 5.2 The Solution: Two-Phase / Early Version Interception

To provide a seamless user experience that works across all binaries:

1. **Define Flags in `Arguments`**:
   Add `--version` (`-V`) and `--verbose` (`-v`) to each binary's `Arguments` struct with doc comments so they are listed in `--help`.
2. **Early Inspection Hook in `main()`**:
   Before calling `Arguments::parse()`, invoke a lightweight shared helper:
   ```rust
   if let Some(exit_code) = build_info::check_version_and_exit("abc-transpose") {
       return exit_code;
   }
   ```
3. **Behavior of `check_version_and_exit`**:
   - Inspects `std::env::args_os()`.
   - Checks if any argument matches `-V`, `--version`, or short clusters containing `V` (e.g. `-Vv` or `-vV`).
   - Checks if any argument matches `-v`, `--verbose`, or short clusters containing `v`.
   - If version was requested:
     - If verbose: prints the extended build information block.
     - If concise: prints `<bin_name> <version>`.
     - Returns `Some(ExitCode::SUCCESS)`.
   - If version was not requested: returns `None`, allowing normal `Arguments::parse()` execution (handling `--help`, required arguments, and normal operation).

---

## 6. Step-by-Step Implementation Details

### Step 1: Create `VERSION` File at Workspace Root

Create a plain text file `/VERSION` with the current version:
```text
0.1.0
```

### Step 2: Implement `build_support/version.rs`

Create a self-contained, dependency-free build helper `/build_support/version.rs`:

```rust
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
use std::path::PathBuf;
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
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "0.1.0".to_owned());

    let (full_version, commit_hash, commit_date) = if let Ok(override_ver) = env::var("ABC_VERSION_OVERRIDE") {
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
    let git_dir_output = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(workspace_root)
        .output();

    let Ok(output) = git_dir_output else { return; };
    if !output.status.success() { return; }

    let git_dir_str = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let git_dir = workspace_root.join(git_dir_str);

    let head_path = git_dir.join("HEAD");
    if head_path.exists() {
        println!("cargo:rerun-if-changed={}", head_path.display());
        if let Ok(head_content) = fs::read_to_string(&head_path) {
            if let Some(ref_path_str) = head_content.strip_prefix("ref: ") {
                let ref_path = git_dir.join(ref_path_str.trim());
                if ref_path.exists() {
                    println!("cargo:rerun-if-changed={}", ref_path.display());
                }
            }
        }
    }

    let index_path = git_dir.join("index");
    if index_path.exists() {
        println!("cargo:rerun-if-changed={}", index_path.display());
    }
}

/// Evaluates commits touching `VERSION` vs `HEAD` and checks dirty status.
fn compute_git_version(workspace_root: &Path, base_version: &str) -> (String, String, String) {
    let head_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output();

    let Ok(head_out) = head_output else {
        return (base_version.to_owned(), "unknown".to_owned(), "unknown".to_owned());
    };

    if !head_out.status.success() {
        return (base_version.to_owned(), "unknown".to_owned(), "unknown".to_owned());
    }

    let head_hash = String::from_utf8_lossy(&head_out.stdout).trim().to_owned();
    let short_hash = if head_hash.len() >= 7 { &head_hash[..7] } else { &head_hash };

    let version_commit_output = Command::new("git")
        .args(["log", "-1", "--format=%H", "--", "VERSION"])
        .current_dir(workspace_root)
        .output();

    let version_commit = version_commit_output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();

    let commit_date = Command::new("git")
        .args(["log", "-1", "--format=%cI"])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    let is_dirty = Command::new("git")
        .args(["diff-index", "--quiet", "HEAD", "--"])
        .current_dir(workspace_root)
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

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

/// Generates build timestamp, honoring `SOURCE_DATE_EPOCH` for reproducibility.
fn compute_build_date() -> String {
    if let Ok(epoch_str) = env::var("SOURCE_DATE_EPOCH") {
        if let Ok(epoch) = epoch_str.trim().parse::<u64>() {
            return format_utc_timestamp(epoch);
        }
    }
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_utc_timestamp(now)
}

/// Minimal UTC RFC 3339 timestamp formatter without external crates.
fn format_utc_timestamp(seconds: u64) -> String {
    let days = seconds / 86400;
    let rem_seconds = seconds % 86400;
    let hours = rem_seconds / 3600;
    let rem_seconds = rem_seconds % 3600;
    let minutes = rem_seconds / 60;
    let secs = rem_seconds % 60;

    // Convert days since UNIX epoch (1970-01-01) to YYYY-MM-DD
    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{secs:02}Z")
}

fn days_to_date(mut days: u64) -> (u32, u32, u32) {
    let mut year = 1970_u32;
    loop {
        let leap = is_leap_year(year);
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap_year(year);
    let month_days = [
        31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31
    ];
    let mut month = 1_u32;
    for &m_days in &month_days {
        if days < m_days { break; }
        days -= m_days;
        month += 1;
    }
    (year, month, (days + 1) as u32)
}

const fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Extracts `rustc --version`.
fn compute_rustc_version() -> String {
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
```

### Step 3: Update `abc-parser/build.rs` and Add `abc-language-server/build.rs`

In `abc-parser/build.rs`, invoke `version::generate_version_info`:

```rust
#[path = "../build_support/version.rs"]
mod version;

fn main() -> io::Result<()> {
    // ... existing architecture module generation ...
    version::generate_version_info(Path::new(".."));
    Ok(())
}
```

Add `abc-language-server/build.rs`:

```rust
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

//! Generates build and version metadata for abc-language-server.

use std::path::Path;

#[path = "../build_support/version.rs"]
mod version;

fn main() {
    version::generate_version_info(Path::new(".."));
}
```

### Step 4: Implement Runtime Formatting Module (`build_info.rs`)

Create `abc-parser/src/build_info.rs` (and reference it in `abc-language-server` or include a counterpart):

```rust
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
use std::process::ExitCode;

/// Version and compilation metadata collected at build time.
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

/// Active build metadata for this crate.
pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: env!("ABC_VERSION_STRING"),
    base_version: env!("ABC_VERSION_BASE"),
    commit_hash: env!("ABC_GIT_COMMIT_HASH"),
    commit_date: env!("ABC_GIT_COMMIT_DATE"),
    build_date: env!("ABC_BUILD_DATE"),
    rustc_version: env!("ABC_BUILD_RUSTC"),
    host: env!("ABC_BUILD_HOST"),
    target: env!("ABC_BUILD_TARGET"),
    profile: env!("ABC_BUILD_PROFILE"),
};

/// Intercepts `--version` and `--verbose` from command-line arguments.
///
/// Returns `Some(ExitCode::SUCCESS)` if version information was requested and
/// displayed, or `None` if argument parsing should proceed normally.
pub fn check_version_and_exit(binary_name: &str) -> Option<ExitCode> {
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
        } else if arg.starts_with('-') && !arg.starts_with("--") {
            let chars = arg[1..].chars();
            if chars.clone().any(|c| c == 'V') {
                version_requested = true;
            }
            if chars.any(|c| c == 'v') {
                verbose_requested = true;
            }
        }
    }

    if version_requested {
        print_version(binary_name, verbose_requested);
        Some(ExitCode::SUCCESS)
    } else {
        None
    }
}

/// Prints concise or verbose version information to standard output.
pub fn print_version(binary_name: &str, verbose: bool) {
    println!("{binary_name} {}", BUILD_INFO.version);
    if verbose {
        println!("release: {}", BUILD_INFO.base_version);
        println!("commit-hash: {}", BUILD_INFO.commit_hash);
        println!("commit-date: {}", BUILD_INFO.commit_date);
        println!("build-date: {}", BUILD_INFO.build_date);
        println!("compiler: {}", BUILD_INFO.rustc_version);
        println!("host: {}", BUILD_INFO.host);
        println!("target: {}", BUILD_INFO.target);
        println!("profile: {}", BUILD_INFO.profile);
    }
}
```

### Step 5: Update Binary Entry Points

In `abc-transpose.rs`, `abc-lint.rs`, and `abc-language-server/src/main.rs`:

1. Add `--version` and `--verbose` documentation flags to `struct Arguments`:
   ```rust
   /// Print version information (combine with --verbose for detailed build info).
   #[arg(short = 'V', long)]
   version: bool,

   /// Emit verbose output (shows detailed build info when combined with --version).
   #[arg(short = 'v', long)]
   verbose: bool,
   ```
2. In `main()`, call `check_version_and_exit(...)` as the first statement:
   ```rust
   fn main() -> ExitCode {
       if let Some(exit_code) = build_info::check_version_and_exit("abc-transpose") {
           return exit_code;
       }
       // ... existing argument parsing and execution ...
   }
   ```

---

## 7. Verification & Testing Strategy

### 7.1 Automated Integration Tests (`cargo nextest run`)

Create an integration test in `abc-parser/tests/version_flag.rs`:

1. **Concise Version Output**:
   - Run `abc-transpose --version`. Assert exit code 0.
   - Assert output matches regex `^abc-transpose \d+\.\d+\.\d+.*$`.
   - Assert standard error is empty.
2. **Short Flag Equivalents**:
   - Verify `abc-transpose -V` produces identical output.
3. **Verbose Version Output**:
   - Run `abc-transpose --version --verbose`. Assert exit code 0.
   - Assert output contains `release:`, `commit-hash:`, `compiler:`, `host:`, and `target:`.
   - Verify combined short flag `-Vv` and `-vV` produce identical output.
4. **Execution Independence from Required Arguments**:
   - Run `abc-transpose --version` without `<INPUT>` or transposition flags. Verify it succeeds with exit code 0 and does not demand inputs.
   - Run `abc-lint --version` without `<INPUT>`. Verify it succeeds with exit code 0.
5. **Language Server Flag**:
   - Run `abc-language-server --version` and `abc-language-server --version --verbose`. Assert exit code 0.

### 7.2 Manual Scenarios Verification

| Scenario | Command | Expected Result |
| :--- | :--- | :--- |
| **Commit on branch** | `git commit ... && cargo run -p abc-parser --bin abc-transpose -- --version` | Shows `0.1.0-<short_hash>` |
| **Dirty working tree** | Modify a tracked file && re-run | Shows `0.1.0-<short_hash>-dirty` |
| **Clean after commit** | Stash or discard changes && re-run | Suffix drops `-dirty` |
| **Update `VERSION`** | `echo "0.2.0" > VERSION && cargo run ...` | Immediately shows `0.2.0-<hash>-dirty` |
| **Non-Git override** | `ABC_VERSION_OVERRIDE="1.0.0-rc1" cargo run ...` | Shows `abc-transpose 1.0.0-rc1` |

---

## 8. Rollout Plan Checklist

- [ ] **Step 1**: Commit root `/VERSION` containing initial version `0.1.0`.
- [ ] **Step 2**: Create `/build_support/version.rs` with standard-library git and build info extraction.
- [ ] **Step 3**: Update `abc-parser/build.rs` to call `generate_version_info`.
- [ ] **Step 4**: Add `abc-language-server/build.rs` to call `generate_version_info`.
- [ ] **Step 5**: Create `abc-parser/src/build_info.rs` with `check_version_and_exit` and `BUILD_INFO`.
- [ ] **Step 6**: Add `-V`/`--version` and `-v`/`--verbose` handling to:
  - `abc-parser/src/bin/abc-transpose.rs`
  - `abc-parser/src/bin/abc-lint.rs`
  - `abc-language-server/src/main.rs`
- [ ] **Step 7**: Add integration tests in `abc-parser/tests/version_flag.rs`.
- [ ] **Step 8**: Validate with `cargo check`, `CARGO_BUILD_WARNINGS=deny cargo clippy --all-targets`, `cargo nextest run`, and `cargo fmt --check`.
