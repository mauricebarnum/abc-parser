# Implementation choices

- Target Rust edition 2024
- Use [chumksy](https://docs.rs/chumsky/latest/chumsky/) to implement the core parser
- Do not create any `unsafe` public APIs
- Use doc comments (`///`) for public API documentation
- Document the purpose of public structs and key methods
- Add examples for complex functionality
- Follow the Rust API [guidelines](https://rust-lang.github.io/api-guidelines/) where practical, document deviations with motivation
- Prefer to select [blessed](https://blessed.rs/crates) crates when choosing dependencies

# Coding Style

- **Clippy**: Strict workspace Cargo.toml settings must be followed.
- **Imports**: Prefer unmerged `use` statements (one item per line). Do not merge imports (e.g. avoid `use std::{foo, bar};`).
- **Docs**: Terse, idiomatic Rust API guidelines; prefer `const fn` when possible.
- **Formatting**: Run `rustfmt` from the nightly toolchain
