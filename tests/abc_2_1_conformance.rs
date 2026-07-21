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

//! Section-linked conformance tests derived from the normative ABC 2.1 PDF.

use abc_parser::AnnotationPlacement;
use abc_parser::ErrorKind;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::GraceElement;
use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::Meter;
use abc_parser::MusicElement;
use abc_parser::ParserOptions;
use abc_parser::PitchClass;
use abc_parser::SourceText;
use abc_parser::Tempo;
use abc_parser::ToAbc;
use abc_parser::parse;
use abc_parser::parse_field;
use abc_parser::parse_music_line;
use abc_parser::parse_with_options;

fn assert_document_valid(source: &str) {
    let report = parse(source);
    assert!(report.is_valid(), "{source:?}: {:#?}", report.errors);
}

fn assert_music_valid(source: &str) -> Vec<MusicElement<String>> {
    let report = parse_music_line(source);
    assert!(report.is_valid(), "{source:?}: {:#?}", report.errors);
    report
        .output
        .unwrap()
        .into_iter()
        .map(|element| element.value)
        .collect()
}

#[test]
fn sections_2_and_8_accept_bom_and_all_required_line_endings() {
    for newline in ["\n", "\r\n", "\r"] {
        let source = ["\u{feff}%abc-2.1", "X:1", "T:Line endings", "K:C", "CDEF |"].join(newline);
        let report = parse(source.as_str());
        assert!(report.is_valid(), "{newline:?}: {:#?}", report.errors);
        assert_eq!(report.output.unwrap().tunes().count(), 1, "{newline:?}");
    }
}

#[test]
fn section_2_2_5_ignores_indented_and_end_of_line_comments() {
    assert_document_valid(
        "%abc-2.1\nX:1 % reference\nT:Comments \\% remain text % comment\nK:C % key\n  % whole line\nCDEF| % music\n",
    );

    let title = parse_field("T:100 \\% traditional % source note").unwrap();
    assert!(matches!(title.value, FieldValue::Text(ref value) if value == "100 \\% traditional "));
}

#[test]
fn sections_3_1_6_and_3_1_7_cover_standard_meter_and_unit_lengths() {
    for source in ["M:C", "M:C|", "M:none", "M:6/8", "M:2+3+2/8", "M:(2+3+2)/8"] {
        assert!(matches!(
            parse_field(source).unwrap().value,
            FieldValue::Meter(_)
        ));
    }
    assert!(matches!(
        parse_field("M:(2+3+2)/8").unwrap().value,
        FieldValue::Meter(Meter::Compound {
            ref groups,
            denominator: 8
        }) if groups == &[2, 3, 2]
    ));

    for source in ["L:1", "L:1/1", "L:1/2", "L:1/8", "L:1/128"] {
        assert!(matches!(
            parse_field(source).unwrap().value,
            FieldValue::UnitLength(_)
        ));
    }
    for source in ["M:3/0", "M:(2+)/8", "L:1/0", "L:nope"] {
        assert!(parse_field(source).is_err(), "{source}");
    }
}

#[test]
fn sections_3_1_1_and_3_1_14_accept_permitted_empty_fields() {
    assert!(matches!(
        parse_field("X:").unwrap().value,
        FieldValue::Empty
    ));
    assert!(matches!(
        parse_field("K:  ").unwrap().value,
        FieldValue::Empty
    ));
    assert!(parse_field("X:0").is_err());
    assert_document_valid("%abc-2.1\nX:\nT:Unnumbered\nK:\n");
}

#[test]
fn sections_2_2_1_and_3_1_1_enforce_required_fields_in_strict_mode() {
    for source in [
        "X:1\nK:C\n",
        "X:1\nT:No key\n",
        "T:No reference\nK:C\n",
        "X:1\nT:Duplicate reference\nX:2\nK:C\n",
    ] {
        let report = parse_with_options(source, ParserOptions::new().strict(true));
        assert!(!report.is_valid(), "{source:?}");
    }
    assert!(
        parse_with_options(
            "X:\nT:Empty reference is legal\nK:\n",
            ParserOptions::new().strict(true)
        )
        .is_valid()
    );

    assert!(!parse("%abc-2.1\nX:1\nK:C\n").is_valid());
    assert!(parse("%abc-2.0\nX:1\nK:C\n").is_valid());
}

#[test]
fn sections_2_2_6_and_3_3_recognize_field_continuations() {
    let report = abc_parser::parse_line("+:continued text % comment");
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert!(matches!(
        report.output,
        Some(Line::FieldContinuation(ref text)) if text == "continued text "
    ));
    assert_document_valid(
        "%abc-2.1\nX:1\nT:Continued fields\nH:first line\n+:second line\n% between continuations\n+:third line\nK:C\nCDEF |\nw:long lyr-ic line\n+:con-tin-ued lyr-ics\n",
    );
}

#[test]
fn section_3_1_8_accepts_all_standard_tempo_shapes() {
    for source in [
        "Q:1/4=120",
        "Q:\"Allegro\" 1/4=120",
        "Q:3/8=50 \"Slowly\"",
        "Q:1/4 3/8 1/4 3/8=40",
        "Q:\"Andante\"",
    ] {
        assert!(
            matches!(parse_field(source).unwrap().value, FieldValue::Tempo(_)),
            "{source}"
        );
    }
    assert!(parse_field("Q:1/8 1/8 1/8 1/8 1/8=60").is_err());
}

#[test]
fn section_3_1_8_retains_text_only_tempo_in_the_ast() {
    let report = parse("X:1\nT:Tempo text\nQ:\"Andante espressivo\"\nK:C\n");
    assert!(report.is_valid(), "{:#?}", report.errors);

    let tempo = report
        .output
        .as_ref()
        .unwrap()
        .tunes()
        .flat_map(|tune| &tune.lines)
        .find_map(|line| match &line.value {
            Line::Field(field) => match &field.value {
                FieldValue::Tempo(tempo) => Some(tempo),
                _ => None,
            },
            _ => None,
        })
        .unwrap();

    assert!(matches!(
        tempo,
        Tempo::TextOnly(SourceText::Synthesized(text)) if text == "Andante espressivo"
    ));
}

#[test]
fn section_3_1_8_retains_metronome_mark_text_in_the_ast() {
    let report = parse("X:1\nT:Tempo text\nQ:\"Allegro\" 1/4=120 \"brightly\"\nK:C\n");
    assert!(report.is_valid(), "{:#?}", report.errors);

    let tempo = report
        .output
        .as_ref()
        .unwrap()
        .tunes()
        .flat_map(|tune| &tune.lines)
        .find_map(|line| match &line.value {
            Line::Field(field) => match &field.value {
                FieldValue::Tempo(tempo) => Some(tempo),
                _ => None,
            },
            _ => None,
        })
        .unwrap();

    assert!(matches!(
        tempo,
        Tempo::MetronomeMark {
            prelude: Some(SourceText::Synthesized(prelude)),
            bpm: 120,
            postlude: Some(SourceText::Synthesized(postlude)),
            ..
        } if prelude == "Allegro" && postlude == "brightly"
    ));
}

#[test]
fn section_10_1_retains_deprecated_tempo_without_interpreting_it() {
    for payload in [
        "C=120",
        "120",
        "999999999999999999999999999999999999999999999999",
    ] {
        let source = format!("Q:{payload}");
        let field = parse_field(&source).unwrap();
        assert!(matches!(
            field.value,
            FieldValue::Tempo(Tempo::Deprecated(ref text)) if text == payload
        ));
        assert_eq!(field.to_abc(), source);
    }

    assert!(parse_field("Q:C2=200").is_err());
}

#[test]
fn section_10_1_warns_for_deprecated_tempo_in_loose_and_strict_modes() {
    let source = "X:1\nT:Deprecated tempo\nQ:C=120\nK:C\n[Q:240] C |\n";
    for report in [
        parse(source),
        parse_with_options(source, ParserOptions::new().strict(true)),
    ] {
        assert!(report.is_valid(), "{:#?}", report.errors);
        assert_eq!(
            report
                .warnings
                .iter()
                .filter(|warning| warning.kind == ErrorKind::DeprecatedSyntax)
                .count(),
            2
        );

        let owned = report.output.unwrap().into_owned(source).unwrap();
        let mut deprecated = owned
            .tunes()
            .flat_map(|tune| &tune.lines)
            .flat_map(|line| match &line.value {
                Line::Field(field) => vec![&field.value],
                Line::Music(elements) => elements
                    .iter()
                    .filter_map(|element| match &element.value {
                        MusicElement::InlineField(field) => Some(&field.value),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            })
            .filter_map(|value| match value {
                FieldValue::Tempo(Tempo::Deprecated(text)) => Some(text.as_str()),
                _ => None,
            });
        assert_eq!(deprecated.next(), Some("C=120"));
        assert_eq!(deprecated.next(), Some("240"));
        assert_eq!(deprecated.next(), None);
    }
}

#[test]
fn section_10_1_warns_for_deprecated_a_and_e_fields() {
    assert_eq!(parse_field("A:Donegal").unwrap().kind, FieldKind::Area);
    assert_eq!(
        parse_field("E:1.2").unwrap().kind,
        FieldKind::ElementSpacing
    );

    let source = "X:1\nT:Deprecated fields\nA:Donegal\nE:1.2\nK:C\n";
    for report in [
        parse(source),
        parse_with_options(source, ParserOptions::new().strict(true)),
    ] {
        assert!(report.is_valid(), "{:#?}", report.errors);
        assert_eq!(
            report
                .warnings
                .iter()
                .filter(|warning| warning.kind == ErrorKind::DeprecatedSyntax)
                .count(),
            2
        );
    }
}

#[test]
fn section_10_1_retains_implicit_multiline_history_until_the_next_field() {
    let source =
        "H:first line\nCDEF | remains history\n%also history\n\nX:1\nT:History\nK:C\nCDEF |\n";
    for report in [
        parse(source),
        parse_with_options(source, ParserOptions::new().strict(true)),
    ] {
        assert!(report.is_valid(), "{:#?}", report.errors);
        assert_eq!(
            report
                .warnings
                .iter()
                .filter(|warning| warning.kind == ErrorKind::DeprecatedSyntax)
                .count(),
            1
        );
    }

    let report = parse(source);
    let document = report.output.unwrap().into_owned(source).unwrap();
    assert!(matches!(
        document.header.as_slice(),
        [
            abc_parser::Spanned {
                value: Line::Field(_),
                ..
            },
            abc_parser::Spanned {
                value: Line::DeprecatedHistoryContinuation(first),
                ..
            },
            abc_parser::Spanned {
                value: Line::DeprecatedHistoryContinuation(second),
                ..
            }
        ] if first == "CDEF | remains history" && second == "%also history"
    ));
    assert!(matches!(
        document.tunes().next().unwrap().lines.last().unwrap().value,
        Line::Music(_)
    ));
    assert!(
        document
            .to_abc()
            .starts_with("H:first line\nCDEF | remains history\n%also history\n\nX:1")
    );

    let current = parse("X:1\nT:History\nH:one line\nK:C\n");
    assert!(current.is_valid(), "{:#?}", current.errors);
    assert!(
        current
            .warnings
            .iter()
            .all(|warning| warning.kind != ErrorKind::DeprecatedSyntax)
    );
}

#[test]
fn sections_3_1_14_and_4_6_accept_key_and_clef_forms() {
    for source in [
        "K:C",
        "K:Am",
        "K:F#MIX",
        "K:none",
        "K:HP",
        "K:Hp",
        "K:D exp _b _e ^f",
        "K:clef=alto",
        "K:perc stafflines=1",
        "K:Am transpose=-2",
        "K:bass middle=d transpose=-24",
    ] {
        assert!(
            matches!(parse_field(source).unwrap().value, FieldValue::Key(_)),
            "{source}"
        );
    }
}

#[test]
fn section_3_2_accepts_standard_inline_fields_and_remarks() {
    let elements =
        assert_music_valid("[I:setbarnb 10][K:C#][L:1/16][M:9/8][P:B][Q:1/4=90][V:2][r:comment]C");
    assert_eq!(
        elements
            .iter()
            .filter(|element| matches!(element, MusicElement::InlineField(_)))
            .count(),
        8
    );
}

#[test]
fn sections_4_1_and_4_3_accept_octaves_and_all_length_shorthands() {
    let elements = assert_music_valid("C,', C' c C C1 C2 C3/2 C/ C// C/// C/128");
    let notes = elements
        .iter()
        .filter_map(|element| match element {
            MusicElement::Note(note) => Some(note),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(notes[0].pitch.class, PitchClass::C);
    assert_eq!(notes[0].pitch.octave, -1);
    assert_eq!(notes[1].pitch.octave, 1);
    assert_eq!(notes[2].pitch.octave, 1);
    assert_eq!(notes[10].length.denominator, 128);
}

#[test]
fn sections_4_4_4_5_and_4_12_accept_rhythm_rests_and_graces() {
    for source in [
        "A>B C<<D E<<<F",
        "z x/2 Z Z4 X X12",
        "{GdGe}A",
        "{/gagab}C",
        "{a3/2b/}C",
        "A<{g}A A{g}<A {a>b}c",
    ] {
        assert_music_valid(source);
    }
    let grace = assert_music_valid("{a>b}c");
    assert!(grace.iter().any(|element| matches!(
        element,
        MusicElement::Grace(group)
            if group
                .elements
                .iter()
                .any(|element| matches!(element, GraceElement::BrokenRhythm(_)))
    )));
}

#[test]
fn sections_4_8_through_4_10_accept_bars_and_endings_liberally() {
    for source in [
        "| |] || [| |: :| :: :|: :||: .| [|]",
        "|:: C ::| [|::: C |[|",
        "|[1 C :|[2 D |]",
        "|1 C :|2 D |]",
        "[1,3,5-7 C :|",
    ] {
        assert_music_valid(source);
    }
    assert!(!parse_music_line("[1, 3 C").is_valid());
}

#[test]
fn sections_4_11_through_4_20_cover_grouping_and_construct_order() {
    for source in [
        "(c (d e f) g a)",
        ".(cde.) C.-C",
        "(2ab (3abc (4:3:2 a2bc",
        "[CEGc] [d2f2][ce][df] [C2E2G2]3 [DD]",
        "\"Am7\"A \"C/E\"B \"G(Em)\"c \"^above\"d \"_below\"e \"<left\"f \"[>right\"g \"@free\"a",
        "\"Gm7\"v.=G,2- ~^c'3",
    ] {
        let elements = assert_music_valid(source);
        if source.contains("@free") {
            assert!(elements.iter().any(|element| matches!(
                element,
                MusicElement::Annotation(annotation)
                    if annotation.placement == AnnotationPlacement::Free
            )));
        }
    }
}

#[test]
fn sections_4_14_and_4_16_cover_decorations_and_symbols() {
    assert_music_valid(".A ~B HC LD ME OF PG SA TB uc vd !trill!e !D.C.!f");
    for source in ["U:T=!trill!", "U: p = \"^+\"", "s:\"^slow\" | !f! ** !fff!"] {
        assert!(parse_field(source).is_ok(), "{source}");
    }
    for source in ["!", "!da capo!", "!bad:name!", "!bad|name!"] {
        assert!(!parse_music_line(source).is_valid(), "{source}");
    }
}

#[test]
fn sections_5_7_and_11_accept_lyrics_voices_overlays_and_directives() {
    assert_document_valid(
        "%abc-2.1\nX:1\nT:Voices and words\nV:one name=\"First voice\" clef=treble\nV:two\nK:C\n[V:one] CDEF | (& G4 & c4 &)\nw:1.~These are lyr-ics |\nW:Unaligned words\ns:\"C\" * !>! *\n%%MIDI program 1\n",
    );
}

#[test]
fn section_8_1_ignores_reserved_forward_compatibility_characters() {
    let report = parse_music_line("@a !pp! #bc2/3* [K:C#] de?f y |**");
    assert!(report.is_valid(), "{:#?}", report.errors);
    assert_eq!(
        report
            .output
            .unwrap()
            .iter()
            .filter(|element| matches!(element.value, MusicElement::Extension(_)))
            .count(),
        6
    );
}

#[test]
fn negative_delimiters_and_structured_values_remain_diagnostics() {
    for source in [
        "[CEG", "{abc", "!trill", "\"Am", "[M:6/x]", "C/0", "-C", "C - C", ">C", "C<",
    ] {
        assert!(!parse_music_line(source).is_valid(), "{source}");
    }
    for source in ["M:6/0", "L:1/0", "X:-1", "V:", "P:@", "U:T"] {
        assert!(parse_field(source).is_err(), "{source}");
    }
    for source in ["P:(AB", "P:AB)", "U:A=!trill!", "U:x=!trill!"] {
        assert!(parse_field(source).is_err(), "{source}");
    }
}
