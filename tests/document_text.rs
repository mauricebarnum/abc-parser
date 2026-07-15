// Copyright 2026 Maurice S. Barnum
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! ABC 2.1 free-text and typeset-text document structure tests.

use abc_parser::DocumentItem;
use abc_parser::ErrorKind;
use abc_parser::Line;
use abc_parser::OwnedDocument;
use abc_parser::ParserOptions;
use abc_parser::Spanned;
use abc_parser::ToAbc;
use abc_parser::TypesetText;
use abc_parser::parse;
use abc_parser::parse_input;
use abc_parser::parse_recovering;
use abc_parser::parse_recovering_with_options;
use abc_parser::validate;
use chumsky::span::SimpleSpan;

const TEXT_DOCUMENT: &str = "%abc-2.1
I:abc-charset utf-8

ABC letters here are prose, not notes.
<p>Raw markup is also free text.</p>

%%text Printed between tunes

%%center Centered between tunes

%%begintext
%%First block line
%% punctuation may begin a block line
%%Second block line
%%endtext

X:1
T:Text directives in a tune
%%center Centered in the tune header
K:C
CDEF |
%%text Printed in the tune body
%%begintext
%%Tune block line
%%endtext
GABc |

Free text after the first tune.

X:2
T:Second tune
K:G
GABc |

Trailing free text.
";

type OwnedLine = Spanned<Line<SimpleSpan<usize>, String>, SimpleSpan<usize>>;

/// Replaces source locations with a common sentinel for logical comparisons.
fn erase_source_locations(document: &mut OwnedDocument<SimpleSpan<usize>>) {
    for line in &mut document.header {
        erase_line_locations(line);
    }
    for item in &mut document.items {
        item.span = SimpleSpan::from(0..0);
        if let DocumentItem::Tune(tune) = &mut item.value {
            for line in &mut tune.lines {
                erase_line_locations(line);
            }
        }
    }
}

/// Replaces locations belonging to one physical line and its music elements.
fn erase_line_locations(line: &mut OwnedLine) {
    line.span = SimpleSpan::from(0..0);
    if let Line::Music(elements) = &mut line.value {
        for element in elements {
            element.span = SimpleSpan::from(0..0);
        }
    }
}

/// Counts retained file-level free-text sections.
fn free_text_count(
    document: &abc_parser::OwnedDocument<chumsky::span::SimpleSpan<usize>>,
) -> usize {
    document
        .items
        .iter()
        .filter(|item| matches!(item.value, DocumentItem::FreeText(_)))
        .count()
}

/// Counts file- and tune-level retained typeset-text nodes.
fn typeset_text_count(
    document: &abc_parser::OwnedDocument<chumsky::span::SimpleSpan<usize>>,
) -> usize {
    let file_count = document
        .items
        .iter()
        .filter(|item| matches!(item.value, DocumentItem::TypesetText(_)))
        .count();
    let tune_count = document
        .tunes()
        .flat_map(|tune| &tune.lines)
        .filter(|line| matches!(line.value, Line::TypesetText(_)))
        .count();
    file_count + tune_count
}

#[test]
fn classifies_ordered_free_and_typeset_text_without_music_errors() {
    let report = parse_recovering(TEXT_DOCUMENT);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.output.tunes().count(), 2);
    assert_eq!(free_text_count(&report.output), 3);
    assert_eq!(typeset_text_count(&report.output), 6);

    let first_free_text = report
        .output
        .items
        .iter()
        .find_map(|item| match &item.value {
            DocumentItem::FreeText(text) => Some(&text.lines),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        first_free_text,
        &[
            "ABC letters here are prose, not notes.",
            "<p>Raw markup is also free text.</p>"
        ]
    );

    let first_tune = report.output.tunes().next().unwrap();
    assert!(
        first_tune
            .lines
            .iter()
            .any(|line| matches!(line.value, Line::TypesetText(TypesetText::Centered(_))))
    );
    assert!(
        first_tune
            .lines
            .iter()
            .any(|line| matches!(line.value, Line::TypesetText(TypesetText::Block(_))))
    );
}

#[test]
fn retention_options_are_independent_and_do_not_change_validation() {
    for (keep_free, keep_typeset, expected_free, expected_typeset) in [
        (true, true, 3, 6),
        (true, false, 3, 0),
        (false, true, 0, 6),
        (false, false, 0, 0),
    ] {
        let options = ParserOptions::new()
            .retain_free_text(keep_free)
            .retain_typeset_text(keep_typeset);
        let report = parse_recovering_with_options(TEXT_DOCUMENT, options);
        assert!(report.errors.is_empty(), "{:#?}", report.errors);
        assert_eq!(report.output.tunes().count(), 2);
        assert_eq!(free_text_count(&report.output), expected_free);
        assert_eq!(typeset_text_count(&report.output), expected_typeset);
    }
}

#[test]
fn malformed_typeset_block_recovers_at_an_empty_line() {
    let source = "%%begintext
%%missing end

X:1
K:C
C |
";
    let report = parse_recovering(source);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.output.tunes().count(), 1);
}

#[test]
fn character_input_preserves_character_index_spans_for_free_text() {
    let characters: Vec<char> = TEXT_DOCUMENT.chars().collect();
    let result = parse_input(characters.as_slice());
    assert!(!result.has_errors());
    let document = result.output().unwrap();
    let free = document
        .items
        .iter()
        .find(|item| matches!(item.value, DocumentItem::FreeText(_)))
        .unwrap();
    assert!(free.span.end <= characters.len());
}

#[test]
fn retained_text_survives_a_logical_emit_round_trip() {
    let parsed = parse_recovering(TEXT_DOCUMENT);
    assert!(parsed.is_valid());
    let emitted = parsed.output.to_abc();
    let reparsed = parse_recovering(&emitted);
    assert!(reparsed.is_valid(), "{:#?}", reparsed.errors);
    assert_eq!(reparsed.output.to_abc(), emitted);
    assert_eq!(free_text_count(&reparsed.output), 3);
    assert_eq!(typeset_text_count(&reparsed.output), 6);

    let mut expected = parsed.output;
    let mut actual = reparsed.output;
    erase_source_locations(&mut expected);
    erase_source_locations(&mut actual);
    assert_eq!(actual, expected);
}

#[test]
fn information_fields_are_diagnosed_inside_free_text() {
    let source = "Ordinary prose.
T:not legal free text

X:1
K:C
C |
";
    let report = parse_recovering(source);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.output.tunes().count(), 1);
    assert_eq!(free_text_count(&report.output), 1);
}

#[test]
fn field_led_blocks_are_tunes_without_reference_fields() {
    let report = parse_recovering("T:No reference\nM:4/4\nK:C\nCDEF |\n");
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert!(report.warnings.is_empty(), "{:#?}", report.warnings);
    assert_eq!(report.output.tunes().count(), 1);

    let report = parse_recovering(
        "X:1\nT:First\nK:C\nCDEF |\n\n% before the second tune\nT:Second\nK:G\nGABc |\n",
    );
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert_eq!(report.output.tunes().count(), 2);
    assert!(matches!(
        report.output.items[1].value,
        DocumentItem::Comment(_)
    ));
}

#[test]
fn malformed_opening_fields_still_select_tune_mode() {
    let report = parse_recovering("M:6/x\nK:C\nCDEF |\n");
    assert_eq!(report.output.tunes().count(), 1);
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.message.contains("M: meter")),
        "{:#?}",
        report.errors
    );
}

#[test]
fn initial_metadata_only_block_remains_the_file_header() {
    let report = parse_recovering("M:4/4\nL:1/8\n");
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert_eq!(report.output.header.len(), 2);
    assert_eq!(report.output.tunes().count(), 0);

    let report = parse_recovering("X:1\nT:Header-only tune\nK:C\n");
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert!(report.output.header.is_empty());
    assert_eq!(report.output.tunes().count(), 1);
}

#[test]
fn music_like_text_warns_without_becoming_invalid_or_a_tune() {
    let source = "% classification comment\nCDEF |\n";
    let report = parse_recovering(source);
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert!(report.has_warnings());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].kind, ErrorKind::MissingReference);
    assert_eq!(report.warnings[0].span, 25..31);
    assert_eq!(report.output.tunes().count(), 0);
    assert!(report.warnings[0].diagnostic(source).contains("2 | CDEF |"));
    assert!(parse(source).is_ok());
    assert!(validate(source).is_ok());

    let ambiguous = parse_recovering("CAGE\n");
    assert!(ambiguous.is_valid());
    assert_eq!(ambiguous.warnings.len(), 1);

    let prose = parse_recovering("ABC letters here are prose.\n");
    assert!(prose.is_valid());
    assert!(prose.warnings.is_empty());

    let later_music = parse_recovering("Ordinary prose.\nCDEF |\n");
    assert!(later_music.is_valid());
    assert!(later_music.warnings.is_empty());

    let two_blocks = parse_recovering("CDEF |\n\nGABc |\n");
    assert!(two_blocks.is_valid());
    assert_eq!(two_blocks.warnings.len(), 2);
}

#[test]
fn possible_music_warning_is_independent_of_text_retention() {
    let report =
        parse_recovering_with_options("CDEF |\n", ParserOptions::new().retain_free_text(false));
    assert!(report.is_valid());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.output.items.is_empty());
}
