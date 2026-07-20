# Span-backed hybrid AST status and follow-up plan

## Current implementation

The parser retains source-derived text as native input spans while keeping
semantic values directly usable. `parse` returns a `ParseReport` containing a
span-backed AST and native-span diagnostics. `IntoOwnedAst::into_owned`
materializes a standalone tree through either an exact source resolver or
`PlaceholderResolver`. Callers choose the resolver explicitly; the public API
does not include a `parse_owned` convenience function.

## Public model

- `SourceText<S>` distinguishes source spans from synthesized strings.
- `ParsedDocument<S>` uses `SourceText<S>`; `OwnedDocument<S>` uses `String`.
- `SourceResolver<S>` resolves spans as `Cow<str>`.
- `IntoOwnedAst<S>` converts parsed nodes using a resolver.
- Exact resolvers support `str` byte spans and `[char]` character spans.
- `PlaceholderResolver` produces `[[ABC_SOURCE_REF:<span Debug>]]` when source
  text is unavailable. This pattern is heuristic and may collide with legitimate
  source text.

## Parser behavior

- Chumsky combinators produce semantic values or spans rather than raw strings.
- Numeric structures are accumulated from tokens without lexeme strings.
- Native input span types are preserved throughout the AST.
- Normalized or synthesized values remain owned.

## Validation coverage

Public items have API documentation, and internal parser functions document
their grammar, invariants, recovery behavior, span units, or allocation behavior
where those details affect maintenance. Tests cover exact and placeholder
conversion, unavailable source text, invalid and UTF-8-sensitive spans, string
and character inputs, physical-line recovery, and kitchen-sink parsing.

The pre-commit configuration runs nightly rustfmt, strict Clippy with
`CARGO_BUILD_WARNINGS=deny`, Nextest, and doctests. CI runs strict Clippy, builds
all targets, and runs the Cargo test suite. The warnings-as-errors rustdoc
command succeeds when run manually but is not part of either automated check.

## Remaining validation gaps

The parser and span-backed ownership model described above are implemented. The
remaining work concerns automated validation rather than missing AST or parser
behavior:

1. Add `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` to both
   pre-commit and CI so broken links and other rustdoc warnings fail automated
   checks.
2. Align CI with the repository's documented local checks by adding nightly
   rustfmt, installing and running Nextest for unit and integration tests, and
   retaining an explicit `cargo test --doc --all-features` step for doctests.
3. Keep the span-unit, recovery-boundary, and ownership-conversion tests as
   contract tests when parser inputs or AST text storage gain new variants.
