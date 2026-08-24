# ADR 0001: Documentation landing page for `cargo doc --open`

- **Status:** Rejected
- **Date:** 2026-08-24

## Context

The repository is a virtual Cargo workspace (`Cargo.toml:1-3`: `[workspace]`, no
`[package]`, `members = ["abc-language-server", "abc-parser"]`, `resolver = "3"`).
Running `cargo doc --open --no-deps --workspace` opens
`target/doc/abc_language_server/index.html` (observed: cargo prints
`Opening .../target/doc/abc_language_server/index.html`). There is no
workspace-level `target/doc/index.html` (observed: `ls target/doc/index.html`
exits with status 2). `target/doc/crates.js` lists members alphabetically:
`["abc_language_server","abc_lint","abc_parser","abc_transpose",...]`.

The desired outcome was: bare `cargo doc --open` opens a README-based
introduction with links to all workspace crates.

Relevant repository facts (all observed this session):

- `abc-parser/src/lib.rs:15-23` already has hand-written `//!` crate-level docs
  with working intra-doc links to `[`architecture`]` and the `abc-transpose`
  binary.
- `README.md:7-26` contains a protected `MAINTAINER-CONTEXT-BEGIN`/`-END` block
  that must be preserved byte-for-byte (per `AGENTS.md`).
- `README.md:32-33` claims diagrams render "as inline SVG" and "do not need
  network access or JavaScript" — now **stale/false** after the aquamarine
  migration landed this session (separate fix needed regardless of this ADR).
- `README.md:54` uses a relative repository link
  `[the language-server guide](abc-language-server/README.md)`.
- Dependency direction is one-way: `abc-language-server/Cargo.toml:11` declares
  `abc-parser = { path = "../abc-parser" }`; `abc-parser` does **not** depend on
  `abc-language-server`.
- `.github/workflows/ci.yml:31,34,37` runs bare `cargo clippy`, `cargo build`,
  `cargo test` (no `--workspace`).
- `.pre-commit-config.yaml:13,27,34` runs `cargo +nightly fmt --all`,
  `cargo nextest run --all-features`, `cargo test --doc --all-features`.
- Per the Cargo Book, a virtual workspace with no `default-members` targets all
  members for bare commands; setting `default-members` narrows bare commands to
  that subset.

Mechanism (observed from cargo source `cargo/ops/cargo_doc.rs:66` on docs.rs):
`--open` opens `compilation.root_crate_names.get(0)` — the first root crate name
in cargo's compilation ordering.

## Options considered

1. **Reorder `members`** in root `Cargo.toml` so `abc-parser` is listed first.
2. **`default-members = ["abc-parser"]`**, plus add `--workspace` to every bare
   CI/pre-commit command so `abc-language-server` stays covered.
3. **Add a dedicated no-code "doc-hub" crate** named to sort before
   `abc_language_server` (e.g. `abc`), depending on both crates, with
   `#![doc = include_str!("../README.md")]` and intra-doc links to each crate.
4. **`#![doc = include_str!("../README.md")]` on `abc-parser`** (the idiomatic
   pattern), making abc-parser the landing page by content rather than by
   selection.

## Decision

Reject all options. Keep the current behavior: `cargo doc --open` opens
`abc_language_server/index.html`. No workspace, CI, or crate-structure changes.

## Rationale

A README cannot serve simultaneously as a good GitHub README and a rustdoc
landing page with working cross-crate links. This is the core blocker, and it is
grounded in concrete findings:

- **Intra-doc links** (`` [`abc_language_server`] ``) resolve in rustdoc only
  when the landing crate depends on the target. `abc-parser` does **not** depend
  on `abc-language-server` (observed, `abc-language-server/Cargo.toml:11`), so
  abc-parser cannot intra-doc-link to the LSP crate without introducing a
  reverse/cyclic dependency. Plain `` `[guide](abc-language-server/README.md)` `
  links work on GitHub but are dead under `target/doc/` (the path does not
  exist there). Observed: `README.md:54` already uses such a relative link.

Per option:

- **Option 1 (reorder `members`)** is insufficient on its own: even if it
  changed the opened crate to `abc-parser`, it would not make the README the
  intro — that still requires Option 4, which is rejected below. *Assumption,
  not verified:* whether reordering would even change the opened crate is
  uncertain. The current member list already lists `abc-language-server` first
  (`Cargo.toml:2`) and that is what `--open` opens, a state consistent with both
  "member-order" and "alphabetical" selection; the two cannot be distinguished
  from the current state without an experiment that was not performed. The
  rejection does not depend on this assumption.

- **Option 2 (`default-members`)** would achieve the goal but at a permanent
  ergonomic cost. `default-members = ["abc-parser"]` narrows **every bare
  workspace command** — `build`, `test`, `clippy`, `nextest` — to `abc-parser`
  only. The existing CI (`.github/workflows/ci.yml:31,34,37`) and pre-commit
  hooks (`.pre-commit-config.yaml:27,34`) run **bare** commands, so
  `abc-language-server` would be silently dropped from CI lint/build/test and
  from local test runs unless every such command gains `--workspace`. A bare
  `cargo nextest run` would report success while skipping the LSP crate's tests
  with no error. `README.md:39` (`cargo nextest run --all-features`) would also
  need updating. This is a standing maintenance tax with a silent-failure mode.

- **Option 3 (doc-hub crate)** would satisfy the literal request with no
  build/test/clippy side effects, but is non-idiomatic: Rust workspaces do not
  conventionally carry a no-code crate purely to host a doc landing page, and
  the README/intra-doc tension above remains for any link placed in the README
  itself (intra-doc links break on GitHub; relative links break in rustdoc).

- **Option 4 (`include_str!` README on abc-parser)** is the idiomatic pattern but
  is a net quality regression here. It would replace abc-parser's existing
  working `//!` crate-level docs and their working intra-doc links
  (`abc-parser/src/lib.rs:15-23`) with README content that embeds the protected
  maintainer block (`README.md:7-26`), carries the now-stale SVG/JS claims
  (`README.md:32-33`), and contains a dead-in-rustdoc relative link
  (`README.md:54`). It also still does not enable linking to the LSP crate
  (one-way dependency, same blocker as above).

## Consequences

- Bare `cargo doc --open` continues to open `abc_language_server/index.html`.
  Users wanting the parser docs should run `cargo doc --open --no-deps -p
  abc-parser` (verified this session: that command opens
  `target/doc/abc_parser/index.html`).
- `README.md:32-33` remains stale ("inline SVG", "no JavaScript") after the
  aquamarine migration. This is independent of this ADR and should be fixed on
  its own.
- No changes to `Cargo.toml`, CI, pre-commit, or crate structure.

## Reconsider if

- rustdoc/cargo gains native workspace-level landing-page support (a real
  `target/doc/index.html` aggregating members, so no crate-selection gymnastics
  is required).
- The project adopts a separate documentation tool (e.g. mdBook) for the user
  guide, making rustdoc purely an API reference and removing the "README as
  landing page" goal.
- The dependency direction reverses (`abc-parser` -> `abc-language-server`), or
  the LSP crate is removed, making cross-crate intra-doc links from abc-parser
  viable and eliminating the README/link tension.
- A future maintainer explicitly accepts the `default-members` CI tax (Option 2)
  and commits to keeping `--workspace` on every bare "run everything" command.
