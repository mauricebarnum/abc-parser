
# Parser architecture

This section describes how character input reaches the public AST. Parser
constructors are generic over Chumsky's `ValueInput<Token = char>` (a by-value
character-input specialization of `Input`). Every [`Spanned`] node uses the
input's native `Input::Span`: `&str` therefore reports UTF-8 byte offsets while
`&[char]` reports character indices.

## Entry points

[`parse`] is the complete-document API. It accepts any compatible character
input plus [`ParserOptions`] and returns a [`ParseReport`] whose optional output
is a source-backed [`ParsedDocument`]. Errors and warnings use the input's native
span type. Recovery normally supplies output alongside diagnostics; an
unrecoverable failure returns no output. Advisory [`ParseReport::warnings`] do
not make the report invalid.

Generic partial-input constructors are [`line_parser`], [`music_line_parser`],
[`music_element_parser`], [`field_parser`], [`directive_parser`], and
[`chord_parser`]. Convenience functions for `&str` remain available as
[`parse_line`], [`parse_music_line`], [`parse_field`], [`parse_directive`], and
[`parse_chord`].

```mermaid
flowchart TD
    source["ValueInput Token=char"] --> entry{Public entry point}
    entry -->|document| recovering[parse]
    entry -->|physical line| line[line_parser]
    entry -->|music fragment| music[music_line_parser]
    entry -->|field/directive/chord| partial[Partial parser combinators]
    recovering --> split[Split at blank lines]
    split --> classify[Classify from first non-comment line]
    line --> classify
    classify --> blank[Blank]
    classify --> comment[Comment]
    classify --> directive[Directive parser]
    classify --> field[Field parser]
    classify -->|field-led block| music
    classify -->|raw block| free[Free text]
    field --> field_value[Structured field-value parser]
    music --> elements[Music element combinators]
    elements --> ast[Spanned AST nodes]
    field_value --> ast
    directive --> ast
    ast --> ordered[Assemble header, tunes, free text, and typeset text]
    ordered --> filter[Apply ParserOptions retention]
    filter --> parsed[ParsedDocument with SourceText spans]
    parsed -->|IntoOwnedAst + source| owned[OwnedDocument with strings]
    parsed -->|PlaceholderResolver| detached[OwnedDocument with reference placeholders]
```

The diagram is Mermaid source so it remains portable in generated rustdoc and
renders in documentation front ends that support Mermaid. The surrounding text
contains the same information for plain rustdoc viewers.

## Complete-document flow

The private complete-document parser composes the source at physical-line
granularity:

1. Empty lines divide the input into blocks and terminate tunes.
2. Leading `%` comments are transparent to classification. An ASCII letter
   followed by `:` selects tune mode; directives and raw lines select text mode.
3. Tune lines are parsed immediately with diagnostic music recovery. Text lines
   retain raw spans without invoking the music grammar.
4. A strict probe of a raw deciding line records a non-fatal
   [`ErrorKind::MissingReference`] warning when the whole line is valid music;
   the block remains free text.
5. `map_with` attaches native input spans. Parser state carries advisory spans
   separately from Chumsky's `Rich` error channel.
6. The initial metadata-only block remains the file header. Later field-led
   blocks become [`Tune`] values even when their first field is not `X:`.
7. Typeset text and comments are assembled in source order before applying
   [`ParserOptions`] retention.

Physical lines are intentional recovery boundaries. ABC fields, directives,
comments, and most music layout constructs cannot consume an arbitrary following
line, so the next line is a dependable place to resume after an unclosed chord,
grace group, decoration, or annotation.

## Free text and typeset text

[`Document::items`] preserves the order of tunes, [`FreeText`] blocks,
file-level [`TypesetText`], comments, and stylesheet directives after the
optional initial header. A tune ends at an empty line or EOF. This prevents
letters in inter-tune prose from being interpreted as notes. A fieldless block
always remains free text; if its deciding line is valid music, [`parse`] reports
an advisory warning.

`%%text` and `%%center` become typed text nodes. `%%begintext` through
`%%endtext` becomes one block node; each standard body line must begin with
`%%`. The same nodes may occur in tune headers and bodies through
[`Line::TypesetText`].

[`ParserOptions`] retains both text categories by default. Its
[`ParserOptions::retain_free_text`] and
[`ParserOptions::retain_typeset_text`] builders independently omit the
corresponding AST nodes. Parsing and validation still occur before omission, so
retention choices never hide diagnostics.

## Information-field flow

[`Field`] records the original letter as [`Field::key`], its standard category
as [`Field::kind`], and a semantic [`FieldValue`]. Structured values use
dedicated parsers:

| Field | AST payload | Examples |
| --- | --- | --- |
| `L:` | [`FieldValue::UnitLength`] | `L:1/8` |
| `M:` | [`FieldValue::Meter`] | `M:3/4`, `M:2+3/8`, `M:C` |
| `Q:` | [`FieldValue::Tempo`] | `Q:1/4=120` |
| `K:` | [`FieldValue::Key`] | `K:G mixolydian clef=bass` |
| `X:` | [`FieldValue::Reference`] | `X:12` |
| `V:` | [`FieldValue::Voice`] | `V:1 name="Soprano"` |
| `P:` | [`FieldValue::Parts`] | `P:A.B.(CD)2` |
| `U:` | [`FieldValue::UserSymbol`] | `U:H=!fermata!` |
| `m:` | [`FieldValue::Macro`] | `m:~n2 = ...` |

Metadata such as titles and composers is represented by
[`FieldValue::Text`]. Application-defined field letters are classified as
[`FieldKind::Extension`] and also retain their text.

Strict [`parse_field`] returns [`ParseError`] when a structured value is
malformed. During document or line recovery, the same payload is retained as
[`FieldValue::Unparsed`] and a diagnostic is added. Inline fields follow the
same rule.

```mermaid
flowchart LR
    raw["L:not-a-length"] --> strict{Mode}
    strict -->|parse_field| error[Err ParseError]
    strict -->|recovering document/line| both["FieldValue::Unparsed + diagnostic"]
    valid["L:1/16"] --> parsed["FieldValue::UnitLength Fraction 1/16"]
```

## Music-code flow

[`music_line_parser`] is
`music_element_parser().repeated().at_least(1).collect()`. The element parser is
a `choice` of bracketed constructs, grace groups, quoted annotations,
decorations, grouped operators, notes/rests, and a one-character recovery
fallback. Repetition and delimiter ownership are handled by Chumsky primitives;
`validate` emits non-fatal `Rich` errors, and `map_with` attaches the native
input span.

Bracketed input is disambiguated before general tokens:

1. `[A:value]` is an inline field.
2. `[1`, `[1,3`, and `[2-4` are variant endings.
3. `[|...` is a bar line.
4. Other bracketed input is a [`Chord`].

Notes are recognized by chained `one_of`, `repeated`, `then`, and `map`
combinators, then decomposed into [`Pitch`], optional [`Accidental`], octave,
and [`NoteLength`]. Chords contain typed [`ChordMember`] values rather than
source strings. Chumsky enforces non-empty, delimiter-safe chord interiors while
producing semantic members directly; there is no secondary string scanner.

## Source-backed text and ownership

[`ParsedDocument`] stores source-derived text as [`SourceText::Span`] using the
input's native span type. Values introduced or normalized by parsing use
[`SourceText::Synthesized`]. Consequently, parsing does not allocate a `String`
for every title, comment, directive argument, decoration, or recovery token.
The span-only AST itself does not borrow the source and may outlive it, although
exactly recovering its text later requires the matching source.

[`IntoOwnedAst::into_owned`] recursively converts a parsed document or subtree
to its `String`-backed equivalent. Passing the original `str` resolves
`SimpleSpan<usize>` as UTF-8 byte offsets; passing the original `[char]` resolves
the same span type as character indices. Invalid source/span combinations
return [`ResolveError`]. This operation copies source-backed text into the
standalone tree; already synthesized strings are moved without copying.

When the source is unavailable or unwanted, [`PlaceholderResolver`] emits the
documented form `[[ABC_SOURCE_REF:<Debug span>]]`. Use
[`is_source_reference_placeholder`] for heuristic detection. A legitimate ABC
source can contain the same shape, so detection cannot prove provenance.

```mermaid
flowchart LR
    input[Input source] --> parse
    parse --> report[ParseReport]
    report --> parsed[ParsedDocument SourceText spans]
    parsed -->|source resolver| exact[OwnedDocument exact strings]
    parsed -->|PlaceholderResolver| placeholders[OwnedDocument reference placeholders]
    exact -->|ToAbc| emitted[Canonical ABC source]
```

## ABC source emission

[`ToAbc`] writes owned or otherwise `AsRef<str>`-backed AST nodes as canonical
ABC notation. It is implemented for complete documents and tunes as well as
individual lines, fields, music elements, pitches, durations, and other public
semantic nodes. [`AbcEmitter`] carries a destination, [`EmitOptions`], and any
context needed while recursively emitting those nodes. The convenience
[`ToAbc::to_abc`] method uses default options; [`ToAbc::to_abc_with_options`]
selects equivalent spellings such as shorthand or explicit note-length
denominators.

Source positions are ignored. Stored textual values and bar spellings are
preserved, while normalized syntax such as note lengths, accidentals,
decorations, and field parameters uses deterministic spellings.

A [`ParsedDocument`] must first be converted with [`IntoOwnedAst::into_owned`]
and an appropriate resolver. [`OwnedDocument`] can be emitted directly with
[`ToAbc::to_abc`]. Emission deliberately produces semantic ABC rather than a
byte-for-byte reconstruction: comments and metadata remain intact, but optional
spacing, quote choices, shorthand decorations, and similar presentation details
may be canonicalized. Text inserted programmatically into a quoted AST position
must not contain an unescaped quote delimiter.

## AST overview

The AST separates document organization, line syntax, structured field values,
and music syntax. [`Spanned<T>`] is used at line and music-element boundaries;
semantic children such as [`Pitch`] do not repeat their parent's span.

```mermaid
classDiagram
    Document *-- DocumentItem
    DocumentItem *-- Tune
    DocumentItem *-- FreeText
    DocumentItem *-- TypesetText
    Document *-- SpannedLine : header
    Tune *-- SpannedLine : lines
    SpannedLine *-- Line
    Line <|-- DirectiveLine
    Line <|-- FieldLine
    Line <|-- MusicLine
    Line <|-- TypesetText
    FieldLine *-- Field
    Field *-- FieldKind
    Field *-- FieldValue
    FieldValue <|-- UnitLength
    FieldValue <|-- Meter
    FieldValue <|-- Tempo
    FieldValue <|-- KeySignature
    FieldValue <|-- VoiceDefinition
    SourceText <|-- SourceSpan
    SourceText <|-- SynthesizedString
    FieldValue *-- SourceText
    MusicLine *-- SpannedMusicElement
    SpannedMusicElement *-- MusicElement
    MusicElement <|-- Note
    MusicElement <|-- Rest
    MusicElement <|-- Chord
    MusicElement <|-- BarLine
    MusicElement <|-- VariantEnding
    MusicElement <|-- Tuplet
    MusicElement <|-- GraceGroup
    Note *-- Pitch
    Note *-- NoteLength
    Pitch *-- Accidental
    Chord *-- ChordMember
```

The principal [`MusicElement`] families are:

- sound-producing or timing nodes: [`Note`], [`Rest`], [`MultiMeasureRest`],
  and [`Chord`];
- measure structure: [`BarLine`], [`VariantEnding`], and [`Tuplet`];
- phrasing and rhythm: [`Slur`], [`Tie`], [`BrokenRhythm`], and [`GraceGroup`];
- presentation: [`Decoration`], [`Annotation`], beam boundaries, and
  [`LineBreak`];
- multi-voice and context changes: [`Overlay`] and inline [`Field`] nodes.

[`MusicElement::Extension`] is reserved for syntax that cannot be assigned a
standard semantic node. When it represents malformed input it is accompanied by
a diagnostic, preserving bytes without pretending they were understood.

## Errors and recovery invariants

[`ParseError::span`] always refers to the original document, including for
errors found in nested inline fields. Diagnostics are emitted in encounter
order. Recovery maintains these invariants:

- repeated element parsing always advances;
- every emitted span is half-open and within the input;
- a malformed structured field retains its payload span and ownership
  conversion trims the resulting fallback string;
- an unclosed delimited music construct consumes no later physical line;
- a later tune can still be discovered after faults in an earlier tune.

All complete-document callers use [`parse`]. Batch tools may reject reports with
errors, while interactive tools can inspect recovered output and render every
diagnostic.
