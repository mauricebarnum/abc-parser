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
use abc_parser::PlaceholderResolver;
use abc_parser::is_source_reference_placeholder;
use abc_parser::parse_input;
use abc_parser::parse_recovering;

const KITCHEN_SINK: &str = include_str!("../test_kitchen_sink.abc");

#[test]
fn parses_kitchen_sink_with_spans() {
    let report = parse_recovering(KITCHEN_SINK);
    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(report.output.tunes.len(), 2);
    for line in report
        .output
        .header
        .iter()
        .chain(report.output.tunes.iter().flat_map(|tune| &tune.lines))
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
        assert_eq!(report.output.tunes.len(), 2);
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
    assert_eq!(string_result.output().unwrap().tunes.len(), 2);
    let string_owned = string_result
        .into_output()
        .unwrap()
        .into_owned(KITCHEN_SINK)
        .unwrap();
    assert_eq!(string_owned.tunes.len(), 2);

    let characters: Vec<char> = KITCHEN_SINK.chars().collect();
    let character_result = parse_input(characters.as_slice());
    assert!(
        !character_result.has_errors(),
        "{:#?}",
        character_result.errors().collect::<Vec<_>>()
    );
    assert_eq!(character_result.output().unwrap().tunes.len(), 2);
    let character_owned = character_result
        .into_output()
        .unwrap()
        .into_owned(characters.as_slice())
        .unwrap();
    assert_eq!(character_owned.tunes.len(), 2);
}

#[test]
fn document_can_be_detached_without_retaining_the_source() {
    let parsed = parse_input(KITCHEN_SINK).into_output().unwrap();
    let detached = parsed.into_owned(&PlaceholderResolver).unwrap();
    let first_title = detached.tunes[0]
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
