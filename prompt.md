# ABC notation parser instructions

## Objective

Create a Rust library that parses the ABC music notation v2.1 as specificed in the
[spec](./abc_standard_v2.1.pdf). The parser must support the following use cases:

1. validate that an input is well-formed
1. parse an input and produce an abstract syntax tree that optionally preserves
   source locations for productions
1. parse a possibly erroneous input and attempt to recover from errors to
   continue parsing
1. support parsing partial inputs such as a line of notes, a directive line, a
   chord, etc.

## Implementation choices

- Target Rust edition 2024
- Use [chumksy](https://docs.rs/chumsky/latest/chumsky/) to implement the core parser
- Do not create any `unsafe` public APIs
- Use doc comments (`///`) for public API documentation
- Document the purpose of public structs and key methods
- Add examples for complex functionality
- Follow the Rust API [guidelines](https://rust-lang.github.io/api-guidelines/) where practical, document deviations with motivation
- Prefer to select [blessed](https://blessed.rs/crates) crates when choosing dependencies

## Validation

- Successfuily parse (./test_kitchen_sink.abc) and print an AST annotated with
  source positions for each item
- Generate test cases by randomly mutating (./test_kitchen_sink.abc) and verifying faults are located, and the parser recovers to parse as much of the input as possible.
- Create tests for all public entry points, with both positive and negative test cases

## Coding Style

- **Clippy**: Strict workspace Cargo.toml settings must be followed.
- **Imports**: Prefer unmerged `use` statements (one item per line). Do not merge imports (e.g. avoid `use std::{foo, bar};`).
- **Docs**: Terse, idiomatic Rust API guidelines; prefer `const fn` when possible.
- **Formatting**: Run `rustfmt` from the nightly toolchain
