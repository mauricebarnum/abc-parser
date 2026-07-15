# abc-parser

A source-spanned, error-recovering parser for ABC music notation 2.1. The crate
offers whole-file recovering parsing as well as entry points for fields,
directives, chords, and individual music lines.

The minimum supported Rust version is 1.95.

`cargo doc --no-deps --open` renders the architecture diagrams as inline SVG.
The generated documentation does not need network access or JavaScript to
display them.

The primary `parse` API accepts any Chumsky `ValueInput<Token = char>` and
returns a `ParseReport` using default `ParserOptions`. Use
`parse_with_options` to configure text retention. The optional source-backed AST
and diagnostics retain the input's native span type, so the same API accepts
strings, character slices, mapped inputs, and value streams without first
converting them to `&str`.

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

Canonical output uses ABC note-length shorthand (`A/`, `A//`, `A///`) for
power-of-two divisors. Pass `--explicit-note-lengths` to emit the equivalent
explicit denominators (`A/2`, `A/4`, `A/8`) instead:

```sh
abc-transpose tunes.abc --key Dm --explicit-note-lengths
```

Pass `-` as the input path to read ABC from standard input. A zero semitone or
step interval is a byte-preserving no-op unless an emission preference such as
`--explicit-note-lengths` requests canonical re-emission.

Music code is represented semantically: pitches, fractional accidentals and
durations, rests, chords, bars and repeats, variant endings, tuplets, grace
groups, slurs, ties, broken rhythm, decorations, annotations, beam boundaries,
and voice overlays all have distinct public AST types. Structured information
fields (`L:`, `M:`, `Q:`, `K:`, `X:`, `V:`, `P:`, `U:`, and `m:`) are parsed
into dedicated value types. In recovery mode, a malformed structured value is
retained as `FieldValue::Unparsed` and accompanied by a diagnostic. Inherently
textual metadata and application-defined fields remain lossless source spans in
`parse` results. `IntoOwnedAst::into_owned` resolves those spans to
standalone strings using the original source or conspicuous placeholders when
the source is unavailable. Owned AST nodes implement `ToAbc`, allowing complete
documents or individual fields and music elements to be emitted as canonical
ABC notation after inspection or transformation. `AbcEmitter` and `EmitOptions`
configure equivalent canonical spellings while carrying one shared emission
context through the complete AST.

At file level, tunes, free-text blocks, and typed `%%text`, `%%center`, and
`%%begintext` annotations are retained in source order. `ParserOptions` can
independently discard free text or typeset text while continuing to parse and
validate it. Field-led blocks are parsed as tunes even without `X:`; a fieldless
block that looks entirely like music remains free text and produces a non-fatal
warning in `ParseReport`.
