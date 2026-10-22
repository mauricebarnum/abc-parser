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

//! ABC music notation 2.1 parsing with source spans and error recovery.
//!
//! The parser keeps extension fields and directives losslessly while recognizing
//! the structure of standard music constructs.
//!
//! The companion [`abc-transpose` command](../abc_transpose/index.html)
//! transposes complete ABC files by destination key or chromatic interval.
//!
//! See [`architecture`] for parser flows and an overview of the public AST.

use std::fmt;

// Referenced only by the `#[cfg_attr(doc, aquamarine::aquamarine)]` attribute
// on the build.rs-generated `architecture` module. This no-op import
// satisfies Cargo's `unused_dependencies` lint; it has minimal compile-time
// impact and no runtime impact.
#[cfg(feature = "doc-mermaid")]
use aquamarine as _;
use chumsky::Parser;
use chumsky::input::ValueInput;
use chumsky::span::Span as ChumskySpan;

mod ast;
mod bar_duration;
pub mod build_info;
mod diagnostics;
mod emit;
mod parser;
mod source;

include!(concat!(env!("OUT_DIR"), "/architecture.rs"));

pub use ast::Accidental;
pub use ast::Annotation;
pub use ast::AnnotationPlacement;
pub use ast::BarKind;
pub use ast::BarLine;
pub use ast::BrokenRhythm;
pub use ast::Chord;
pub use ast::ChordMember;
pub use ast::Decoration;
pub use ast::Directive;
pub use ast::DirectiveKind;
pub use ast::Document;
pub use ast::DocumentItem;
pub use ast::EndingSelector;
pub use ast::Field;
pub use ast::FieldKind;
pub use ast::FieldParameter;
pub use ast::FieldValue;
pub use ast::Fraction;
pub use ast::FreeText;
pub use ast::GraceElement;
pub use ast::GraceGroup;
pub use ast::KeyAccidental;
pub use ast::KeySignature;
pub use ast::KeyTonic;
pub use ast::Line;
pub use ast::LineBreak;
pub use ast::MacroDefinition;
pub use ast::Meter;
pub use ast::MultiMeasureRest;
pub use ast::MusicElement;
pub use ast::Note;
pub use ast::NoteLength;
pub use ast::Overlay;
pub use ast::OwnedDocument;
pub use ast::ParsedDocument;
pub use ast::ParserOptions;
pub use ast::PartSequence;
pub use ast::PartToken;
pub use ast::Pitch;
pub use ast::PitchClass;
pub use ast::Rest;
pub use ast::RestKind;
pub use ast::Slur;
pub use ast::Span;
pub use ast::Spanned;
pub use ast::SymbolDefinition;
pub use ast::Tempo;
pub use ast::Tie;
pub use ast::Tune;
pub use ast::Tuplet;
pub use ast::TypesetText;
pub use ast::VariantEnding;
pub use ast::VoiceDefinition;
use ast::field_kind;
pub use bar_duration::BarDurationOptions;
pub use bar_duration::BarDurationPickupPolicy;
pub use bar_duration::bar_duration_warnings;
pub use chumsky::span::SimpleSpan;
pub use diagnostics::DiagnosticRenderer;
pub use diagnostics::ErrorKind;
pub use diagnostics::ParseError;
pub use diagnostics::ParseReport;
pub use diagnostics::ParseWarning;
pub use diagnostics::RelatedSpan;
use diagnostics::chumsky_error;
use diagnostics::chumsky_error_with_kind_and_offset;
use diagnostics::error;
use diagnostics::first_parser_error;
pub use emit::AbcEmitter;
pub use emit::EmitOptions;
pub use emit::NoteLengthStyle;
pub use emit::ToAbc;
pub use parser::Block;
pub use parser::BlocksContext;
pub use parser::chord_parser;
pub use parser::directive_parser;
pub use parser::field_parser;
pub use parser::line_parser;
pub use parser::music_element_parser;
pub use parser::music_line_parser;
pub use parser::note_length_parser;
pub use parser::parse_blocks;
pub use parser::version_marker;
pub use source::IntoOwnedAst;
pub use source::PlaceholderResolver;
pub use source::ResolveError;
pub use source::SOURCE_REFERENCE_PREFIX;
pub use source::SOURCE_REFERENCE_SUFFIX;
pub use source::SourceResolver;
pub use source::SourceText;
pub use source::is_source_reference_placeholder;

/// Parses a complete ABC document with default options and physical-line error recovery.
///
/// The returned AST uses the input's native spans and retains source-derived
/// text as [`SourceText`] values. Use [`IntoOwnedAst::into_owned`] with the
/// original input to create a standalone document.
///
/// # Examples
///
/// ```
/// use abc_parser::IntoOwnedAst;
/// use abc_parser::parse;
///
/// let source = "X:1\nT:Example\nK:C\nCDEF |\n";
/// let report = parse(source);
/// assert!(report.is_valid());
/// let document = report.output.unwrap().into_owned(source).unwrap();
/// assert_eq!(document.tunes().count(), 1);
/// ```
pub fn parse<'src, I>(input: I) -> ParseReport<ParsedDocument<I::Span>, I::Span>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord + fmt::Display,
{
    parse_with_options(input, ParserOptions::default())
}

/// Parses a complete ABC document using the supplied options.
///
/// Like [`parse`], this parser uses physical-line error recovery and returns
/// source-backed text with the input's native spans.
pub fn parse_with_options<'src, I>(
    input: I,
    options: ParserOptions,
) -> ParseReport<ParsedDocument<I::Span>, I::Span>
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
    <I::Span as ChumskySpan>::Context: PartialEq + fmt::Debug,
    <I::Span as ChumskySpan>::Offset: Ord + fmt::Display,
{
    let (result, diagnostics) = parser::parse_document(input, options);
    let (output, faults) = result.into_output_errors();
    let mut errors = faults.iter().map(chumsky_error).collect::<Vec<_>>();
    errors.extend(diagnostics.0);
    errors.sort_by_key(|error| (error.span.start(), error.span.end()));
    let mut warnings = diagnostics.1;
    warnings.sort_by_key(|warning| (warning.span.start(), warning.span.end()));
    ParseReport {
        output,
        errors,
        warnings,
    }
}

/// Parses one physical ABC line.
pub fn parse_line(source: &str) -> ParseReport<Line<SimpleSpan<usize>, String>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = line_parser().parse(source).into_output_errors();
    let output = output.and_then(|line| line.value.into_owned(source).ok());
    let errors = faults
        .iter()
        .map(|error| chumsky_error_with_kind_and_offset(error, ErrorKind::InvalidMusic, 0))
        .collect();
    ParseReport {
        output,
        errors,
        warnings: Vec::new(),
    }
}

/// Parses a `%%` directive line.
///
/// # Errors
/// Returns an error if the prefix or directive name is invalid.
pub fn parse_directive(source: &str) -> Result<Directive, ParseError> {
    let source = source.trim_end_matches(['\r', '\n']);
    directive_parser()
        .then_ignore(parser::trailing_comment())
        .parse(source)
        .into_result()
        .map_err(|errors| {
            first_parser_error(
                &errors,
                ErrorKind::InvalidDirective,
                "invalid directive",
                source.len(),
            )
        })?
        .into_owned(source)
        .map_err(|fault| {
            error(
                ErrorKind::InvalidDirective,
                &fault.to_string(),
                0,
                source.len(),
            )
        })
}

/// Parses an information field such as `K:C major`.
///
/// # Errors
/// Returns an error unless the input starts with one ASCII letter and `:`.
pub fn parse_field(source: &str) -> Result<Field, ParseError> {
    let source = source.trim_end_matches(['\r', '\n']);
    field_parser()
        .then_ignore(parser::trailing_comment())
        .parse(source)
        .into_result()
        .map_err(|errors| {
            first_parser_error(
                &errors,
                ErrorKind::InvalidField,
                "invalid field",
                source.len(),
            )
        })?
        .into_owned(source)
        .map_err(|fault| error(ErrorKind::InvalidField, &fault.to_string(), 0, source.len()))
}

/// Parses a complete bracketed chord.
///
/// ```
/// use abc_parser::parse_chord;
/// let chord = parse_chord("[^CEG]3/2").unwrap();
/// assert_eq!(chord.length.numerator, 3);
/// assert_eq!(chord.length.denominator, 2);
/// ```
///
/// # Errors
/// Returns an error for missing brackets, empty contents, or an invalid length.
pub fn parse_chord(source: &str) -> Result<Chord, ParseError> {
    chord_parser()
        .parse(source)
        .into_result()
        .map_err(|errors| {
            first_parser_error(
                &errors,
                ErrorKind::InvalidMusic,
                "invalid chord",
                source.len(),
            )
        })
}

/// Parses an ABC note-length suffix into its rational value.
///
/// An empty string evaluates to the default multiplier `1/1`.
///
/// # Examples
///
/// ```
/// use abc_parser::NoteLength;
/// use abc_parser::parse_note_length;
///
/// assert_eq!(parse_note_length("").unwrap(), NoteLength { numerator: 1, denominator: 1 });
/// assert_eq!(parse_note_length("/2").unwrap(), NoteLength { numerator: 1, denominator: 2 });
/// assert_eq!(parse_note_length("/").unwrap(), NoteLength { numerator: 1, denominator: 2 });
/// assert_eq!(parse_note_length("//").unwrap(), NoteLength { numerator: 1, denominator: 4 });
/// assert_eq!(parse_note_length("3/").unwrap(), NoteLength { numerator: 3, denominator: 2 });
/// assert_eq!(parse_note_length("3/2").unwrap(), NoteLength { numerator: 3, denominator: 2 });
/// ```
///
/// # Errors
///
/// Returns an error when the string is not a valid note-length suffix or has
/// an overflowing denominator.
pub fn parse_note_length(source: &str) -> Result<NoteLength, ParseError> {
    note_length_parser()
        .then_ignore(chumsky::primitive::end())
        .parse(source)
        .into_result()
        .map_err(|errors| {
            first_parser_error(
                &errors,
                ErrorKind::InvalidMusic,
                "invalid note length",
                source.len(),
            )
        })
}

/// Parses a line as music code, recovering after malformed elements.
pub fn parse_music_line(
    source: &str,
) -> ParseReport<Vec<Spanned<MusicElement<String>, SimpleSpan<usize>>>> {
    let source = source.trim_end_matches(['\r', '\n']);
    let (output, faults) = music_line_parser()
        .then_ignore(parser::trailing_comment())
        .parse(source)
        .into_output_errors();
    let output = output.map(|items| {
        items
            .into_iter()
            .filter_map(|item| item.into_owned(source).ok())
            .collect()
    });
    let errors = faults
        .iter()
        .map(|error| chumsky_error_with_kind_and_offset(error, ErrorKind::InvalidMusic, 0))
        .collect();
    ParseReport {
        output,
        errors,
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_single_music_element(source: &str) -> MusicElement {
        let report = parse_music_line(source);
        assert!(report.is_valid(), "{source}: {:#?}", report.errors);
        let mut elements = report.output.unwrap();
        assert_eq!(elements.len(), 1, "{source}: {elements:#?}");
        elements.remove(0).value
    }

    #[test]
    fn parses_public_partial_entries() {
        assert_eq!(parse_field("K:G mixolydian").unwrap().key, 'K');
        assert_eq!(parse_directive("%%staves (1 2)").unwrap().name, "staves");
        assert_eq!(
            parse_directive("%%textual value").unwrap().kind,
            DirectiveKind::Other
        );
        let voice_field = parse_field("V:1 name=\"Soprano\" snm=\"S\" clef=treble").unwrap();
        assert_eq!(voice_field.key, 'V');
        assert!(matches!(
            voice_field.value,
            FieldValue::Voice(ref voice)
                if voice.id == "1"
                    && voice.properties.len() == 3
                    && voice.properties[0].name.as_deref() == Some("name")
                    && voice.properties[0].value == "Soprano"
                    && voice.properties[1].name.as_deref() == Some("snm")
                    && voice.properties[1].value == "S"
                    && voice.properties[2].name.as_deref() == Some("clef")
                    && voice.properties[2].value == "treble"
        ));
        assert_eq!(parse_chord("[CEG]4").unwrap().length.numerator, 4);
        let music = parse_music_line("|: CDEF [CEG]2 :|");
        assert!(music.is_valid());
        let elements = music.output.unwrap();
        assert!(matches!(
            elements.first().map(|e| &e.value),
            Some(MusicElement::Bar(BarLine {
                kind: BarKind::RepeatStart,
                ..
            }))
        ));
        assert!(matches!(
            elements.last().map(|e| &e.value),
            Some(MusicElement::Bar(BarLine {
                kind: BarKind::RepeatEnd,
                ..
            }))
        ));
    }

    #[test]
    fn directive_parser_retains_body_while_scanning_once() {
        let directive = parse_directive("%%staves   (1 2)").unwrap();
        assert_eq!(directive.name, "staves");
        assert_eq!(directive.arguments, "(1 2)");
        assert_eq!(directive.body, "staves   (1 2)");
        assert_eq!(directive.kind, DirectiveKind::Other);

        let directive = parse_directive("%%text").unwrap();
        assert_eq!(directive.arguments, "");
        assert_eq!(directive.body, "text");
        assert_eq!(directive.kind, DirectiveKind::Text);

        let characters = "%%text  é".chars().collect::<Vec<_>>();
        let directive = directive_parser()
            .parse(characters.as_slice())
            .into_result()
            .unwrap();
        let SourceText::Span(body) = directive.body else {
            panic!("parsed directive bodies retain their source span");
        };
        assert_eq!(body.start..body.end, 2..characters.len());
    }

    #[test]
    fn music_elements_have_semantic_structure() {
        let report =
            parse_music_line("^/2c'3/2 z/ X4 |: [1,3-5 (3:2:3 !trill! \"^text\" C>>D.-D &");
        assert!(report.is_valid(), "{:#?}", report.errors);
        let output = report.output.as_ref().unwrap();
        assert!(matches!(
            output[0].value,
            MusicElement::Note(Note {
                pitch: Pitch {
                    class: PitchClass::C,
                    octave: 2,
                    accidental: Some(Accidental::Sharp(Fraction {
                        numerator: 1,
                        denominator: 2
                    }))
                },
                length: NoteLength {
                    numerator: 3,
                    denominator: 2
                }
            })
        ));
        assert!(output.iter().any(|item| matches!(
            item.value,
            MusicElement::Bar(BarLine {
                kind: BarKind::RepeatStart,
                ..
            })
        )));
        assert!(output.iter().any(|item| matches!(
            item.value,
            MusicElement::Tuplet(Tuplet {
                actual: 3,
                normal: Some(2),
                affected: Some(3)
            })
        )));
        assert!(
            output
                .iter()
                .any(|item| matches!(item.value, MusicElement::BrokenRhythm(_)))
        );
    }

    #[test]
    fn adjacent_bars_abbreviate_first_and_second_endings() {
        let source = "|: a |[1 b :|2 c";
        let report = parse_music_line(source);
        assert!(report.is_valid(), "{:#?}", report.errors);
        let output = report.output.as_ref().unwrap();
        let endings = output
            .iter()
            .filter(|element| matches!(element.value, MusicElement::Ending(_)))
            .collect::<Vec<_>>();
        assert_eq!(endings.len(), 2);
        assert_eq!(endings[0].span, SimpleSpan::from(6..8));
        assert!(matches!(
            endings[0].value,
            MusicElement::Ending(VariantEnding { ref selectors, .. })
                if selectors == &[EndingSelector::Number(1)]
        ));
        assert_eq!(endings[1].span, SimpleSpan::from(13..14));
        assert!(matches!(
            endings[1].value,
            MusicElement::Ending(VariantEnding { ref selectors, .. })
                if selectors == &[EndingSelector::Number(2)]
        ));

        let document = parse("X:1\n|: a |1 b :|2 c\n");
        assert!(document.is_valid(), "{:#?}", document.errors);
    }

    #[test]
    fn malformed_abbreviated_endings_report_targeted_diagnostics() {
        let separated = parse_music_line("|: a |[1 b :| 2 c");
        assert_eq!(separated.errors.len(), 1, "{:#?}", separated.errors);
        assert_eq!(separated.errors[0].span, 14..15);
        assert_eq!(
            separated.errors[0].message,
            "variant ending 2 must be adjacent to the bar line or begin with '['"
        );
        let output = separated.output.as_ref().unwrap();
        assert!(output.iter().any(|element| {
            matches!(&element.value, MusicElement::Extension(value) if value == "2")
        }));
        assert!(matches!(
            output.last().map(|element| &element.value),
            Some(MusicElement::Note(Note {
                pitch: Pitch {
                    class: PitchClass::C,
                    ..
                },
                ..
            }))
        ));

        for source in ["|3", "|12"] {
            let report = parse_music_line(source);
            assert_eq!(report.errors.len(), 1, "{source}: {:#?}", report.errors);
            assert!(
                report.errors[0]
                    .message
                    .contains("only endings 1 and 2 may use the abbreviated form"),
                "{source}: {:#?}",
                report.errors
            );
        }
    }

    #[test]
    fn variant_ending_recovery_does_not_change_other_number_syntax() {
        for source in ["C2", "(2cd", ":|[2 c", ":| [2 c", "[1,3-5 c"] {
            let report = parse_music_line(source);
            assert!(report.is_valid(), "{source}: {:#?}", report.errors);
        }

        let standalone = parse_music_line("2 c");
        assert!(!standalone.is_valid());
        assert!(
            standalone
                .errors
                .iter()
                .all(|error| !error.message.contains("variant ending")),
            "{:#?}",
            standalone.errors
        );
    }

    #[test]
    fn common_music_tokens_preserve_prioritized_choice_semantics() {
        assert!(matches!(
            parse_single_music_element("C"),
            MusicElement::Note(_)
        ));
        assert!(matches!(
            parse_single_music_element(" "),
            MusicElement::BeamBreak(_)
        ));
        assert!(matches!(
            parse_single_music_element("z"),
            MusicElement::Rest(_)
        ));
        assert!(matches!(
            parse_single_music_element("Z"),
            MusicElement::MultiMeasureRest(_)
        ));
        assert!(matches!(
            parse_single_music_element("$"),
            MusicElement::LineBreak(_)
        ));
        assert!(matches!(
            parse_single_music_element("|"),
            MusicElement::Bar(_)
        ));
    }

    #[test]
    fn marker_spellings_map_to_their_documented_semantics() {
        let accidental_cases = [
            ("=C", Accidental::Natural),
            (
                "^C",
                Accidental::Sharp(Fraction {
                    numerator: 1,
                    denominator: 1,
                }),
            ),
            (
                "_C",
                Accidental::Flat(Fraction {
                    numerator: 1,
                    denominator: 1,
                }),
            ),
        ];
        for (source, expected) in accidental_cases {
            assert!(matches!(
                parse_single_music_element(source),
                MusicElement::Note(Note {
                    pitch: Pitch {
                        accidental: Some(actual),
                        ..
                    },
                    ..
                }) if actual == expected
            ));
        }

        for (source, expected) in [("z", RestKind::Visible), ("x", RestKind::Invisible)] {
            assert!(matches!(
                parse_single_music_element(source),
                MusicElement::Rest(Rest { kind, .. }) if kind == expected
            ));
        }
        for (source, expected) in [("Z", false), ("X", true)] {
            assert!(matches!(
                parse_single_music_element(source),
                MusicElement::MultiMeasureRest(MultiMeasureRest { invisible, .. })
                    if invisible == expected
            ));
        }

        for (source, expected) in [
            ("\"text\"", AnnotationPlacement::ChordSymbol),
            ("\"^text\"", AnnotationPlacement::Above),
            ("\"_text\"", AnnotationPlacement::Below),
            ("\"<text\"", AnnotationPlacement::Left),
            ("\">text\"", AnnotationPlacement::Right),
            ("\"@text\"", AnnotationPlacement::Free),
        ] {
            assert!(matches!(
                parse_single_music_element(source),
                MusicElement::Annotation(Annotation { placement, .. }) if placement == expected
            ));
        }

        for (source, expected_name, expected_legacy) in
            [("!turn!", "turn", false), ("+turn+", "turn", true)]
        {
            assert!(matches!(
                parse_single_music_element(source),
                MusicElement::Decoration(Decoration {
                    name,
                    legacy_delimiter,
                }) if name == expected_name && legacy_delimiter == expected_legacy
            ));
        }

        for (symbol, expected_name) in [
            ('.', "staccato"),
            ('~', "roll"),
            ('H', "fermata"),
            ('L', "accent"),
            ('M', "lowermordent"),
            ('O', "coda"),
            ('P', "uppermordent"),
            ('S', "segno"),
            ('T', "trill"),
            ('u', "upbow"),
            ('v', "downbow"),
        ] {
            assert!(matches!(
                parse_single_music_element(&symbol.to_string()),
                MusicElement::Decoration(Decoration {
                    name,
                    legacy_delimiter: false,
                }) if name == expected_name
            ));
        }
    }

    #[test]
    fn pitch_letters_map_to_their_class_and_base_octave() {
        for (letter, expected_class) in [
            ('A', PitchClass::A),
            ('B', PitchClass::B),
            ('C', PitchClass::C),
            ('D', PitchClass::D),
            ('E', PitchClass::E),
            ('F', PitchClass::F),
            ('G', PitchClass::G),
        ] {
            for (spelling, expected_octave) in [(letter, 0), (letter.to_ascii_lowercase(), 1)] {
                assert!(matches!(
                    parse_single_music_element(&spelling.to_string()),
                    MusicElement::Note(Note {
                        pitch: Pitch { class, octave, .. },
                        ..
                    }) if class == expected_class && octave == expected_octave
                ));
            }
        }

        for (source, expected) in [
            ("K:C#", KeyAccidental::Sharp),
            ("K:Cb", KeyAccidental::Flat),
        ] {
            assert!(matches!(
                parse_field(source).unwrap().value,
                FieldValue::Key(KeySignature {
                    tonic: Some(KeyTonic {
                        accidental: Some(actual),
                        ..
                    }),
                    ..
                }) if actual == expected
            ));
        }
    }

    #[test]
    fn tuplet_defaults_resolve_from_ratio_and_meter() {
        let compact = |actual_notes| Tuplet {
            actual: actual_notes,
            normal: None,
            affected: None,
        };
        for (actual_notes, normal_notes) in [
            (2, 3),
            (3, 2),
            (4, 3),
            (5, 2),
            (6, 2),
            (7, 2),
            (8, 3),
            (9, 2),
        ] {
            let tuplet = compact(actual_notes);
            assert_eq!(tuplet.normal_note_count(false), Some(normal_notes));
            assert_eq!(tuplet.affected_note_count(), actual_notes);
        }
        for actual_notes in [5, 7, 9] {
            assert_eq!(compact(actual_notes).normal_note_count(true), Some(3));
        }

        let explicit = Tuplet {
            actual: 10,
            normal: Some(7),
            affected: Some(4),
        };
        assert_eq!(explicit.normal_note_count(false), Some(7));
        assert_eq!(explicit.normal_note_count(true), Some(7));
        assert_eq!(explicit.affected_note_count(), 4);
        assert_eq!(compact(10).normal_note_count(false), None);
    }

    #[test]
    fn structured_fields_are_parsed_and_recover_losslessly() {
        assert_eq!(
            parse_field("L:1/16").unwrap().value,
            FieldValue::UnitLength(Fraction {
                numerator: 1,
                denominator: 16
            })
        );
        assert_eq!(
            parse_field("L: 1/16").unwrap().value,
            FieldValue::UnitLength(Fraction {
                numerator: 1,
                denominator: 16
            })
        );
        assert!(matches!(
            parse_field("M:(2+3)/8").unwrap().value,
            FieldValue::Meter(Meter::Compound {
                groups,
                denominator: 8
            }) if groups == [2, 3]
        ));
        assert_eq!(
            parse_field("M:3/4").unwrap().value,
            FieldValue::Meter(Meter::Simple(Fraction {
                numerator: 3,
                denominator: 4
            }))
        );
        assert_eq!(
            parse_field("M: 3/4").unwrap().value,
            FieldValue::Meter(Meter::Simple(Fraction {
                numerator: 3,
                denominator: 4
            }))
        );
        assert!(matches!(
            parse_field("K:G mixolydian clef=bass").unwrap().value,
            FieldValue::Key(KeySignature {
                tonic: Some(KeyTonic {
                    class: PitchClass::G,
                    ..
                }),
                ..
            })
        ));

        assert!(parse_field("L:not-a-length").is_err());
        let report = parse_line("L:  not-a-length  ");
        assert_eq!(report.errors.len(), 1);
        assert!(matches!(
            report.output,
            Some(Line::Field(Field {
                value: FieldValue::Unparsed(ref value),
                ..
            })) if value == "not-a-length"
        ));
    }

    #[test]
    fn malformed_structured_fields_report_expected_syntax() {
        for (source, context) in [
            ("L:1/x", "L: unit note length"),
            ("M:6/x", "M: meter"),
            ("Q:1/4=x", "Q: tempo"),
            ("K:?", "K: key signature"),
            ("X:nope", "X: reference number"),
            ("V:", "V: voice definition"),
            ("P:@", "P: part sequence"),
            ("U:symbol", "U: user symbol definition"),
            ("m:pattern", "m: macro definition"),
        ] {
            let report = parse_line(source);
            assert!(!report.is_valid(), "{source}");
            assert!(
                report.errors.iter().any(|error| {
                    error.message.contains("found") && error.message.contains(context)
                }),
                "{source}: {:#?}",
                report.errors
            );
            assert!(
                matches!(
                    report.output,
                    Some(Line::Field(Field {
                        value: FieldValue::Unparsed(_),
                        ..
                    }))
                ),
                "{source}: {:#?}",
                report.output
            );
        }
    }

    #[test]
    fn malformed_music_reports_expected_syntax_and_recovers() {
        for (source, expected) in [
            ("[CEG", "chord"),
            ("{C", "grace group"),
            ("\"Am", "annotation"),
            ("!trill", "decoration"),
            ("(999", "tuplet"),
            ("[M:6/x]", "inline M: meter"),
            ("}", "music element"),
        ] {
            let report = parse_music_line(source);
            assert!(!report.is_valid(), "{source}");
            assert!(
                report
                    .errors
                    .iter()
                    .any(|error| error.message.contains(expected)),
                "{source}: {:#?}",
                report.errors
            );
        }

        let document = parse("X:1\nK:C\n[CEG\nCDEF |\n");
        assert_eq!(document.errors.len(), 1);
        assert!(
            document
                .errors
                .iter()
                .any(|error| { error.message.contains("']'") && error.message.contains("chord") }),
            "{:#?}",
            document.errors
        );
        let tune = document.output.as_ref().unwrap().tunes().next().unwrap();
        assert_eq!(tune.lines.len(), 4);
        assert!(matches!(tune.lines[3].value, Line::Music(_)));
    }

    #[test]
    fn malformed_directives_report_expected_syntax() {
        let error = parse_directive("%%").unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidDirective);
        assert!(
            error.message.contains("stylesheet directive name"),
            "{error}"
        );

        let report = parse_line("%%");
        assert!(!report.is_valid());
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.message.contains("stylesheet directive name")),
            "{:#?}",
            report.errors
        );
    }

    #[test]
    fn partial_parser_errors_use_construct_specific_kinds() {
        assert_eq!(
            parse_field("L:not-a-length").unwrap_err().kind,
            ErrorKind::InvalidField
        );
        assert_eq!(
            parse_field("Key:C").unwrap_err().kind,
            ErrorKind::InvalidField
        );
        assert_eq!(
            parse_directive("%% bad").unwrap_err().kind,
            ErrorKind::InvalidDirective
        );
        assert_eq!(
            parse_chord("[CEG").unwrap_err().kind,
            ErrorKind::InvalidMusic
        );
    }

    #[test]
    fn every_error_span_is_in_bounds() {
        let original = "X:1\nK:C\nCDEF GABc |\n";
        for index in 0..original.len() {
            let mut mutated = original.to_owned();
            mutated.replace_range(index..=index, "@");
            let report = parse(mutated.as_str());
            assert!(
                report
                    .errors
                    .iter()
                    .all(|fault| fault.span.end <= mutated.len())
            );
        }
    }
}
