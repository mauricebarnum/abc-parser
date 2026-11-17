# abc-musicxml

`abc-musicxml` converts ABC 2.1 music notation into W3C MusicXML 4.0
partwise scores (`<score-partwise>`) and multi-tune opus collections
(`opus.xml`), with optional source provenance annotations.

Build or install it from this workspace:

```sh
cargo build --release -p abc-musicxml
cargo install --path abc-musicxml
```

Or using [`Justfile`](../Justfile):

```sh
just run-musicxml -- [args]
just install-musicxml
```

## Features

- **Standards-compliant MusicXML 4.0**: Generates `<score-partwise version="4.0">`
  output validated against the normative W3C `musicxml.xsd` and `opus.xsd` schemas.
- **Exact rational timing**: Computes least common multiple (LCM) `<divisions>`
  per quarter note across tuplets, broken rhythms, and compound durations.
- **Full pitch & key coverage**: Circle-of-fifths key signatures, modal scales
  (Dorian, Phrygian, Lydian, Mixolydian, Aeolian, Locrian), Highland Bagpipe keys
  (`K:HP` and `K:Hp`), microtones, intra-measure accidental propagation, and
  cross-measure ties.
- **Polyphonic voices & score directives**: Lowers `V:` voices, `&` overlays,
  and `%%score` / `%%staves` directives into part groups, braced multi-staff
  keyboards (single coupled parts with `<part-symbol>brace</part-symbol>` and
  lagging staff padding), multi-voice single staves, and voice layers coordinated
  with `<backup>`.
- **Playback & layout directives**: Lowers `%%MIDI` channel, program, volume,
  and pan directives into `<score-instrument>` and `<midi-instrument>` definitions;
  lowers `$`, `!linebreak!`, and `!newpage!` to `<print>` break directives;
  and documents typesetting stylesheet parameters in XML comments.
- **Unpitched percussion**: Maps percussion clefs (`clef=perc`, `K:perc`) to
  `<unpitched>` notes with display pitch coordinates.
- **Compressed `.mxl` archives**: Supports standard compressed `.mxl` ZIP containers
  conforming to W3C `container.xsd` with `META-INF/container.xml`.
- **Expressive notations & lyrics**: Maps decorations to `<articulations>` and
  `<ornaments>`, chords to `<harmony>`, and `w:` lines to aligned `<lyric>`
  elements with syllabic state (`begin`, `middle`, `end`, `single`) and melisma extend.
- **Two-level source provenance**: Embeds originating ABC source as standard
  XML comments without violating XML Schema validity.

## Command-line usage

```sh
# Convert a single-tune ABC file to standard output
abc-musicxml tune.abc

# Select a specific tune from a multi-tune collection
abc-musicxml tunes.abc -x 1 -o tune-1.musicxml

# Batch convert all tunes to a directory with an opus.xml collection catalog
abc-musicxml tunes.abc --out-dir output/

# Embed source line comments
abc-musicxml tune.abc -a line

# Embed detailed line/column and byte-offset span comments on each element
abc-musicxml tune.abc -a verbose

# Best-effort conversion of files containing recoverable syntax errors
abc-musicxml tune.abc --recover

# Omit the standard DOCTYPE declaration
abc-musicxml tune.abc --no-doctype
```

### CLI Options

| Flag | Description |
|---|---|
| `-x, --tune <REF>` | Select a specific tune by its `X:` reference number |
| `-o, --out <FILE>` | Write output to a file instead of stdout |
| `--out-dir <DIR>` | Batch convert all tunes to `<DIR>/<stem>-<X>.musicxml` (or `.mxl`) and generate `opus.xml` |
| `--mxl` | Emit compressed `.mxl` archive container (auto-detected when output ends in `.mxl`) |
| `-a, --annotate <LEVEL>` | Provenance comments: `none` (default), `line`, or `verbose` |
| `--no-doctype` | Omit `<!DOCTYPE score-partwise ...>` declaration |
| `--recover` | Best-effort conversion of inputs with syntax errors |
| `-V, --version` | Print version information (combine with `-v` for detailed build info) |
| `-v, --verbose` | Enable verbose diagnostics or detailed version output |

## Source provenance annotations

The converter supports embedding originating ABC source lines and token spans
as standard XML comments preceding generated elements.

- **Level 0 (`AnnotationLevel::None`)**: Clean MusicXML output with no comments.
- **Level 1 (`AnnotationLevel::Line`)**: Emits source file, 1-based line number,
  and verbatim line text before generated constructs:
  ```xml
  <!-- test.abc:12: |: CDEF GABc | cBAG FEDC :| -->
  ```
- **Level 2 (`AnnotationLevel::Verbose`)**: Emits Level 1 line comments plus
  Hybrid Option D span comments with 1-based line:column coordinates, 0-based
  UTF-8 byte ranges, and source token snippets:
  ```xml
  <!-- @12:4..12:5 [240..241]: C -->
  <note>
    <pitch>
      <step>C</step>
      <octave>4</octave>
    </pitch>
    ...
  </note>
  ```

All comments are sanitized per XML 1.0 §2.5 rules (illegal `--` sequences and
trailing hyphens are percent-encoded) to guarantee 100% schema validation.

## Library API

```rust
use abc_musicxml::AnnotationLevel;
use abc_musicxml::MusicXmlOptions;
use abc_musicxml::tune_to_musicxml;
use abc_parser::parse;

let source = "X:1\nT:Sample\nM:4/4\nL:1/4\nK:C\nc d e f |\n";
let parsed = parse(source);
let doc = parsed.output.unwrap().into_owned(source).unwrap();
let tune = doc.tunes().next().unwrap();

let options = MusicXmlOptions::new()
    .with_source_name("sample.abc")
    .with_annotation_level(AnnotationLevel::Verbose);

let output = tune_to_musicxml(tune, &doc.header, Some(source), &options).unwrap();
println!("{}", output.xml);
```

## Cargo Features

- **`mxl`** (enabled by default): Adds compressed `.mxl` ZIP container support via `zip`.
- **`parallel`** (enabled by default): Enables multi-core work-stealing parallel tune conversion and `--out-dir` batch writing via `rayon`. Disable default features (`--no-default-features`) for minimal or single-threaded environments (such as WASM).
