# abc-parser

A source-spanned, error-recovering parser for ABC music notation 2.1. The crate
offers whole-file validation and parsing as well as entry points for fields,
directives, chords, and individual music lines.

The primary APIs are Chumsky parser constructors generic over
`ValueInput<Token = char>`. AST spans retain the input's native span type, so
the same parser accepts strings, character slices, mapped inputs, and value
streams without first converting them to `&str`.

```sh
cargo run -p abc-parser --example kitchen_sink -- test_kitchen_sink.abc
cargo run -p abc-parser --example transpose_kitchen_sink
cargo test --workspace
```

Music code is represented semantically: pitches, fractional accidentals and
durations, rests, chords, bars and repeats, variant endings, tuplets, grace
groups, slurs, ties, broken rhythm, decorations, annotations, beam boundaries,
and voice overlays all have distinct public AST types. Structured information
fields (`L:`, `M:`, `Q:`, `K:`, `X:`, `V:`, `P:`, `U:`, and `m:`) are parsed
into dedicated value types. In recovery mode, a malformed structured value is
retained as `FieldValue::Unparsed` and accompanied by a diagnostic. Inherently
textual metadata and application-defined fields remain lossless source spans in
`parse_input` results. `IntoOwnedAst::into_owned` resolves those spans to
standalone strings using the original source or conspicuous placeholders when
the source is unavailable. Owned AST nodes implement `ToAbc`, allowing complete
documents or individual fields and music elements to be emitted as canonical
ABC notation after inspection or transformation.

At file level, tunes, free-text blocks, and typed `%%text`, `%%center`, and
`%%begintext` annotations are retained in source order. `ParserOptions` can
independently discard free text or typeset text while continuing to parse and
validate it.
