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

//! End-to-end tests using the repository's ABC 2.1 fixture.

use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::OwnedDocument;
use abc_parser::PlaceholderResolver;
use abc_parser::Spanned;
use abc_parser::ToAbc;
use abc_parser::is_source_reference_placeholder;
use abc_parser::parse_input;
use abc_parser::parse_recovering;
use chumsky::span::SimpleSpan;

const KITCHEN_SINK: &str = include_str!("../test_kitchen_sink.abc");

type OwnedLine = Spanned<Line<SimpleSpan<usize>, String>, SimpleSpan<usize>>;

/// Replaces physical-line and music-element locations with a common sentinel.
fn erase_source_locations(document: &mut OwnedDocument<SimpleSpan<usize>>) {
    for line in &mut document.header {
        erase_line_locations(line);
    }
    for item in &mut document.items {
        item.span = SimpleSpan::from(0..0);
        if let abc_parser::DocumentItem::Tune(tune) = &mut item.value {
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

#[test]
fn parses_kitchen_sink_with_spans() {
    let report = parse_recovering(KITCHEN_SINK);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.output.tunes().count(), 2);
    for line in report
        .output
        .header
        .iter()
        .chain(report.output.tunes().flat_map(|tune| &tune.lines))
    {
        assert!(line.span.end <= KITCHEN_SINK.len());
        assert!(line.span.start <= line.span.end);
    }
}

#[test]
fn mutations_report_bounded_faults_and_keep_later_tunes() {
    for needle in ["[CEG]4", "{GAG}", "!trill!", "K:G mixolydian"] {
        let start = KITCHEN_SINK.find(needle).unwrap();
        let mut mutant = KITCHEN_SINK.to_owned();
        mutant.replace_range(start..=start, "@");
        let report = parse_recovering(&mutant);
        assert!(
            !report.errors.is_empty(),
            "mutation of {needle} was accepted"
        );
        assert_eq!(report.output.tunes().count(), 2);
        assert!(
            report
                .errors
                .iter()
                .all(|error| error.span.end <= mutant.len())
        );
    }
}

#[test]
fn chumsky_document_parser_accepts_string_and_character_inputs() {
    let string_result = parse_input(KITCHEN_SINK);
    assert!(
        !string_result.has_errors(),
        "{:#?}",
        string_result.errors().collect::<Vec<_>>()
    );
    assert_eq!(string_result.output().unwrap().tunes().count(), 2);
    let string_owned = string_result
        .into_output()
        .unwrap()
        .into_owned(KITCHEN_SINK)
        .unwrap();
    assert_eq!(string_owned.tunes().count(), 2);

    let characters: Vec<char> = KITCHEN_SINK.chars().collect();
    let character_result = parse_input(characters.as_slice());
    assert!(
        !character_result.has_errors(),
        "{:#?}",
        character_result.errors().collect::<Vec<_>>()
    );
    assert_eq!(character_result.output().unwrap().tunes().count(), 2);
    let character_owned = character_result
        .into_output()
        .unwrap()
        .into_owned(characters.as_slice())
        .unwrap();
    assert_eq!(character_owned.tunes().count(), 2);
}

#[test]
fn document_can_be_detached_without_retaining_the_source() {
    let parsed = parse_input(KITCHEN_SINK).into_output().unwrap();
    let detached = parsed.into_owned(&PlaceholderResolver).unwrap();
    let first_title = detached
        .tunes()
        .next()
        .unwrap()
        .lines
        .iter()
        .find_map(|line| match &line.value {
            abc_parser::Line::Field(abc_parser::Field {
                key: 'T',
                value: abc_parser::FieldValue::Text(text),
                ..
            }) => Some(text),
            _ => None,
        })
        .unwrap();
    assert!(is_source_reference_placeholder(first_title));
}

#[test]
fn emitted_kitchen_sink_parses_as_a_complete_document() {
    let parsed = parse_recovering(KITCHEN_SINK);
    assert!(parsed.is_valid());

    let emitted = parsed.output.to_abc();
    let reparsed = parse_recovering(&emitted);

    assert!(reparsed.errors.is_empty(), "{:#?}", reparsed.errors);
    assert!(emitted.contains("M:4/4"));
    assert!(emitted.contains("[CEG]4"));
    assert!(emitted.contains("|: CDEF GABc | cBAG FEDC :|"));

    let mut expected = parsed.output;
    let mut actual = reparsed.output;
    erase_source_locations(&mut expected);
    erase_source_locations(&mut actual);
    assert_eq!(actual, expected);
}
