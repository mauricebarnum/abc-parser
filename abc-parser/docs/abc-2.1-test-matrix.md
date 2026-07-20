# ABC 2.1 parser test matrix

The normative source for this matrix is `abc_standard_v2.1.pdf`. Section
numbers below refer to that document. The matrix distinguishes syntax that the
parser must recognize from rendering or playback behavior that belongs to a
consumer of the AST.

| Standard section | Positive and edge cases | Negative cases | Evidence |
|---|---|---|---|
| 2.1 File identification | `%abc`, `%abc-2.1`, initial UTF-8 BOM | misplaced BOM | `abc_2_1_conformance`, `document_text` |
| 2.2 File structure | file header, headerless tune, empty tune body, tunebook, free and typeset text, whitespace separators | field in free text, unterminated text block | `abc_2_1_conformance`, `document_text`, `kitchen_sink` |
| 2.2.4, 8 Line endings | LF, CRLF, CR, trailing horizontal whitespace | nonblank text where a separator is required | `abc_2_1_conformance`, `document_text` |
| 2.2.5 Comments | full-line, indented, end-of-line, escaped percent in text | unescaped comment text parsed as music | `abc_2_1_conformance` |
| 2.2.6, 3.3 Continuation | music `\`, field `+:`, intervening comments/directives | continuation after no field, continuation through an empty line | `abc_2_1_conformance`; representation limits are in the report |
| 3.1 Information fields | every defined letter, repeated textual fields, unknown letters retained | non-letter/multi-letter keys | `abc_2_1_conformance`, unit tests |
| 3.1.1 `X:` | positive integer, empty value | zero, negative, duplicate in one tune | `abc_2_1_conformance`; empty-value representation is tracked in the report |
| 3.1.6 `M:` | `C`, `C|`, `none`, simple and additive meters, optional numerator parentheses | zero denominator, incomplete additive meter | `abc_2_1_conformance`, unit tests |
| 3.1.7 `L:` | `1`, `1/1`, common powers of two | zero denominator, non-fractional text | `abc_2_1_conformance`, unit tests |
| 3.1.8 `Q:` | one to four beats, prelude/postlude, text only | zero denominator, more than four beats, missing BPM | `abc_2_1_conformance` |
| 3.1.9 `P:` | part names, repeats, separators, nested groups | unbalanced groups, unsupported character | `abc_2_1_conformance`, unit tests |
| 3.1.14 `K:` | tonic/mode aliases, `none`, empty key, bagpipe keys, explicit accidentals, clef-only and transposition parameters | invalid tonic/accidental/parameter shape | `abc_2_1_conformance`, unit tests |
| 3.1.17 `I:` | standard and unknown instructions retained | malformed field key | `abc_2_1_conformance` |
| 3.2, 7.3 Inline fields | `I`, `K`, `L`, `M`, `P`, `Q`, `V`, and `r` | missing bracket, multiple fields in one bracket | `abc_2_1_conformance`, unit tests |
| 4.1 Pitch | both cases, repeated and mixed comma/apostrophe modifiers | modifier without note, invalid pitch letter | `abc_2_1_conformance`, unit tests |
| 4.2 Accidentals | natural, single/double sharp and flat | accidental without pitch, zero microtone denominator | unit tests |
| 4.3 Note lengths | integer, fraction, slash shorthand through 1/128, arbitrary legal rational | zero denominator, malformed slash suffix | `abc_2_1_conformance`, unit tests |
| 4.4 Broken rhythm | one through three `<`/`>`, grace-transparent placement | malformed/non-ASCII operator | `abc_2_1_conformance`, unit tests |
| 4.5 Rests | `z`, `x`, `Z`, `X`, explicit/default lengths and counts | zero/overflow count policy | `abc_2_1_conformance`, unit tests |
| 4.6 Clefs/transposition | positional and named clefs, signed transpose/octave, middle/stafflines | malformed named parameter | field cases above |
| 4.7 Beams | adjacency, spaces/tabs, ignored backquotes | none (layout semantics are consumer behavior) | `abc_2_1_conformance`, unit tests |
| 4.8 Bars | named bars, repeat variants, dotted/invisible, liberal bar sequences | isolated thick-bar bracket | `abc_2_1_conformance`, unit tests |
| 4.9-4.10 Endings | `[1`, `[1,3,5-7`, `|1`, `:|2` | spaces in selector list, descending/empty range policy | `abc_2_1_conformance`, unit tests |
| 4.11 Ties/slurs | plain/dotted, nested, chords | detached tie and invalid ordering | `abc_2_1_conformance`; contextual validation is tracked in the report |
| 4.12 Grace notes | appoggiatura, acciaccatura, lengths and broken rhythm | empty/unclosed group | `abc_2_1_conformance`, unit tests |
| 4.13 Tuplets | compact 2-9, omitted `q`/`r`, fully explicit | zero/overflow and malformed components | `abc_2_1_conformance`, unit tests |
| 4.14 Decorations | standard shorthand and `!name!`, dialectal `+name+` | empty/unclosed/forbidden characters | `abc_2_1_conformance`, unit tests |
| 4.15-4.16 Symbols | `s:` retention and `U:` definitions | malformed assignment, invalid redefinable symbol | `abc_2_1_conformance`, unit tests |
| 4.17 Chords | notes/rests, individual and outer lengths, accidentals, unison | empty, spaces, unclosed, malformed member | `abc_2_1_conformance`, unit tests |
| 4.18-4.19 Quoted text | chord symbols, all annotation placements, Unicode accidentals | unterminated quote | `abc_2_1_conformance`, unit tests |
| 4.20 Construct order | full legal prefix/suffix order | detached or misplaced postfix operators | `abc_2_1_conformance`; contextual validation is tracked in the report |
| 5 Lyrics | `w:`, `W:`, empty verse, numbering, alignment characters, continued lyrics | syntax retained losslessly; note/lyric cardinality is advisory consumer behavior | `abc_2_1_conformance` |
| 6.1 Typesetting controls | `\`, `$`, `$$`, `y`, text directives | continuation before empty line | `abc_2_1_conformance`, `document_text` |
| 7 Multiple voices | physical/inline `V:`, properties, `&`, `(&`, `&)` | overlay extent is semantic consumer validation | `abc_2_1_conformance`, unit tests |
| 8.1 Forward compatibility | reserved `# * ; ? @` between groups | reserved character in a structured field remains significant | `abc_2_1_conformance` |
| 8.2 Text strings | UTF-8, mnemonic/entity/fixed-width encodings, escaped `%`, quoted placement text | raw `%` starts a comment, unterminated annotation | `abc_2_1_conformance` |
| 9 Macros | static/transposing definitions retained structurally | missing target/replacement, length limits | unit tests; expansion is intentionally consumer behavior |
| 10.1 Outdated fields | deprecated `A:` and `E:` warnings, implicit multiline `H:` retention, raw `Q:C=<digits>` and `Q:<digits>` payloads | unsupported `Q:C2=<digits>` form | `abc_2_1_conformance` |
| 10, 12 Dialects | deprecated syntax only when its instruction selects that dialect | obsolete chord delimiters, conflicting delimiters in strict mode | partially implemented; see report |
| 11 Directives | standard and application names retained, text directives structured | empty/invalid name, unterminated text block | `abc_2_1_conformance`, `document_text`, unit tests |

The conformance suite deliberately tests parsing and syntax diagnostics. Musical
timing, typeset placement, macro expansion, lyric-to-note alignment, and
playback state are not silently claimed as parser validation when the public
AST only retains the notation for a downstream consumer.
