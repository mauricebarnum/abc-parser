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

//! Measure segmentation, barline parsing, variant ending tracking, and voice measure merging.

use abc_parser::BarKind;
use abc_parser::BarLine;
use abc_parser::EndingSelector;
use abc_parser::LineBreak;
use abc_parser::MusicElement;
use abc_parser::SimpleSpan;
use abc_parser::VariantEnding;

use crate::lower::types::EndingType;
use crate::lower::types::LoweredBarline;
use crate::lower::types::LoweredEnding;
use crate::lower::types::RawMeasureItem;
use crate::lower::types::RawMeasureItemKind;
use crate::lower::types::RepeatDirection;
use crate::lower::types::SegmentedMeasure;
use crate::lower::types::Span;
use crate::lower::types::VoiceEvent;
use crate::source_map::SourceProvenance;

/// Partitions a flat stream of voice events into discrete measures bounded by barlines.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn segment_voice_measures(events: Vec<VoiceEvent>) -> Vec<SegmentedMeasure> {
    let mut measures: Vec<SegmentedMeasure> = Vec::new();
    let mut current = SegmentedMeasure::default();
    let mut active_ending_number: Option<String> = None;
    let mut pending_new_system = false;
    let mut pending_new_page = false;

    for event in events {
        match event {
            VoiceEvent::Meter(m, prov) => {
                current.items.push(RawMeasureItem {
                    line_span: prov.line_span(),
                    element_span: prov.element_span(),
                    kind: RawMeasureItemKind::Meter(m, prov),
                });
            }
            VoiceEvent::UnitLength(u, prov) => {
                current.items.push(RawMeasureItem {
                    line_span: prov.line_span(),
                    element_span: prov.element_span(),
                    kind: RawMeasureItemKind::UnitLength(u),
                });
            }
            VoiceEvent::Key(k, prov) => {
                current.items.push(RawMeasureItem {
                    line_span: prov.line_span(),
                    element_span: prov.element_span(),
                    kind: RawMeasureItemKind::Key(k, prov),
                });
            }
            VoiceEvent::VoiceParams(info) => {
                let span = info
                    .provenance
                    .as_ref()
                    .map_or_else(|| Span::from(0..0), SourceProvenance::line_span);
                current.items.push(RawMeasureItem {
                    line_span: span,
                    element_span: span,
                    kind: RawMeasureItemKind::VoiceParams(info),
                });
            }
            VoiceEvent::Propagation(prop) => {
                current.items.push(RawMeasureItem {
                    line_span: Span::from(0..0),
                    element_span: Span::from(0..0),
                    kind: RawMeasureItemKind::Propagation(prop),
                });
            }
            VoiceEvent::Tempo(tempo, prov) => {
                current.items.push(RawMeasureItem {
                    line_span: prov.line_span(),
                    element_span: prov.element_span(),
                    kind: RawMeasureItemKind::Tempo(tempo, prov),
                });
            }
            VoiceEvent::Lyrics(field) => {
                current.items.push(RawMeasureItem {
                    line_span: field.line_span,
                    element_span: field.line_span,
                    kind: RawMeasureItemKind::Lyrics(field),
                });
            }
            VoiceEvent::Symbols(field) => {
                current.items.push(RawMeasureItem {
                    line_span: field.line_span,
                    element_span: field.line_span,
                    kind: RawMeasureItemKind::Symbols(field),
                });
            }
            VoiceEvent::MusicElement {
                element,
                line_span,
                element_span,
            } => match element {
                MusicElement::Note(note) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Note(note),
                    });
                }
                MusicElement::Rest(rest) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Rest(rest),
                    });
                }
                MusicElement::Chord(chord) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Chord(chord),
                    });
                }
                MusicElement::MultiMeasureRest(mm_rest) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::MultiMeasureRest(mm_rest),
                    });
                }
                MusicElement::Grace(grace) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Grace(grace),
                    });
                }
                MusicElement::Tuplet(tuplet) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Tuplet(tuplet),
                    });
                }
                MusicElement::BrokenRhythm(rhythm) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::BrokenRhythm(rhythm),
                    });
                }
                MusicElement::Tie(tie) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Tie(tie),
                    });
                }
                MusicElement::Slur(slur) => {
                    current.note_first_span(line_span);
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Slur(
                            slur,
                            SourceProvenance::new(line_span, element_span),
                        ),
                    });
                }
                MusicElement::Decoration(dec) => {
                    if dec.name.eq_ignore_ascii_case("linebreak")
                        || dec.name.eq_ignore_ascii_case("line-break")
                    {
                        if has_timed_items(&current.items) {
                            pending_new_system = true;
                        } else {
                            current.new_system = true;
                        }
                    } else if dec.name.eq_ignore_ascii_case("newpage") {
                        if has_timed_items(&current.items) {
                            pending_new_page = true;
                        } else {
                            current.new_page = true;
                        }
                    } else {
                        current.items.push(RawMeasureItem {
                            line_span,
                            element_span,
                            kind: RawMeasureItemKind::Decoration(
                                dec,
                                SourceProvenance::new(line_span, element_span),
                            ),
                        });
                    }
                }
                MusicElement::Annotation(ann) => {
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Annotation(
                            ann,
                            SourceProvenance::new(line_span, element_span),
                        ),
                    });
                }
                MusicElement::LineBreak(lb) => match lb {
                    LineBreak::Break => {
                        if has_timed_items(&current.items) {
                            pending_new_system = true;
                        } else {
                            current.new_system = true;
                        }
                    }
                    LineBreak::Paragraph => {
                        if has_timed_items(&current.items) {
                            pending_new_page = true;
                        } else {
                            current.new_page = true;
                        }
                    }
                    _ => {}
                },
                MusicElement::BeamBreak(_) => {
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::BeamBreak,
                    });
                }
                MusicElement::BeamContinuation(_) => {
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::BeamContinuation,
                    });
                }
                MusicElement::Ending(ending) => {
                    current.note_first_span(line_span);
                    let number = format_ending_numbers(&ending);
                    let ending_prov = SourceProvenance::new(line_span, element_span);

                    if let Some(prev_num) = active_ending_number.take()
                        && let Some(prev_measure) = measures.last_mut()
                    {
                        attach_ending_closure(prev_measure, prev_num, EndingType::Discontinue);
                    }

                    if has_timed_items(&current.items) {
                        push_segmented_measure(
                            &mut current,
                            &mut measures,
                            &mut pending_new_system,
                            &mut pending_new_page,
                        );
                        current.note_first_span(line_span);
                    }

                    let left = current.left_barline.get_or_insert(LoweredBarline {
                        location: "left",
                        bar_style: None,
                        fermata: None,
                        ending: None,
                        repeat: None,
                        provenance: Some(ending_prov),
                    });
                    left.ending = Some(LoweredEnding {
                        number: number.clone(),
                        ending_type: EndingType::Start,
                    });
                    active_ending_number = Some(number);
                }
                MusicElement::Bar(bar) => {
                    let bar_prov = SourceProvenance::new(line_span, element_span);
                    apply_bar_token(
                        &bar,
                        line_span,
                        bar_prov,
                        &mut current,
                        &mut measures,
                        &mut active_ending_number,
                        &mut pending_new_system,
                        &mut pending_new_page,
                    );
                }
                MusicElement::Overlay(overlay) => {
                    current.items.push(RawMeasureItem {
                        line_span,
                        element_span,
                        kind: RawMeasureItemKind::Overlay(overlay),
                    });
                }
                _ => {}
            },
        }
    }

    if pending_new_system {
        current.new_system = true;
    }
    if pending_new_page {
        current.new_page = true;
    }

    if current.has_musical_content() || measures.is_empty() {
        if let Some(open_num) = active_ending_number.take() {
            let right = current.right_barline.get_or_insert(LoweredBarline {
                location: "right",
                bar_style: None,
                fermata: None,
                ending: None,
                repeat: None,
                provenance: None,
            });
            if right.ending.is_none() {
                right.ending = Some(LoweredEnding {
                    number: open_num,
                    ending_type: EndingType::Discontinue,
                });
            }
        }
        measures.push(current);
    } else {
        if let Some(open_num) = active_ending_number.take()
            && let Some(last_measure) = measures.last_mut()
        {
            attach_ending_closure(last_measure, open_num, EndingType::Discontinue);
        }
        if let Some(last_measure) = measures.last_mut() {
            if current.new_system {
                last_measure.new_system = true;
            }
            if current.new_page {
                last_measure.new_page = true;
            }
            last_measure.items.extend(current.items);
        }
    }

    measures
}

/// Returns whether `items` contains any note, rest, chord, multi-measure rest, or grace note.
#[must_use]
pub fn has_timed_items(items: &[RawMeasureItem]) -> bool {
    items.iter().any(|item| {
        matches!(
            item.kind,
            RawMeasureItemKind::Note(_)
                | RawMeasureItemKind::Rest(_)
                | RawMeasureItemKind::Chord(_)
                | RawMeasureItemKind::MultiMeasureRest(_)
                | RawMeasureItemKind::Grace(_)
        )
    })
}

/// Attaches a closing ending barline notation to `measure`.
pub fn attach_ending_closure(
    measure: &mut SegmentedMeasure,
    number: String,
    default_type: EndingType,
) {
    let right = measure.right_barline.get_or_insert(LoweredBarline {
        location: "right",
        bar_style: None,
        fermata: None,
        ending: None,
        repeat: None,
        provenance: None,
    });
    if right.ending.is_none() {
        let ending_type = if matches!(right.repeat, Some((RepeatDirection::Backward, _))) {
            EndingType::Stop
        } else {
            default_type
        };
        right.ending = Some(LoweredEnding {
            number,
            ending_type,
        });
    }
}

/// Removes and returns trailing fermata decoration from the end of `items`.
pub fn pop_trailing_fermata(
    items: &mut Vec<RawMeasureItem>,
) -> Option<(&'static str, Option<SourceProvenance>)> {
    let mut idx = items.len();
    while idx > 0 {
        idx -= 1;
        match &items[idx].kind {
            RawMeasureItemKind::BeamBreak | RawMeasureItemKind::BeamContinuation => {}
            RawMeasureItemKind::Decoration(dec, prov) => {
                if dec.name == "fermata" {
                    let p = prov.clone();
                    items.remove(idx);
                    return Some(("upright", Some(p)));
                } else if dec.name == "invertedfermata" {
                    let p = prov.clone();
                    items.remove(idx);
                    return Some(("inverted", Some(p)));
                }
                break;
            }
            _ => break,
        }
    }
    None
}

fn push_segmented_measure(
    current: &mut SegmentedMeasure,
    measures: &mut Vec<SegmentedMeasure>,
    pending_new_system: &mut bool,
    pending_new_page: &mut bool,
) {
    measures.push(std::mem::take(current));
    if *pending_new_system {
        current.new_system = true;
        *pending_new_system = false;
    }
    if *pending_new_page {
        current.new_page = true;
        *pending_new_page = false;
    }
}

/// Processes an ABC [`BarLine`] token, applying barlines, endings, repeats, and measure breaks.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub fn apply_bar_token(
    bar: &BarLine<String>,
    line_span: Span,
    bar_prov: SourceProvenance,
    current: &mut SegmentedMeasure,
    measures: &mut Vec<SegmentedMeasure>,
    active_ending_number: &mut Option<String>,
    pending_new_system: &mut bool,
    pending_new_page: &mut bool,
) {
    let trailing_fermata = pop_trailing_fermata(&mut current.items);
    let at_measure_start = !has_timed_items(&current.items) && current.left_barline.is_none();

    match bar.kind {
        BarKind::Single => {
            if at_measure_start {
                current.note_first_span(line_span);
                if trailing_fermata.is_some() {
                    let left = current.left_barline.get_or_insert(LoweredBarline {
                        location: "left",
                        bar_style: None,
                        fermata: None,
                        ending: None,
                        repeat: None,
                        provenance: Some(bar_prov),
                    });
                    left.fermata = trailing_fermata;
                }
            } else {
                if trailing_fermata.is_some() {
                    current.right_barline = Some(LoweredBarline {
                        location: "right",
                        bar_style: None,
                        fermata: trailing_fermata,
                        ending: None,
                        repeat: None,
                        provenance: Some(bar_prov),
                    });
                }
                push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            }
        }
        BarKind::RepeatStart => {
            if !at_measure_start {
                if let Some(open_num) = active_ending_number.take() {
                    let right = current.right_barline.get_or_insert(LoweredBarline {
                        location: "right",
                        bar_style: None,
                        fermata: None,
                        ending: None,
                        repeat: None,
                        provenance: None,
                    });
                    right.ending = Some(LoweredEnding {
                        number: open_num,
                        ending_type: EndingType::Discontinue,
                    });
                }
                push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            }
            current.note_first_span(line_span);
            current.left_barline = Some(LoweredBarline {
                location: "left",
                bar_style: Some("heavy-light"),
                fermata: trailing_fermata,
                ending: None,
                repeat: Some((
                    RepeatDirection::Forward,
                    repeat_count_from_source(&bar.source),
                )),
                provenance: Some(bar_prov),
            });
        }
        BarKind::ThickThin => {
            if at_measure_start {
                current.note_first_span(line_span);
                current.left_barline = Some(LoweredBarline {
                    location: "left",
                    bar_style: Some("heavy-light"),
                    fermata: trailing_fermata,
                    ending: None,
                    repeat: None,
                    provenance: Some(bar_prov),
                });
            } else {
                current.right_barline = Some(LoweredBarline {
                    location: "right",
                    bar_style: Some("heavy-light"),
                    fermata: trailing_fermata,
                    ending: active_ending_number.take().map(|number| LoweredEnding {
                        number,
                        ending_type: EndingType::Discontinue,
                    }),
                    repeat: None,
                    provenance: Some(bar_prov),
                });
                push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            }
        }
        BarKind::Double | BarKind::ThinThick | BarKind::Dotted | BarKind::Invisible => {
            let style = match bar.kind {
                BarKind::Double => "light-light",
                BarKind::ThinThick => "light-heavy",
                BarKind::Dotted => "dotted",
                BarKind::Invisible => "none",
                _ => "regular",
            };
            let ending = if matches!(bar.kind, BarKind::Double | BarKind::ThinThick) {
                active_ending_number.take().map(|number| LoweredEnding {
                    number,
                    ending_type: EndingType::Discontinue,
                })
            } else {
                None
            };
            current.note_first_span(line_span);
            current.right_barline = Some(LoweredBarline {
                location: "right",
                bar_style: Some(style),
                fermata: trailing_fermata,
                ending,
                repeat: None,
                provenance: Some(bar_prov),
            });
            push_segmented_measure(current, measures, pending_new_system, pending_new_page);
        }
        BarKind::RepeatEnd => {
            current.note_first_span(line_span);
            current.right_barline = Some(LoweredBarline {
                location: "right",
                bar_style: Some("light-heavy"),
                fermata: trailing_fermata,
                ending: active_ending_number.take().map(|number| LoweredEnding {
                    number,
                    ending_type: EndingType::Stop,
                }),
                repeat: Some((
                    RepeatDirection::Backward,
                    repeat_count_from_source(&bar.source),
                )),
                provenance: Some(bar_prov),
            });
            push_segmented_measure(current, measures, pending_new_system, pending_new_page);
        }
        BarKind::RepeatBoth => {
            current.note_first_span(line_span);
            current.right_barline = Some(LoweredBarline {
                location: "right",
                bar_style: Some("light-heavy"),
                fermata: trailing_fermata,
                ending: active_ending_number.take().map(|number| LoweredEnding {
                    number,
                    ending_type: EndingType::Stop,
                }),
                repeat: Some((RepeatDirection::Backward, None)),
                provenance: Some(bar_prov.clone()),
            });
            push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            current.note_first_span(line_span);
            current.left_barline = Some(LoweredBarline {
                location: "left",
                bar_style: Some("heavy-light"),
                fermata: None,
                ending: None,
                repeat: Some((RepeatDirection::Forward, None)),
                provenance: Some(bar_prov),
            });
        }
        BarKind::Other => {
            if bar.source.starts_with(':') {
                current.note_first_span(line_span);
                current.right_barline = Some(LoweredBarline {
                    location: "right",
                    bar_style: Some("light-heavy"),
                    fermata: trailing_fermata,
                    ending: active_ending_number.take().map(|number| LoweredEnding {
                        number,
                        ending_type: EndingType::Stop,
                    }),
                    repeat: Some((
                        RepeatDirection::Backward,
                        repeat_count_from_source(&bar.source),
                    )),
                    provenance: Some(bar_prov.clone()),
                });
                push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            }
            if bar.source.ends_with(':') {
                current.note_first_span(line_span);
                current.left_barline = Some(LoweredBarline {
                    location: "left",
                    bar_style: Some("heavy-light"),
                    fermata: None,
                    ending: None,
                    repeat: Some((
                        RepeatDirection::Forward,
                        repeat_count_from_source(&bar.source),
                    )),
                    provenance: Some(bar_prov),
                });
            } else if !bar.source.starts_with(':') && !at_measure_start {
                push_segmented_measure(current, measures, pending_new_system, pending_new_page);
            }
        }
    }
}

/// Parses repeat count from colon count in source bar token (e.g. `::` or `:::`);
#[must_use]
pub fn repeat_count_from_source(source: &str) -> Option<u32> {
    let colons = source.bytes().filter(|&b| b == b':').count();
    (colons > 1).then(|| u32::try_from(colons + 1).unwrap_or(2))
}

/// Formats variant ending selector list into comma-separated numbers string.
#[must_use]
pub fn format_ending_numbers(ending: &VariantEnding) -> String {
    let mut numbers = Vec::new();
    for selector in &ending.selectors {
        match *selector {
            EndingSelector::Number(n) => numbers.push(n.max(1)),
            EndingSelector::Range { start, end } => {
                let lo = start.min(end).max(1);
                let hi = start.max(end).max(1);
                for n in lo..=hi {
                    numbers.push(n);
                }
            }
        }
    }
    numbers
        .into_iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Merges multiple voice measure sequences into a single segmented measure sequence
/// using overlay markers (`&`) between voices within each measure.
#[must_use]
pub fn merge_segmented_measures(
    segmented_voices: &[Vec<SegmentedMeasure>],
) -> Vec<SegmentedMeasure> {
    let max_measures = segmented_voices.iter().map(Vec::len).max().unwrap_or(0);
    let mut merged = Vec::with_capacity(max_measures);

    for m in 0..max_measures {
        let mut combined_items = Vec::new();
        let mut left_barline = None;
        let mut right_barline = None;
        let mut first_line_span = None;
        let mut new_system = false;
        let mut new_page = false;

        for (voice_layer, voice_measures) in segmented_voices.iter().enumerate() {
            if m < voice_measures.len() {
                let seg = &voice_measures[m];
                if seg.new_system {
                    new_system = true;
                }
                if seg.new_page {
                    new_page = true;
                }
                if voice_layer == 0 {
                    left_barline.clone_from(&seg.left_barline);
                    right_barline.clone_from(&seg.right_barline);
                    first_line_span = seg.first_line_span;
                } else {
                    if left_barline.is_none() {
                        left_barline.clone_from(&seg.left_barline);
                    }
                    if right_barline.is_none() {
                        right_barline.clone_from(&seg.right_barline);
                    }
                    if first_line_span.is_none() {
                        first_line_span = seg.first_line_span;
                    }
                    let line_span = seg
                        .first_line_span
                        .unwrap_or_else(|| SimpleSpan::from(0..0));
                    combined_items.push(RawMeasureItem {
                        kind: RawMeasureItemKind::MergedVoice(voice_layer),
                        line_span,
                        element_span: SimpleSpan::from(0..0),
                    });
                }
                combined_items.extend(seg.items.iter().cloned());
            }
        }

        merged.push(SegmentedMeasure {
            first_line_span,
            left_barline,
            right_barline,
            items: combined_items,
            new_system,
            new_page,
        });
    }

    merged
}
