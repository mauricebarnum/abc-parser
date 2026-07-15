# Span-backed hybrid AST plan

## Summary

The parser will retain source-derived text as native input spans while keeping
semantic values directly usable. `parse` returns a `ParseReport` containing a
span-backed AST and native-span diagnostics. `IntoOwnedAst::into_owned`
materializes a standalone tree through either an exact source resolver or
`PlaceholderResolver`. No `parse_owned` convenience API is provided.

## Public model

- `SourceText<S>` distinguishes source spans from synthesized strings.
- `ParsedDocument<S>` uses `SourceText<S>`; `OwnedDocument<S>` uses `String`.
- `SourceResolver<S>` resolves spans as `Cow<str>`.
- `IntoOwnedAst<S>` converts parsed nodes using a resolver.
- Exact resolvers support `str` byte spans and `[char]` character spans.
- `PlaceholderResolver` produces `[[ABC_SOURCE_REF:<span Debug>]]` when source
  text is unavailable. This pattern is heuristic and may collide with legitimate
  source text.

## Parser changes

- Chumsky combinators produce semantic values or spans rather than raw strings.
- Numeric structures are accumulated from tokens without lexeme strings.
- Native input span types are preserved throughout the AST.
- Normalized or synthesized values remain owned.

## Documentation and validation

Every public item and every non-trivial private function will document its
grammar, invariants, recovery behavior, span units, and allocation behavior as
applicable. Tests will cover exact and placeholder conversion, missing source,
UTF-8 boundaries, string and character inputs, recovery, kitchen-sink parsing,
strict Clippy, nightly rustfmt, doctests, and warnings-as-errors rustdoc.
