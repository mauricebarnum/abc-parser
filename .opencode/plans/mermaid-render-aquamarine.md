# Plan: Replace `merman-rustdoc` with `aquamarine` for Mermaid rendering in rustdoc

## Goal

Switch the `abc-parser` crate from `merman-rustdoc` (build-time SVG rendering of
Mermaid diagrams) to `aquamarine` (the de-facto standard proc-macro that embeds
mermaid.js for client-side rendering in rustdoc). Keep the existing
Markdown-driven `build.rs` workflow intact so `docs/architecture.md` remains the
single source for the documented `architecture` module.

## Context (current state)

- `abc-parser/Cargo.toml:13` declares `merman-rustdoc = "0.7.0"` as a normal
  dependency, and line 16 ignores it under
  `[package.metadata.cargo-machete]` (it is only referenced inside a
  `cfg_attr(doc, …)`).
- `abc-parser/build.rs:30` emits
  `#[cfg_attr(doc, merman_rustdoc::merman(pipeline = "parity"))]` as the first
  line of the generated `$OUT_DIR/architecture.rs`, followed by one
  `#[doc = "…"]` attribute per line of `docs/architecture.md`, then
  `pub mod architecture {}`.
- `abc-parser/src/lib.rs:40` includes the generated module via
  `include!(concat!(env!("OUT_DIR"), "/architecture.rs"));`.
- `docs/architecture.md` contains 7 Mermaid fenced blocks: 3 `flowchart`
  (TD/LR/TB) and 4 `classDiagram`. Four of them begin with a YAML frontmatter
  `---config: … ---` block (setting `htmlLabels`, `themeVariables.fontSize`,
  and `flowchart.{nodeSpacing,rankSpacing,wrappingWidth}`).
- The `abc-language-server` workspace member does **not** use merman/aquamarine.
- CI (`.github/workflows/ci.yml`) runs clippy/build/test only; it does **not**
  build docs.
- `parser-codex.bak/` is an untracked, non-workspace backup that also references
  `merman-rustdoc`; it is out of scope.

## Behavioral tradeoffs (recorded for context)

| Aspect | `merman-rustdoc` (current) | `aquamarine` (target) |
| --- | --- | --- |
| Rendering | Build-time SVG via `roughr`/`merman-render` | Client-side via embedded `mermaid.js` (browser draws SVG) |
| Viewing requires JS | No (static SVG baked into HTML) | Yes (diagrams appear only when JS runs) |
| Dependency footprint | Large (`merman`, `merman-core`, `merman-render`, `roughr-merman`, `lalrpop`, `logos`, `lol_html`, `euclid`, `serde_yaml`, `dugong`, `manatee`, …) | Small (`include_dir`, `itertools`, `proc-macro-error2`, `proc-macro2`, `quote`, `syn`) |
| Ecosystem fit | Niche | De-facto standard for Mermaid-in-rustdoc |
| Mermaid features | Server-side renderer subset | Full `mermaid.js` (flowchart, classDiagram, frontmatter config, `%%init%%`) |

## Scope

In scope: `abc-parser` crate only.
Out of scope: `parser-codex.bak/` (untracked backup), `abc-language-server`,
CI workflow changes.

## Implementation tasks

1. **`abc-parser/Cargo.toml`** — swap the dependency and the machete ignore:
   - Remove `merman-rustdoc = "0.7.0"`.
   - Add `aquamarine = "0.6.0"` (normal `[dependencies]` entry; used via
     `cfg_attr(doc, …)` on the generated module, same shape as merman-rustdoc
     was).
   - In `[package.metadata.cargo-machete]`, change
     `ignored = ["merman-rustdoc"]` to `ignored = ["aquamarine"]` (static
     analysis still can't see the `cfg_attr(doc)` reference).

2. **`abc-parser/build.rs:30`** — change the generated attribute:
   - From: `String::from("#[cfg_attr(doc, merman_rustdoc::merman(pipeline = \"parity\"))]\n")`
   - To: `String::from("#[cfg_attr(doc, aquamarine::aquamarine)]\n")`
   - Keep everything else (rerun-if-changed, per-line `#[doc = {line:?}]`,
     `pub mod architecture {}`) unchanged.

3. **`Cargo.lock`** — regenerate by running `cargo update -p merman-rustdoc`
   (or a plain `cargo build`), then `cargo build` to pull aquamarine 0.6.0.
   Verify the `merman`, `merman-core`, `merman-render`, `merman-rustdoc`, and
   `roughr-merman` entries are gone and `aquamarine` is present.

4. **`abc-parser/docs/architecture.md:59-61`** — reword the prose to reflect
   client-side rendering. Current text:

   > The diagrams are stored as Mermaid source in this file and rendered as
   > inline SVG in the generated documentation. The surrounding text contains
   > the same information in documentation formats that do not display SVG.

   Replace with:

   > The diagrams are stored as Mermaid source in this file and rendered as
   > inline diagrams via mermaid.js in the generated documentation. The
   > surrounding text contains the same information for readers that do not
   > render Mermaid.

## Validation tasks

1. **Build** — `cargo build --all-targets` succeeds (workspace compiles with
   the new dependency).
2. **Clippy (strict)** — `CARGO_BUILD_WARNINGS=deny cargo clippy --all-targets --all-features`
   is clean (matches CI gate).
3. **Tests** — `cargo nextest run` (and `cargo test` for any doctests) still
   pass; no behavior change to non-doc code.
4. **cargo-machete** — `cargo machete` no longer reports `aquamarine` as unused
   (the ignore entry covers it) and does not report `merman-rustdoc`.
5. **Doc build (core check)** — `cargo doc --no-deps -p abc-parser` succeeds
   without warnings. `missing_docs = "deny"` still holds for the generated
   `architecture` module.
6. **Mermaid rendering** — open `target/doc/abc_parser/architecture/index.html`
   in a browser with JS enabled and confirm all 7 diagrams render:
   - 3 flowcharts (TD/LR/TB) with node shapes/labels.
   - 4 class diagrams.
   - The 4 diagrams with YAML frontmatter `---config: … ---` honor the config
     (font size, `htmlLabels: false`, spacing). If any frontmatter-driven
     config is dropped by aquamarine's parser, convert those blocks to the
     `%%init%%` directive syntax documented by aquamarine and re-check.
7. **Lockfile hygiene** — `cargo tree -p abc-parser -e normal` shows
   `aquamarine` and no `merman*`/`roughr-merman` crates.
8. **Format** — run `rustfmt` (nightly) on `build.rs`; no formatting
   regressions.

## Out of scope / notes

- `parser-codex.bak/` is an untracked backup; it still references
  `merman-rustdoc` but is not a workspace member and does not affect
  `cargo`/`cargo doc`. Left untouched unless asked otherwise.
- No CI changes are required (CI does not build docs). Adding doc-build
  coverage to CI is a separate task.
- No `unsafe` public APIs are introduced (aquamarine is a proc-macro;
  workspace `unsafe_code = "deny"` still holds).
