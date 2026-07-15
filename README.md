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
cargo run -p abc-parser --bin abc-transpose -- test_kitchen_sink.abc --semitones 1
cargo test --workspace
```

The `abc-transpose` binary writes each transposed tune to standard output. It
accepts a destination key, signed semitones, or signed whole-tone steps in exact
increments of `0.5`:

```sh
abc-transpose tunes.abc --key Dm > tunes-in-d-minor.abc
abc-transpose tunes.abc --semitones -1 > tunes-down-one-semitone.abc
abc-transpose tunes.abc --steps 1.5 > tunes-up-three-semitones.abc
```

Pass `-` as the input path to read ABC from standard input. A zero semitone or
step interval is a byte-preserving no-op.

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
validate it. Field-led blocks are parsed as tunes even without `X:`; a fieldless
block that looks entirely like music remains free text and produces a non-fatal
warning in `ParseReport`.
