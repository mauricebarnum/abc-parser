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

//! Generates the documented architecture module and version metadata.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

#[path = "../build_support/version.rs"]
mod version;

fn main() -> io::Result<()> {
    const ARCHITECTURE_PATH: &str = "docs/architecture.md";

    println!("cargo:rerun-if-changed={ARCHITECTURE_PATH}");

    let architecture = fs::read_to_string(ARCHITECTURE_PATH)?;
    let mut generated =
        String::from("#[cfg_attr(all(doc, feature = \"doc-mermaid\"), aquamarine::aquamarine)]\n");

    for line in architecture.lines() {
        writeln!(generated, "#[doc = {line:?}]").expect("writing to a String cannot fail");
    }

    generated.push_str("pub mod architecture {}\n");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));

    fs::write(output.join("architecture.rs"), generated)?;

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap_or_else(|| Path::new(".."));
    version::generate_version_info(workspace_root);

    Ok(())
}
