# ABC language server

`abc-language-server` provides editor assistance for ABC 2.1 documents over
standard input and output using Language Server Protocol 3.18.

Build or install it from this workspace:

```sh
cargo build --release -p abc-language-server
cargo install --path abc-language-server
```

Configure an LSP client to run `abc-language-server` for `*.abc` files. The
server uses full document synchronization and negotiates UTF-8 or UTF-16
positions with the client. It currently provides:

- versioned push diagnostics from the recovering ABC parser;
- field, directive, meter, and key completions;
- hover help for fields and music tokens;
- tune and voice document symbols;
- tune and `%%begintext` folding ranges;
- syntax-aware selection ranges and semantic tokens;
- conservative document and range formatting; and
- source-local code actions for equivalent note-length spellings.

Formatting is disabled when the document has parse errors. By default it only
honors the client's trailing-whitespace and final-newline preferences; it does
not replace valid ABC spellings.

## Configuration

The server reads the `abc` configuration section through
`workspace/configuration`, including resource-scoped values when the client
supports them. The same object can be supplied as `initializationOptions` by a
client without configuration support:

```json
{
  "abc": {
    "validation": {
      "strict": false,
      "ambiguousMusic": "warning",
      "barDuration": "warning",
      "legacyDecoration": "warning"
    },
    "format": {
      "noteLength": "preserve"
    }
  }
}
```

`ambiguousMusic`, `barDuration`, and `legacyDecoration` accept `off`, `hint`,
`information`, `warning`, or `error`. Strict validation requires tune structure
such as an `X:` reference field; it is off by default so incomplete files
remain useful while editing.

`barDuration` checks closed, non-empty bars against their effective meter. It
allows an underfull opening bar in each voice and metered section as a possible
pickup, and does not diagnose the trailing open bar while it is being edited.

`format.noteLength` accepts:

- `preserve` (default), which leaves the author's spelling unchanged;
- `shorthand`, which uses repeated slashes for power-of-two divisors; or
- `explicit`, which writes the numeric denominator.

This preference is only an emission style. The parser needs no option to
understand either notation: `A/` and `A/2` both mean one half of the unit note
length, while `A//` and `A/4` both mean one quarter. The server also offers the
two conversions as code actions over a selected range, independent of the
formatting preference.

The diagnostic settings are interpretation choices because they affect which
potential problems are reported. Note-length style is not an interpretation
choice because all equivalent spellings are accepted without configuration.

## Protocol scope

The server advertises only implemented capabilities. Version 0.1 intentionally
does not provide definition, references, rename, workspace symbols, file
operations, commands, or rendered-score previews. Unknown requests continue to
receive the standard method-not-found response from the protocol framework.
