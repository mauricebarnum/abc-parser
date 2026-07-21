# Implementation choices

- Target Rust edition 2024
- Use [chumksy](https://docs.rs/chumsky/latest/chumsky/) to implement the core parser
- Treat the checked-in `abc_standard_v2.1.txt` as the canonical text of the
  ABC 2.1 specification for grammar, validation, and syntax diagnostics. The
  checked-in `abc_standard_v2.1.pdf` may include referenced assets that are not
  present in the canonical text export.
- The canonical text was retrieved as `abc-standard-v2.1.txt` with:

  ```sh
  curl -L -X POST \
    -H 'User-Agent: Mozilla/5.0' \
    -d 'do=export_raw' \
    -o abc-standard-v2.1.txt \
    'https://abcnotation.com/wiki/abc:standard:v2.1'
  ```

  The checked-in `abc_standard_v2.1.txt` is byte-identical to that export.
- Do not create any `unsafe` public APIs
- Use doc comments (`///`) for public API documentation
- Document the purpose of public structs and key methods
- Add examples for complex functionality
- Follow the Rust API [guidelines](https://rust-lang.github.io/api-guidelines/) where practical, document deviations with motivation
- Prefer to select [blessed](https://blessed.rs/crates) crates when choosing dependencies

# Protected documentation

- Do not modify, reformat, move, summarize, or delete the markers or any
  content from `MAINTAINER-CONTEXT-BEGIN` through
  `MAINTAINER-CONTEXT-END`.
- This content is authored exclusively by the repository owner and must be
  preserved byte-for-byte.

# Coding Style

- **Clippy**: Strict workspace Cargo.toml settings must be followed. Set
  `CARGO_BUILD_WARNINGS=deny` when running Clippy.
- **Imports**: Prefer unmerged `use` statements (one item per line). Do not merge imports (e.g. avoid `use std::{foo, bar};`).
- **Docs**: Terse, idiomatic Rust API guidelines; prefer `const fn` when possible.
- **Formatting**: Run `rustfmt` from the nightly toolchain

# Testing

- Prefer `cargo nextest run` over `cargo test` for running unit and integration
  tests.
- Use `cargo test` only when the task is particularly suited to it, such as
  running documentation tests that Nextest does not support.

# Commit Messages

- Wrap commit-message lines at 72 characters, except for exposition that
  syntactically needs to be longer.
- Include a trailer block in every commit that identifies the actual AI
  contributor, tool, and model used. Do not copy example values when they do
  not describe the commit:

  ```text
  Co-authored-by: <AI contributor name> <AI contributor email>
  AI-Tool: <AI tool used>
  AI-Model: <AI model used>
  ```
