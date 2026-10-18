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

//! Note, rest, chord, grace, bar, decoration, and music-line combinators.

use chumsky::IterParser;
use chumsky::Parser;
use chumsky::error::Rich;
use chumsky::input::ValueInput;
use chumsky::prelude::any;
use chumsky::prelude::choice;
use chumsky::prelude::empty;
use chumsky::prelude::end;
use chumsky::prelude::just;
use chumsky::prelude::one_of;
use chumsky::prelude::select;
use chumsky::recovery::via_parser;

use super::Extra;
use super::ParsedMusic;
use super::fields::inline_field;
use super::primitives::line_character;
use super::primitives::note_length_parser;
use super::primitives::pitch_letter;
use super::primitives::unsigned;
use crate::Accidental;
use crate::Annotation;
use crate::AnnotationPlacement;
use crate::BarKind;
use crate::BarLine;
use crate::BrokenRhythm;
use crate::Chord;
use crate::ChordMember;
use crate::Decoration;
use crate::EndingSelector;
use crate::Fraction;
use crate::GraceElement;
use crate::GraceGroup;
use crate::Line;
use crate::LineBreak;
use crate::MultiMeasureRest;
use crate::MusicElement;
use crate::Note;
use crate::Overlay;
use crate::Pitch;
use crate::Rest;
use crate::RestKind;
use crate::Slur;
use crate::SourceText;
use crate::Spanned;
use crate::Tie;
use crate::Tuplet;
use crate::VariantEnding;

/// Parses an accidental amount following one repeated marker.
pub(crate) fn accidental_amount<'src, I>(
    marker: char,
) -> impl Parser<'src, I, Fraction, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just(marker)
        .repeated()
        .at_least(1)
        .count()
        .then(unsigned().or_not())
        .then(just('/').ignore_then(unsigned().or_not()).or_not())
        .try_map(|((markers, numerator), denominator), span| {
            let denominator = denominator.flatten().unwrap_or(1);
            if denominator == 0 {
                return Err(Rich::custom(
                    span,
                    "accidental denominator must not be zero",
                ));
            }
            let amount = Fraction {
                numerator: numerator.unwrap_or_else(|| u32::try_from(markers).unwrap_or(u32::MAX)),
                denominator,
            };
            Ok(amount)
        })
}

/// Parses a complete written accidental.
pub(crate) fn accidental<'src, I>() -> impl Parser<'src, I, Accidental, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just('=').to(Accidental::Natural),
        accidental_amount('^').map(Accidental::Sharp),
        accidental_amount('_').map(Accidental::Flat),
    ))
    .labelled("accidental")
    .as_context()
}

/// Parses apostrophes as positive and commas as negative octave modifiers.
pub(crate) fn octave_modifier<'src, I>() -> impl Parser<'src, I, i8, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    empty()
        .to(0_i8)
        .foldl(one_of("',").repeated(), |octave, marker| {
            if marker == '\'' {
                octave.saturating_add(1)
            } else {
                octave.saturating_sub(1)
            }
        })
}

/// Parses a pitched note directly into semantic pitch and duration values.
pub(crate) fn note<'src, I>() -> impl Parser<'src, I, Note, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    accidental()
        .or_not()
        .then(pitch_letter())
        .then(octave_modifier())
        .then(note_length_parser())
        .map(|(((accidental, letter), octave_modifier), length)| Note {
            pitch: Pitch {
                class: letter.class,
                octave: letter.octave_offset.saturating_add(octave_modifier),
                accidental,
            },
            length,
        })
        .labelled("note")
        .as_context()
}

/// Parses a visible or invisible single rest.
pub(crate) fn rest<'src, I>() -> impl Parser<'src, I, Rest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just('z').to(RestKind::Visible),
        just('x').to(RestKind::Invisible),
    ))
    .then(note_length_parser())
    .map(|(kind, length)| Rest { kind, length })
    .labelled("rest")
    .as_context()
}

/// Parses a multi-measure rest, defaulting its measure count to one.
pub(crate) fn multi_measure_rest<'src, I>()
-> impl Parser<'src, I, MultiMeasureRest, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((just('Z').to(false), just('X').to(true)))
        .then(unsigned().or_not())
        .map(|(invisible, measures)| MultiMeasureRest {
            invisible,
            measures: measures.unwrap_or(1),
        })
        .labelled("multi-measure rest")
        .as_context()
}

/// Parses one bracketed chord directly from member and duration parsers.
pub fn chord_parser<'src, I>() -> impl Parser<'src, I, Chord, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((note().map(ChordMember::Note), rest().map(ChordMember::Rest)))
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .delimited_by(just('['), just(']'))
        .then(note_length_parser())
        .map(|(members, length)| Chord { members, length })
        .labelled("chord")
        .as_context()
}

/// Parses a variant-ending selector list such as [1,3-5.
pub(crate) fn variant_ending<'src, I>()
-> impl Parser<'src, I, VariantEnding, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let selector = unsigned()
        .then(just('-').ignore_then(unsigned()).or_not())
        .map(|(start, end)| {
            end.map_or(EndingSelector::Number(start), |end| EndingSelector::Range {
                start,
                end,
            })
        });
    just('[')
        .ignore_then(
            selector
                .separated_by(just(','))
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map(|selectors| VariantEnding {
            explicit_bracket: true,
            selectors,
        })
        .labelled("variant ending")
        .as_context()
}

/// Parses standard bar spellings first, then accepts liberal bar sequences.
pub(crate) fn bar<'src, I>()
-> impl Parser<'src, I, BarLine<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    choice((
        just("[|]").to(BarKind::Invisible),
        just(":||:").to(BarKind::RepeatBoth),
        just(":|:").to(BarKind::RepeatBoth),
        just("::").to(BarKind::RepeatBoth),
        just("|:").to(BarKind::RepeatStart),
        just(":|").to(BarKind::RepeatEnd),
        just("|]").to(BarKind::ThinThick),
        just("[|").to(BarKind::ThickThin),
        just("||").to(BarKind::Double),
        just(".|").to(BarKind::Dotted),
        just('|').to(BarKind::Single),
        one_of("|:")
            .repeated()
            .at_least(1)
            .ignored()
            .to(BarKind::Other),
    ))
    .map_with(|kind, extra| BarLine {
        kind,
        source: SourceText::Span(extra.span()),
    })
    .labelled("bar line")
}

/// Parses an abbreviated variant ending together with its preceding bar line.
pub(crate) fn bar_variant_ending<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let bar = bar().map_with(|value, extra| Spanned {
        value: MusicElement::Bar(value),
        span: extra.span(),
    });
    let digits = any()
        .filter(char::is_ascii_digit)
        .repeated()
        .at_least(1)
        .collect::<String>()
        .map_with(|digits, extra| (digits, extra.span()));
    let adjacent = bar
        .clone()
        .then(digits)
        .validate(|(bar, (digits, span)): (_, (String, I::Span)), _, emitter| {
            let value = match digits.as_str() {
                "1" => MusicElement::Ending(VariantEnding {
                    explicit_bracket: false,
                    selectors: vec![EndingSelector::Number(1)],
                }),
                "2" => MusicElement::Ending(VariantEnding {
                    explicit_bracket: false,
                    selectors: vec![EndingSelector::Number(2)],
                }),
                _ => {
                    emitter.emit(Rich::custom(
                        span.clone(),
                        format!(
                            "variant ending {digits} must begin with '['; only endings 1 and 2 may use the abbreviated form"
                        ),
                    ));
                    MusicElement::Extension(SourceText::Span(span.clone()))
                }
            };
            vec![bar, Spanned { value, span }]
        });
    let beam_break = one_of(" \t")
        .repeated()
        .at_least(1)
        .to_span()
        .map(|span: I::Span| Spanned {
            value: MusicElement::BeamBreak(SourceText::Span(span.clone())),
            span,
        });
    let separated = bar.then(beam_break).then(digits).validate(
        move |((bar, beam_break), (digits, span)): ((_, _), (String, I::Span)), _, emitter| {
            emitter.emit(Rich::custom(
                span.clone(),
                format!(
                    "variant ending {digits} must be adjacent to the bar line or begin with '['"
                ),
            ));
            vec![
                bar,
                beam_break,
                Spanned {
                    value: MusicElement::Extension(SourceText::Span(span.clone())),
                    span,
                },
            ]
        },
    );
    choice((adjacent, separated))
}

/// Parses a grace group, including the optional acciaccatura slash.
pub(crate) fn grace<'src, I>() -> impl Parser<'src, I, GraceGroup, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let item = choice((
        note().map(GraceElement::Note),
        broken_rhythm().map(GraceElement::BrokenRhythm),
    ));
    just('/')
        .or_not()
        .then(item.repeated().at_least(1).collect::<Vec<_>>())
        .delimited_by(just('{'), just('}'))
        .validate(|(slash, elements), extra, emitter| {
            if !elements
                .iter()
                .any(|element| matches!(element, GraceElement::Note(_)))
            {
                emitter.emit(Rich::custom(
                    extra.span(),
                    "grace group must contain a note",
                ));
            }
            GraceGroup {
                acciaccatura: slash.is_some(),
                elements,
            }
        })
        .labelled("grace group")
        .as_context()
}

/// Parses named, legacy, and shorthand decorations.
pub(crate) fn decoration<'src, I>()
-> impl Parser<'src, I, Decoration<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let named = |delimiter| {
        just(delimiter)
            .ignore_then(
                any()
                    .filter(move |character: &char| {
                        *character != delimiter
                            && !character.is_whitespace()
                            && !matches!(character, '[' | ']' | '|' | ':' | '\r' | '\n')
                    })
                    .repeated()
                    .at_least(1)
                    .to_span()
                    .map(SourceText::Span),
            )
            .then_ignore(just(delimiter))
    };
    let shorthand_name = select! {
        '.' => "staccato",
        '~' => "roll",
        'H' => "fermata",
        'L' => "accent",
        'M' => "lowermordent",
        'O' => "coda",
        'P' => "uppermordent",
        'S' => "segno",
        'T' => "trill",
        'u' => "upbow",
        'v' => "downbow",
    };
    let shorthand = shorthand_name.map(|name| Decoration {
        name: SourceText::Synthesized(name.into()),
        legacy_delimiter: false,
    });
    choice((
        named('!').map(|name| Decoration {
            name,
            legacy_delimiter: false,
        }),
        named('+').map(|name| Decoration {
            name,
            legacy_delimiter: true,
        }),
        shorthand,
    ))
    .labelled("decoration")
    .as_context()
}

/// Parses a quoted chord symbol or positioned annotation.
pub(crate) fn annotation<'src, I>()
-> impl Parser<'src, I, Annotation<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let placement = choice((
        just('^').to(AnnotationPlacement::Above),
        just('_').to(AnnotationPlacement::Below),
        just('<').to(AnnotationPlacement::Left),
        just('>').to(AnnotationPlacement::Right),
        just('@').to(AnnotationPlacement::Free),
        empty().to(AnnotationPlacement::ChordSymbol),
    ));
    just('"')
        .ignore_then(placement)
        .then(
            any()
                .filter(|character| !matches!(character, '"' | '\r' | '\n'))
                .repeated()
                .to_span()
                .map(SourceText::Span),
        )
        .then_ignore(just('"'))
        .map(|(placement, text)| Annotation { placement, text })
        .labelled("annotation")
        .as_context()
}

/// Parses compact and extended tuplet prefixes.
pub(crate) fn tuplet<'src, I>() -> impl Parser<'src, I, Tuplet, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    just('(')
        .ignore_then(unsigned())
        .then(just(':').ignore_then(unsigned().or_not()).or_not())
        .then(just(':').ignore_then(unsigned().or_not()).or_not())
        .validate(
            |((actual_notes, normal_notes), affected_notes), extra, emitter| {
                let mut convert = |value| {
                    if let Ok(value) = u8::try_from(value) {
                        value
                    } else {
                        emitter.emit(Rich::custom(extra.span(), "tuplet value exceeds u8"));
                        u8::MAX
                    }
                };
                Tuplet {
                    actual: convert(actual_notes),
                    normal: normal_notes.flatten().map(&mut convert),
                    affected: affected_notes.flatten().map(convert),
                }
            },
        )
        .labelled("tuplet")
        .as_context()
}

/// Parses repeated broken-rhythm operators while retaining direction.
pub(crate) fn broken_rhythm<'src, I>() -> impl Parser<'src, I, BrokenRhythm, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
{
    choice((
        just('>')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| BrokenRhythm {
                greater: true,
                count: u8::try_from(count).unwrap_or(u8::MAX),
            }),
        just('<')
            .repeated()
            .at_least(1)
            .count()
            .map(|count| BrokenRhythm {
                greater: false,
                count: u8::try_from(count).unwrap_or(u8::MAX),
            }),
    ))
    .labelled("broken-rhythm marker")
}

/// Parses one recognized music-code element without an extension fallback.
pub(crate) fn semantic_music_element_parser<'src, I>()
-> impl Parser<'src, I, MusicElement<SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let beam_break = one_of(" \t")
        .repeated()
        .at_least(1)
        .to_span()
        .map(SourceText::Span)
        .map(MusicElement::BeamBreak);
    let line_break = choice((
        just('\\').to(LineBreak::Continue),
        just("$$").to(LineBreak::Paragraph),
        just('$').to(LineBreak::Break),
        just('y').to(LineBreak::Space),
    ))
    .map(MusicElement::LineBreak);
    let plain_open_slur = just('(')
        .then_ignore(
            any()
                .filter(|character: &char| !character.is_ascii_digit())
                .rewind()
                .ignored()
                .or(end()),
        )
        .to(MusicElement::Slur(Slur {
            opening: true,
            dotted: false,
        }));
    choice((
        note().map(MusicElement::Note),
        beam_break,
        multi_measure_rest().map(MusicElement::MultiMeasureRest),
        rest().map(MusicElement::Rest),
        line_break,
        inline_field().map(MusicElement::InlineField),
        chord_parser().map(MusicElement::Chord),
        variant_ending().map(MusicElement::Ending),
        bar().map(MusicElement::Bar),
        grace().map(MusicElement::Grace),
        annotation().map(MusicElement::Annotation),
        just(".-").to(MusicElement::Tie(Tie { dotted: true })),
        decoration().map(MusicElement::Decoration),
        tuplet().map(MusicElement::Tuplet),
        just("(&").to(MusicElement::Overlay(Overlay::Start)),
        just("&)").to(MusicElement::Overlay(Overlay::End)),
        just('&').to(MusicElement::Overlay(Overlay::NextVoice)),
        just(".(").to(MusicElement::Slur(Slur {
            opening: true,
            dotted: true,
        })),
        just(".)").to(MusicElement::Slur(Slur {
            opening: false,
            dotted: true,
        })),
        plain_open_slur,
        just(')').to(MusicElement::Slur(Slur {
            opening: false,
            dotted: false,
        })),
        just('-').to(MusicElement::Tie(Tie { dotted: false })),
        broken_rhythm().map(MusicElement::BrokenRhythm),
        just('`')
            .repeated()
            .at_least(1)
            .count()
            .map(MusicElement::BeamContinuation),
        one_of("#*;?@")
            .map_with(|_, extra| MusicElement::Extension(SourceText::Span(extra.span()))),
    ))
    .labelled("music element")
    .as_context()
}

/// Parses one music element while preserving the strict parser's failure.
pub(crate) fn diagnostic_music_element_parser<'src, I>()
-> impl Parser<'src, I, Spanned<MusicElement<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let fallback = line_character()
        .filter(|character| *character != '%')
        .map_with(|_, extra| MusicElement::Extension(SourceText::Span(extra.span())));
    semantic_music_element_parser()
        .recover_with(via_parser(fallback))
        .map_with(|value, extra| Spanned {
            value,
            span: extra.span(),
        })
}

/// Parses a complete semantic music sequence, including abbreviated endings.
pub(crate) fn semantic_music_sequence_parser<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let element = semantic_music_element_parser()
        .map_with(|value, extra| Spanned {
            value,
            span: extra.span(),
        })
        .map(|element| vec![element]);
    choice((bar_variant_ending(), element))
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|chunks| chunks.into_iter().flatten().collect())
}

/// Parses a recovering music sequence, including abbreviated endings.
pub(crate) fn diagnostic_music_sequence_parser<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    let element = diagnostic_music_element_parser().map(|element| vec![element]);
    choice((bar_variant_ending(), element))
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|chunks| chunks.into_iter().flatten().collect())
}

/// Builds a validating parser for one semantic music-code element.
pub fn music_element_parser<'src, I>()
-> impl Parser<'src, I, Spanned<MusicElement<SourceText<I::Span>>, I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    diagnostic_music_element_parser::<I>()
}

/// Builds a recovering music-line parser with sequence-level validation.
pub(crate) fn recovering_music_line_parser<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    diagnostic_music_sequence_parser::<I>().validate(|elements, _, emitter| {
        for (index, element) in elements.iter().enumerate() {
            match element.value {
                MusicElement::Tie(_) => {
                    let attached = index.checked_sub(1).is_some_and(|previous| {
                        matches!(
                            elements[previous].value,
                            MusicElement::Note(_) | MusicElement::Chord(_)
                        )
                    });
                    if !attached {
                        emitter.emit(Rich::custom(
                            element.span.clone(),
                            "tie must be adjacent to the preceding note or chord",
                        ));
                    }
                }
                MusicElement::BrokenRhythm(_) => {
                    let is_timed_group = |candidate: &Spanned<_, _>| {
                        matches!(
                            candidate.value,
                            MusicElement::Note(_) | MusicElement::Chord(_) | MusicElement::Rest(_)
                        )
                    };
                    let is_transparent = |candidate: &Spanned<_, _>| {
                        matches!(
                            candidate.value,
                            MusicElement::Grace(_)
                                | MusicElement::BeamBreak(_)
                                | MusicElement::BeamContinuation(_)
                        )
                    };
                    let previous = elements[..index]
                        .iter()
                        .rev()
                        .find(|candidate| !is_transparent(candidate))
                        .is_some_and(is_timed_group);
                    let next = elements[index + 1..]
                        .iter()
                        .find(|candidate| !is_transparent(candidate))
                        .is_some_and(is_timed_group);
                    if !previous || !next {
                        emitter.emit(Rich::custom(
                            element.span.clone(),
                            "broken-rhythm marker must occur between timed note groups",
                        ));
                    }
                }
                _ => {}
            }
        }
        elements
    })
}

/// Builds a parser for a complete physical music-code line.
pub fn music_line_parser<'src, I>()
-> impl Parser<'src, I, ParsedMusic<I::Span>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    recovering_music_line_parser()
}

/// Parses a complete music line without recovery for advisory classification.
pub(crate) fn strict_music_line_parser<'src, I>()
-> impl Parser<'src, I, Line<I::Span, SourceText<I::Span>>, Extra<'src, I>> + Clone
where
    I: ValueInput<'src, Token = char>,
    I::Span: Clone,
{
    semantic_music_sequence_parser().map(Line::Music)
}
