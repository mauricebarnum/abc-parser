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

//! Tune and document lowering engine from `abc-parser` AST to `MusicXML` 4.0.

pub(crate) mod beams;
pub(crate) mod emit;
pub(crate) mod header;
pub(crate) mod lyrics_align;
pub(crate) mod note;
pub(crate) mod segment;
pub(crate) mod spanners;
pub(crate) mod types;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::fmt;

use abc_parser::BarDurationPickupPolicy;
use abc_parser::Chord;
use abc_parser::ChordMember;
use abc_parser::ErrorKind;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::Line;
use abc_parser::Meter;
use abc_parser::MultiMeasureRest;
use abc_parser::MusicElement;
use abc_parser::Note;
use abc_parser::NoteLength;
use abc_parser::Overlay;
use abc_parser::ParseWarning;
use abc_parser::Pitch;
use abc_parser::Rest;
use abc_parser::RestKind;
use abc_parser::Spanned;
use abc_parser::Tune;
use abc_parser::pitch::Offset;
use abc_parser::pitch::class_index;

pub(crate) use self::types::PitchOctaveKey;
pub(crate) use self::types::SlurDescriptor;
pub(crate) use self::types::SpannerEndpoint;
pub(crate) use self::types::VoiceLayerKey;
use crate::AnnotationLevel;
use crate::MusicXmlError;
use crate::MusicXmlOptions;
use crate::lower::beams::finish_beam_group;
use crate::lower::beams::note_beam_level_count;
use crate::lower::emit::emit_measure;
use crate::lower::emit::emit_score_header;
use crate::lower::header::apply_body_directive;
use crate::lower::header::apply_body_field;
use crate::lower::header::consume_continuations;
use crate::lower::header::ensure_voice;
use crate::lower::header::expand_and_push_music_element;
use crate::lower::header::extract_field_segments;
use crate::lower::header::process_header_slice;
use crate::lower::header::push_typeset_credit;
use crate::lower::lyrics_align::lower_symbol_element;
use crate::lower::note::apply_decoration_or_direction;
use crate::lower::note::build_note_kind;
use crate::lower::note::lower_annotation;
use crate::lower::note::lower_grace_group;
use crate::lower::note::tempo_to_direction;
use crate::lower::segment::merge_segmented_measures;
use crate::lower::segment::segment_voice_measures;
use crate::lower::spanners::ActiveTupletState;
use crate::lower::spanners::TimedItemMods;
use crate::lower::spanners::attach_slur_stop;
use crate::lower::types::LoweredAttributes;
use crate::lower::types::LoweredHeader;
use crate::lower::types::LoweredLyric;
use crate::lower::types::LoweredMeasure;
use crate::lower::types::LoweredNotations;
use crate::lower::types::LoweredNote;
use crate::lower::types::LoweredPart;
use crate::lower::types::LoweredSlur;
use crate::lower::types::LoweredTune;
use crate::lower::types::LyricsField;
use crate::lower::types::MeasureItem;
use crate::lower::types::NoteSlotCoord;
use crate::lower::types::RawMeasureItem;
use crate::lower::types::RawMeasureItemKind;
use crate::lower::types::SegmentedMeasure;
use crate::lower::types::Span;
use crate::lower::types::SymbolsField;
use crate::lower::types::VoiceDeclInfo;
use crate::lower::types::VoiceDefaults;
use crate::lower::types::VoiceEvent;
use crate::lyrics::LyricKind;
use crate::lyrics::Step;
use crate::lyrics::SymbolKind;
use crate::lyrics::align;
use crate::lyrics::tokenize_lyrics;
use crate::lyrics::tokenize_symbols;
use crate::pitch::AccidentalPropagation;
use crate::pitch::ClefSpec;
use crate::pitch::MeasureAccidentalState;
use crate::pitch::ResolvedKey;
use crate::pitch::StemPolicy;
use crate::pitch::TransposeSpec;
use crate::score_directive::PartListEntry;
use crate::score_directive::StaffLayerSpec;
use crate::score_directive::StaffSpec;
use crate::score_directive::parse_score_directive;
use crate::source_map::SourceMap;
use crate::source_map::SourceProvenance;
use crate::timing::Duration;
use crate::timing::broken_rhythm_factors;
use crate::timing::compute_divisions;
use crate::timing::decompose_graphic_duration;
use crate::timing::is_compound_meter;
use crate::timing::lcm;
use crate::timing::meter_bar_duration;
use crate::timing::resolve_tuplet;
use crate::writer::ProvenanceEmitter;
use crate::writer::XmlWriter;

/// Lowers one parsed ABC [`Tune`] (together with optional file-header lines)
/// into a `MusicXML` 4.0 `<score-partwise>` document written to `output`.
///
/// # Errors
///
/// Returns [`MusicXmlError::FormatError`] if writing to `output` fails.
pub fn write_tune_musicxml<W: fmt::Write>(
    output: &mut W,
    tune: &Tune<Span, String>,
    file_header: &[Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    options: &MusicXmlOptions,
) -> Result<Vec<ParseWarning<Span>>, MusicXmlError> {
    let source_name = options.source_name().unwrap_or("<input>");
    let owned_map = (options.annotation_level() != AnnotationLevel::None)
        .then(|| source.map(|src| SourceMap::new(src, source_name)))
        .flatten();
    write_tune_musicxml_with_map(output, tune, file_header, source, owned_map, options)
}

pub(crate) fn write_tune_musicxml_with_map<W: fmt::Write>(
    output: &mut W,
    tune: &Tune<Span, String>,
    file_header: &[Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    source_map: Option<SourceMap<'_>>,
    options: &MusicXmlOptions,
) -> Result<Vec<ParseWarning<Span>>, MusicXmlError> {
    let mut emitter = ProvenanceEmitter::new(source_map, options.annotation_level());
    let mut writer = XmlWriter::new(output, options.indent_spaces());

    let mut warnings = Vec::new();
    let lowered = lower_tune(tune, file_header, source, options, &mut warnings);

    writer.write_declaration(options.include_doctype())?;
    writer.open_tag_with_attrs("score-partwise", &[("version", "4.0")])?;

    emit_score_header(
        &mut writer,
        &mut emitter,
        &lowered.header,
        &lowered.parts,
        &lowered.part_list_entries,
        lowered.score_directive_provenance.as_ref(),
    )?;

    for part in &lowered.parts {
        emitter.reset_part_cursors();
        writer.open_tag_with_attrs("part", &[("id", &part.part_id)])?;
        for measure in &part.measures {
            emit_measure(&mut writer, &mut emitter, measure, part.divisions)?;
        }
        writer.close_tag("part")?;
    }

    writer.close_tag("score-partwise")?;
    Ok(warnings)
}

/// Extracts the primary `X:` reference number and concatenated `T:` title of a tune.
#[must_use]
pub fn extract_tune_identity(tune: &Tune<Span, String>) -> (Option<u32>, Option<String>) {
    let mut reference = None;
    let mut title = None;

    let mut idx = 0;
    while idx < tune.lines.len() {
        if let Line::Field(field) = &tune.lines[idx].value {
            match field.kind {
                FieldKind::Reference if reference.is_none() => {
                    if let FieldValue::Reference(num) = field.value {
                        reference = Some(num);
                    }
                    idx += 1;
                }
                FieldKind::Title if title.is_none() => {
                    let initial = match &field.value {
                        FieldValue::Text(t) | FieldValue::Unparsed(t) => t.trim().to_owned(),
                        _ => String::new(),
                    };
                    let mut buf = Some(initial);
                    let scan = consume_continuations(&tune.lines, idx, &mut buf);
                    title = buf;
                    idx = scan.next_idx;
                }
                _ => {
                    idx += 1;
                }
            }
        } else {
            idx += 1;
        }
        if reference.is_some() && title.is_some() {
            break;
        }
    }

    (reference, title)
}

fn lower_tune(
    tune: &Tune<Span, String>,
    file_header: &[Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    options: &MusicXmlOptions,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> LoweredTune {
    let mut header = LoweredHeader::default();
    let mut defaults = VoiceDefaults::default();
    let mut voice_decls: Vec<VoiceDeclInfo> = Vec::new();
    let mut voice_events: BTreeMap<String, Vec<VoiceEvent>> = BTreeMap::new();
    let mut voice_order: Vec<String> = Vec::new();

    process_header_slice(
        file_header,
        source,
        &mut header,
        &mut defaults,
        &mut voice_decls,
        warnings,
    );

    let body_start_idx = partition_body_start(&tune.lines);
    process_header_slice(
        &tune.lines[..body_start_idx],
        source,
        &mut header,
        &mut defaults,
        &mut voice_decls,
        warnings,
    );

    for decl in &voice_decls {
        if !voice_order.contains(&decl.id) {
            voice_order.push(decl.id.clone());
            voice_events.entry(decl.id.clone()).or_default();
        }
    }

    collect_body_events(
        &tune.lines[body_start_idx..],
        source,
        &mut header,
        &defaults,
        &mut voice_order,
        &mut voice_decls,
        &mut voice_events,
        warnings,
    );

    if voice_order.is_empty() {
        voice_order.push(String::new());
        voice_events.insert(String::new(), Vec::new());
    }

    let (staves, part_list_entries, score_directive_provenance) =
        resolve_score_layout(&defaults, &voice_order, warnings);

    let multiple_staves = staves.len() > 1;
    let mut parts = Vec::with_capacity(staves.len());
    for (staff_idx, staff) in staves.iter().enumerate() {
        let is_part_one = staff_idx == 0;
        let part = if staff.staves.len() > 1 {
            build_multi_staff_part(
                staff,
                &voice_decls,
                &defaults,
                &mut voice_events,
                is_part_one,
                options.pickup_policy(),
                warnings,
            )
        } else {
            build_voice_part(
                &staff.voice_ids,
                staff.part_id.clone(),
                None,
                multiple_staves,
                &voice_decls,
                &defaults,
                &mut voice_events,
                is_part_one,
                options.pickup_policy(),
                warnings,
            )
        };
        parts.push(part);
    }

    synchronize_part_measure_counts(&mut parts, &defaults);

    LoweredTune {
        header,
        part_list_entries,
        score_directive_provenance,
        parts,
    }
}

fn partition_body_start(lines: &[Spanned<Line<Span, String>, Span>]) -> usize {
    for (idx, line) in lines.iter().enumerate() {
        match &line.value {
            Line::Field(field) if field.kind == FieldKind::Key => return idx + 1,
            Line::Music(_) => return idx,
            _ => {}
        }
    }
    lines.len()
}

struct BodyCursorState {
    current_voice: String,
    active_user_symbols: BTreeMap<char, Vec<MusicElement<String>>>,
    last_was_lyrics: bool,
    current_verse: u32,
    last_was_symbols: bool,
    current_symbol_layer: usize,
}

#[allow(clippy::too_many_arguments)]
fn collect_body_events(
    body_lines: &[Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    header: &mut LoweredHeader,
    defaults: &VoiceDefaults,
    voice_order: &mut Vec<String>,
    voice_decls: &mut Vec<VoiceDeclInfo>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    let mut cursor = BodyCursorState {
        current_voice: voice_order.first().cloned().unwrap_or_default(),
        active_user_symbols: defaults.user_symbols.clone(),
        last_was_lyrics: false,
        current_verse: 1,
        last_was_symbols: false,
        current_symbol_layer: 0,
    };

    let mut idx = 0;
    while idx < body_lines.len() {
        let line = &body_lines[idx];
        let mut extra_spans = Vec::new();
        let mut combined_text_override = None;

        if let Line::Field(field) = &line.value {
            let mut text_buf = match &field.value {
                FieldValue::Text(t) | FieldValue::Unparsed(t) => Some(t.clone()),
                _ => None,
            };
            let scan = consume_continuations(body_lines, idx, &mut text_buf);
            if matches!(field.kind, FieldKind::Lyrics | FieldKind::Symbols) {
                push_lyrics_or_symbols(
                    field.kind,
                    line,
                    &scan.continuation_lines,
                    source,
                    &mut cursor,
                    voice_order,
                    voice_events,
                );
                idx = scan.next_idx;
                continue;
            }
            extra_spans = scan.extra_spans;
            combined_text_override = text_buf;
            idx = scan.next_idx;
        } else {
            idx += 1;
        }

        if !matches!(line.value, Line::Comment(_)) {
            cursor.last_was_lyrics = false;
            cursor.last_was_symbols = false;
        }

        let prov = SourceProvenance::from_line(line.span).with_extra_line_spans(extra_spans);
        dispatch_body_line(
            line,
            combined_text_override.as_deref(),
            &prov,
            source,
            header,
            &mut cursor,
            voice_order,
            voice_decls,
            voice_events,
            warnings,
        );
    }
}

fn push_lyrics_or_symbols(
    kind: FieldKind,
    line: &Spanned<Line<Span, String>, Span>,
    continuation_lines: &[&Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    cursor: &mut BodyCursorState,
    voice_order: &mut Vec<String>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
) {
    ensure_voice(voice_order, voice_events, &cursor.current_voice);
    let segments = extract_field_segments(line, continuation_lines, source);
    let stream = voice_events
        .get_mut(&cursor.current_voice)
        .expect("voice initialized");
    if kind == FieldKind::Lyrics {
        let tokens = tokenize_lyrics(segments);
        cursor.current_verse = if cursor.last_was_lyrics {
            cursor.current_verse + 1
        } else {
            cursor.last_was_lyrics = true;
            1
        };
        cursor.last_was_symbols = false;
        stream.push(VoiceEvent::Lyrics(LyricsField {
            verse: cursor.current_verse,
            tokens,
            line_span: line.span,
        }));
    } else {
        let tokens = tokenize_symbols(segments);
        cursor.current_symbol_layer = if cursor.last_was_symbols {
            cursor.current_symbol_layer + 1
        } else {
            cursor.last_was_symbols = true;
            0
        };
        cursor.last_was_lyrics = false;
        stream.push(VoiceEvent::Symbols(SymbolsField {
            layer: cursor.current_symbol_layer,
            tokens,
            line_span: line.span,
        }));
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch_body_line(
    line: &Spanned<Line<Span, String>, Span>,
    text_override: Option<&str>,
    prov: &SourceProvenance,
    source: Option<&str>,
    header: &mut LoweredHeader,
    cursor: &mut BodyCursorState,
    voice_order: &mut Vec<String>,
    voice_decls: &mut Vec<VoiceDeclInfo>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    match &line.value {
        Line::Field(field) => {
            apply_body_field(
                field,
                text_override,
                prov,
                source,
                header,
                &mut cursor.active_user_symbols,
                &mut cursor.current_voice,
                voice_order,
                voice_decls,
                voice_events,
                warnings,
            );
        }
        Line::Directive(directive) => {
            apply_body_directive(
                directive,
                prov,
                &cursor.current_voice,
                voice_order,
                voice_events,
            );
        }
        Line::Music(elements) => {
            ensure_voice(voice_order, voice_events, &cursor.current_voice);
            for elem in elements {
                if let MusicElement::InlineField(field) = &elem.value {
                    let inline_prov = SourceProvenance::new(line.span, elem.span);
                    apply_body_field(
                        field,
                        None,
                        &inline_prov,
                        source,
                        header,
                        &mut cursor.active_user_symbols,
                        &mut cursor.current_voice,
                        voice_order,
                        voice_decls,
                        voice_events,
                        warnings,
                    );
                } else {
                    ensure_voice(voice_order, voice_events, &cursor.current_voice);
                    let stream = voice_events
                        .get_mut(&cursor.current_voice)
                        .expect("voice initialized");
                    expand_and_push_music_element(
                        elem,
                        line.span,
                        source,
                        &cursor.active_user_symbols,
                        stream,
                    );
                }
            }
        }
        Line::TypesetText(typeset) => {
            push_typeset_credit(&mut header.credits, typeset, prov.clone());
        }
        _ => {}
    }
}

fn default_staff_layout(voice_order: &[String]) -> (Vec<StaffSpec>, Vec<PartListEntry>) {
    let mut def_staves = Vec::with_capacity(voice_order.len());
    let mut def_entries = Vec::with_capacity(voice_order.len());
    for (part_idx, voice_id) in voice_order.iter().enumerate() {
        def_staves.push(StaffSpec {
            part_id: format!("P{}", part_idx + 1),
            voice_ids: vec![voice_id.clone()],
            staves: vec![StaffLayerSpec {
                voice_ids: vec![voice_id.clone()],
            }],
        });
        def_entries.push(PartListEntry::ScorePart(part_idx));
    }
    (def_staves, def_entries)
}

fn resolve_score_layout(
    defaults: &VoiceDefaults,
    voice_order: &[String],
    warnings: &mut Vec<ParseWarning<Span>>,
) -> (Vec<StaffSpec>, Vec<PartListEntry>, Option<SourceProvenance>) {
    if let Some((kind, ref args, ref prov)) = defaults.score_directive {
        let layout = parse_score_directive(kind, args);
        if layout.staves.is_empty() {
            let (staves, entries) = default_staff_layout(voice_order);
            return (staves, entries, None);
        }
        let mentioned_voices = layout.all_voice_ids();
        for v in voice_order {
            if !mentioned_voices.contains(v.as_str()) && !v.is_empty() {
                warnings.push(ParseWarning {
                    kind: ErrorKind::InvalidDirective,
                    message: format!(
                        "voice \"{v}\" omitted because it is not listed in score directive"
                    ),
                    span: prov.element_span(),
                    related: Vec::new(),
                });
            }
        }
        return (layout.staves, layout.entries, Some(prov.clone()));
    }
    let (staves, entries) = default_staff_layout(voice_order);
    (staves, entries, None)
}

#[allow(clippy::too_many_arguments)]
fn build_voice_part(
    voice_ids: &[String],
    part_id: String,
    empty_voice_fallback_name: Option<String>,
    multiple_staves: bool,
    voice_decls: &[VoiceDeclInfo],
    defaults: &VoiceDefaults,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    is_part_one: bool,
    pickup_policy: BarDurationPickupPolicy,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> LoweredPart {
    if voice_ids.len() == 1 {
        let voice_id = &voice_ids[0];
        let decl = voice_decls.iter().find(|d| d.id == *voice_id);
        let part_name = decl.and_then(|d| d.name.clone()).unwrap_or_else(|| {
            if let Some(fallback) = empty_voice_fallback_name
                && voice_id.is_empty()
            {
                fallback
            } else if !voice_id.is_empty() && multiple_staves {
                format!("Voice {voice_id}")
            } else if !voice_id.is_empty() {
                voice_id.clone()
            } else {
                "Music".to_owned()
            }
        });
        let part_abbreviation = decl.and_then(|d| d.subname.clone());
        let declaration_provenance = decl.and_then(|d| d.provenance.clone());
        let events = voice_events.remove(voice_id).unwrap_or_default();
        let segmented = segment_voice_measures(events);
        return build_part_from_segmented(
            part_id,
            part_name,
            part_abbreviation,
            declaration_provenance,
            decl,
            defaults,
            &segmented,
            is_part_one,
            pickup_policy,
            warnings,
        );
    }

    let decls: Vec<Option<&VoiceDeclInfo>> = voice_ids
        .iter()
        .map(|vid| voice_decls.iter().find(|d| d.id == *vid))
        .collect();
    let primary_decl = decls.first().copied().flatten();
    let names: Vec<String> = voice_ids
        .iter()
        .zip(&decls)
        .map(|(vid, d)| {
            d.and_then(|decl| decl.name.clone())
                .unwrap_or_else(|| vid.clone())
        })
        .collect();
    let part_name = names.join(" / ");
    let abbrevs: Vec<String> = decls
        .iter()
        .filter_map(|d| d.and_then(|decl| decl.subname.clone()))
        .collect();
    let part_abbreviation = if abbrevs.len() == voice_ids.len() {
        Some(abbrevs.join(" / "))
    } else {
        primary_decl.and_then(|d| d.subname.clone())
    };
    let declaration_provenance = primary_decl.and_then(|d| d.provenance.clone());
    let segmented_voices: Vec<Vec<SegmentedMeasure>> = voice_ids
        .iter()
        .map(|vid| segment_voice_measures(voice_events.remove(vid).unwrap_or_default()))
        .collect();
    let segmented = merge_segmented_measures(&segmented_voices);
    build_part_from_segmented(
        part_id,
        part_name,
        part_abbreviation,
        declaration_provenance,
        primary_decl,
        defaults,
        &segmented,
        is_part_one,
        pickup_policy,
        warnings,
    )
}

fn collect_staff_clefs(
    attrs: &LoweredAttributes,
    staff_num: u8,
    out: &mut Vec<(ClefSpec, Option<u8>, Option<SourceProvenance>)>,
) {
    if let Some((clef, ref prov)) = attrs.clef {
        out.push((clef, Some(staff_num), prov.clone()));
    } else {
        for (clef, _, prov) in &attrs.clefs {
            out.push((*clef, Some(staff_num), prov.clone()));
        }
    }
}

fn build_multi_staff_part(
    staff: &StaffSpec,
    voice_decls: &[VoiceDeclInfo],
    defaults: &VoiceDefaults,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    is_part_one: bool,
    pickup_policy: BarDurationPickupPolicy,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> LoweredPart {
    let mut layer_parts = Vec::with_capacity(staff.staves.len());
    for (layer_idx, layer) in staff.staves.iter().enumerate() {
        let part = build_voice_part(
            &layer.voice_ids,
            format!("{}-S{}", staff.part_id, layer_idx + 1),
            Some(format!("Staff {}", layer_idx + 1)),
            false,
            voice_decls,
            defaults,
            voice_events,
            is_part_one && layer_idx == 0,
            pickup_policy,
            warnings,
        );
        layer_parts.push(part);
    }

    let combined_divisions = layer_parts
        .iter()
        .fold(1_u128, |acc, p| lcm(acc, p.divisions));
    let max_measures = layer_parts
        .iter()
        .map(|p| p.measures.len())
        .max()
        .unwrap_or(0);
    let mut lowered_measures = Vec::with_capacity(max_measures);

    for m in 0..max_measures {
        lowered_measures.push(merge_multi_staff_measure(
            m,
            &mut layer_parts,
            combined_divisions,
            staff.staves.len(),
            defaults,
            warnings,
        ));
    }

    let primary_decl = staff
        .voice_ids
        .first()
        .and_then(|vid| voice_decls.iter().find(|d| d.id == *vid));
    let part_name = primary_decl
        .and_then(|d| d.name.clone())
        .unwrap_or_else(|| "Piano".to_owned());
    let part_abbreviation = primary_decl.and_then(|d| d.subname.clone());
    let declaration_provenance = primary_decl.and_then(|d| d.provenance.clone());
    let midi_instrument = primary_decl.and_then(|d| d.midi).or(defaults.midi);

    LoweredPart {
        part_id: staff.part_id.clone(),
        part_name,
        part_abbreviation,
        declaration_provenance,
        divisions: combined_divisions,
        measures: lowered_measures,
        midi_instrument,
    }
}

fn merge_multi_staff_measure(
    m: usize,
    layer_parts: &mut [LoweredPart],
    combined_divisions: u128,
    staves_len: usize,
    defaults: &VoiceDefaults,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> LoweredMeasure {
    let target_duration = layer_parts
        .iter()
        .map(|p| {
            p.measures
                .get(m)
                .map_or(Duration::ZERO, |meas| meas.actual_duration)
        })
        .max()
        .unwrap_or(Duration::ZERO);

    let mut combined_items = Vec::new();
    let mut cumulative_voice_offset = 0_u32;
    let mut new_system = false;
    let mut new_page = false;
    let mut left_barline = None;
    let mut right_barline = None;
    let mut measure_number = String::new();
    let mut implicit = false;
    let mut provenance = None;
    let mut expected_duration = None;

    for (layer_idx, layer_part) in layer_parts.iter_mut().enumerate() {
        let staff_num = (layer_idx + 1) as u8;
        if layer_idx > 0 && !target_duration.is_zero() {
            combined_items.push(MeasureItem::Backup {
                duration: target_duration,
                provenance: None,
            });
        }

        let mut layer_voice_count = 1_u32;
        let mut layer_duration = Duration::ZERO;

        if let Some(layer_meas) = layer_part.measures.get_mut(m) {
            new_system |= layer_meas.new_system;
            new_page |= layer_meas.new_page;
            if left_barline.is_none() {
                left_barline = layer_meas.left_barline.take();
            }
            if right_barline.is_none() {
                right_barline = layer_meas.right_barline.take();
            }
            if measure_number.is_empty() && !layer_meas.number.is_empty() {
                measure_number.clone_from(&layer_meas.number);
            }
            implicit |= layer_meas.implicit;
            if provenance.is_none() {
                provenance = layer_meas.provenance.take();
            }
            if expected_duration.is_none() {
                expected_duration = layer_meas.expected_duration;
            }
            layer_duration = layer_meas.actual_duration;

            for item in std::mem::take(&mut layer_meas.items) {
                match item {
                    MeasureItem::Note(mut note) => {
                        note.staff = Some(staff_num);
                        layer_voice_count = layer_voice_count.max(note.voice);
                        note.voice += cumulative_voice_offset;
                        combined_items.push(MeasureItem::Note(note));
                    }
                    MeasureItem::Direction(_) | MeasureItem::Backup { .. } => {
                        combined_items.push(item);
                    }
                    MeasureItem::Harmony(_) if layer_idx == 0 => {
                        combined_items.push(item);
                    }
                    MeasureItem::Harmony(_) => {}
                }
            }
        }

        if layer_duration < target_duration {
            push_staff_pad_rest(
                &mut combined_items,
                m,
                &measure_number,
                staff_num,
                cumulative_voice_offset + 1,
                layer_duration,
                target_duration,
                warnings,
            );
        }
        cumulative_voice_offset += layer_voice_count;
    }

    let combined_attrs =
        build_multi_staff_attributes(m, layer_parts, combined_divisions, staves_len, defaults);

    LoweredMeasure {
        number: measure_number,
        implicit,
        provenance,
        left_barline,
        attributes: (!combined_attrs.is_empty()).then_some(combined_attrs),
        items: combined_items,
        right_barline,
        actual_duration: target_duration,
        expected_duration,
        new_system,
        new_page,
    }
}

#[allow(clippy::too_many_arguments)]
fn push_staff_pad_rest(
    combined_items: &mut Vec<MeasureItem>,
    measure_idx: usize,
    measure_number: &str,
    staff_num: u8,
    voice: u32,
    layer_duration: Duration,
    target_duration: Duration,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    let pad_dur = target_duration.saturating_sub(layer_duration);
    if !layer_duration.is_zero() {
        warnings.push(ParseWarning {
            kind: ErrorKind::InvalidMusic,
            message: format!(
                "measure {} duration mismatch on staff {staff_num}: expected {target_duration}, found {layer_duration}; padding with invisible rest",
                if measure_number.is_empty() {
                    (measure_idx + 1).to_string()
                } else {
                    measure_number.to_owned()
                },
            ),
            span: Span::from(0..0),
            related: Vec::new(),
        });
    }
    combined_items.push(MeasureItem::Note(LoweredNote::timed_rest(
        false, pad_dur, voice, None, 0, None, None,
    )));
    if let Some(MeasureItem::Note(n)) = combined_items.last_mut() {
        n.staff = Some(staff_num);
    }
}

fn build_multi_staff_attributes(
    m: usize,
    layer_parts: &[LoweredPart],
    combined_divisions: u128,
    staves_len: usize,
    defaults: &VoiceDefaults,
) -> LoweredAttributes {
    let mut combined_attrs = LoweredAttributes::default();
    if m == 0 {
        combined_attrs.divisions = Some((combined_divisions, defaults.unit_length.1.clone()));
        combined_attrs.staves = Some(staves_len as u8);
        combined_attrs.part_symbol = Some("brace");
        if let Some(first_layer) = layer_parts.first()
            && let Some(first_m) = first_layer.measures.first()
            && let Some(ref attrs) = first_m.attributes
        {
            combined_attrs.key.clone_from(&attrs.key);
            combined_attrs.time.clone_from(&attrs.time);
            combined_attrs.transpose.clone_from(&attrs.transpose);
            combined_attrs
                .multiple_rest
                .clone_from(&attrs.multiple_rest);
        }
        for (layer_idx, layer_part) in layer_parts.iter().enumerate() {
            if let Some(first_m) = layer_part.measures.first()
                && let Some(ref attrs) = first_m.attributes
            {
                collect_staff_clefs(attrs, (layer_idx + 1) as u8, &mut combined_attrs.clefs);
            }
        }
    } else {
        for (layer_idx, layer_part) in layer_parts.iter().enumerate() {
            if let Some(meas) = layer_part.measures.get(m)
                && let Some(ref attrs) = meas.attributes
            {
                collect_staff_clefs(attrs, (layer_idx + 1) as u8, &mut combined_attrs.clefs);
                if combined_attrs.key.is_none() && attrs.key.is_some() {
                    combined_attrs.key.clone_from(&attrs.key);
                }
                if combined_attrs.time.is_none() && attrs.time.is_some() {
                    combined_attrs.time.clone_from(&attrs.time);
                }
                if combined_attrs.multiple_rest.is_none() && attrs.multiple_rest.is_some() {
                    combined_attrs
                        .multiple_rest
                        .clone_from(&attrs.multiple_rest);
                }
            }
        }
    }
    combined_attrs
}

#[derive(Clone, Debug)]
struct VoiceLayerCtx {
    unit_length: abc_parser::Fraction,
    key: ResolvedKey,
    propagation: AccidentalPropagation,
    octave_shift: i8,
    active_slurs: Vec<SlurDescriptor>,
}

struct PartLoweringCtx<'a> {
    defaults: &'a VoiceDefaults,
    active_meter: Option<Meter>,
    active_clef: ClefSpec,
    active_transpose: Option<TransposeSpec>,
    default_layer: VoiceLayerCtx,
    voice_layers: BTreeMap<VoiceLayerKey, VoiceLayerCtx>,
    open_ties: BTreeMap<PitchOctaveKey, (Offset, Option<SourceProvenance>)>,
    all_sounding_durations: Vec<Duration>,
    lowered_measures: Vec<LoweredMeasure>,
    next_tuplet_number: u8,
    slots: Vec<NoteSlotCoord>,
    collected_lyrics: Vec<(LyricsField, usize, usize)>,
    collected_symbols: Vec<(SymbolsField, usize, usize)>,
    warnings: &'a mut Vec<ParseWarning<Span>>,
}

struct MeasureLoweringState<'a> {
    measure_accidentals: MeasureAccidentalState,
    measure_items: Vec<MeasureItem>,
    measure_duration: Duration,
    primary_layer: usize,
    primary_voice_index: u32,
    current_voice_index: u32,
    current_layer_duration: Duration,
    pending_notations: LoweredNotations,
    pending_slurs_start: Vec<SlurDescriptor>,
    pending_slide_stop: bool,
    active_beam_group: Vec<usize>,
    break_next_beam: bool,
    attrs: LoweredAttributes,
    item_mods: &'a [TimedItemMods],
}

impl MeasureLoweringState<'_> {
    fn new(attrs: LoweredAttributes, item_mods: &[TimedItemMods]) -> MeasureLoweringState<'_> {
        MeasureLoweringState {
            measure_accidentals: MeasureAccidentalState::new(),
            measure_items: Vec::new(),
            measure_duration: Duration::ZERO,
            primary_layer: 0,
            primary_voice_index: 1,
            current_voice_index: 1,
            current_layer_duration: Duration::ZERO,
            pending_notations: LoweredNotations::default(),
            pending_slurs_start: Vec::new(),
            pending_slide_stop: false,
            active_beam_group: Vec::new(),
            break_next_beam: true,
            attrs,
            item_mods,
        }
    }

    const fn voice_layer_key(&self) -> VoiceLayerKey {
        VoiceLayerKey::new(
            self.primary_layer,
            self.current_voice_index
                .saturating_sub(self.primary_voice_index),
        )
    }
}

struct ResolvedChordMember {
    resolved: crate::pitch::ResolvedPitch,
    incoming_tie: Option<(Offset, Option<SourceProvenance>)>,
}

#[allow(clippy::too_many_arguments)]
fn build_part_from_segmented(
    part_id: String,
    part_name: String,
    part_abbreviation: Option<String>,
    declaration_provenance: Option<SourceProvenance>,
    decl: Option<&VoiceDeclInfo>,
    defaults: &VoiceDefaults,
    segmented: &[SegmentedMeasure],
    is_part_one: bool,
    pickup_policy: BarDurationPickupPolicy,
    warnings: &mut Vec<ParseWarning<Span>>,
) -> LoweredPart {
    let active_key = defaults.key.0.clone();
    let active_clef = decl
        .and_then(|d| d.clef)
        .or(active_key.parameters.clef)
        .unwrap_or(ClefSpec::TREBLE);
    let active_transpose = decl
        .and_then(|d| d.transpose)
        .or(active_key.parameters.transpose);
    let active_octave_shift = decl
        .and_then(|d| d.octave_shift)
        .or(active_key.parameters.octave_shift)
        .unwrap_or(0);
    let default_layer = VoiceLayerCtx {
        unit_length: defaults.unit_length.0,
        key: active_key,
        propagation: defaults.propagation,
        octave_shift: active_octave_shift,
        active_slurs: Vec::new(),
    };

    let mut ctx = PartLoweringCtx {
        defaults,
        active_meter: defaults.meter.as_ref().map(|(m, _)| m.clone()),
        active_clef,
        active_transpose,
        default_layer,
        voice_layers: BTreeMap::new(),
        open_ties: BTreeMap::new(),
        all_sounding_durations: Vec::new(),
        lowered_measures: Vec::new(),
        next_tuplet_number: 1,
        slots: Vec::new(),
        collected_lyrics: Vec::new(),
        collected_symbols: Vec::new(),
        warnings,
    };

    for (seg_idx, seg) in segmented.iter().enumerate() {
        ctx.lower_segmented_measure(seg_idx, seg, decl, is_part_one);
    }

    ctx.apply_lyrics();
    ctx.apply_symbols();

    if ctx.all_sounding_durations.is_empty() {
        ctx.all_sounding_durations
            .push(Duration::from_fraction(defaults.unit_length.0));
    }

    let (divisions, exceeds_midi) = compute_divisions(ctx.all_sounding_durations);
    if exceeds_midi {
        ctx.warnings.push(ParseWarning {
            kind: ErrorKind::InvalidMusic,
            message: format!(
                "computed MusicXML <divisions> ({divisions}) exceeds the Standard MIDI File limit of 16383"
            ),
            span: Span::from(0..0),
            related: Vec::new(),
        });
    }

    if let Some(first_measure) = ctx.lowered_measures.first_mut() {
        let attrs = first_measure
            .attributes
            .get_or_insert_with(LoweredAttributes::default);
        attrs.divisions = Some((divisions, defaults.unit_length.1.clone()));
    }

    assign_measure_numbers(&mut ctx.lowered_measures, pickup_policy);

    LoweredPart {
        part_id,
        part_name,
        part_abbreviation,
        declaration_provenance,
        divisions,
        measures: ctx.lowered_measures,
        midi_instrument: decl.and_then(|d| d.midi).or(defaults.midi),
    }
}

impl PartLoweringCtx<'_> {
    fn layer(&self, key: VoiceLayerKey) -> &VoiceLayerCtx {
        self.voice_layers.get(&key).unwrap_or(&self.default_layer)
    }

    fn layer_mut(&mut self, key: VoiceLayerKey) -> &mut VoiceLayerCtx {
        let default_layer = &self.default_layer;
        self.voice_layers
            .entry(key)
            .or_insert_with(|| default_layer.clone())
    }

    fn lower_segmented_measure(
        &mut self,
        seg_idx: usize,
        seg: &SegmentedMeasure,
        decl: Option<&VoiceDeclInfo>,
        is_part_one: bool,
    ) {
        let is_first_measure = seg_idx == 0 && self.lowered_measures.is_empty();
        let mut attrs = LoweredAttributes::default();

        if is_first_measure {
            let primary_key = &self.layer(VoiceLayerKey::default()).key;
            attrs.key = Some((primary_key.kind.clone(), self.defaults.key.1.clone()));
            if let Some((meter, prov)) = &self.defaults.meter {
                attrs.time = Some((meter.clone(), prov.clone()));
            }
            let clef_prov = decl
                .and_then(|d| d.provenance.clone())
                .or_else(|| self.defaults.key.1.clone());
            attrs.clef = Some((self.active_clef, clef_prov.clone()));
            if let Some(trans) = self.active_transpose {
                attrs.transpose = Some((trans, clef_prov));
            }
        }

        if let Some((mm_rest, l_span, e_span)) = seg.items.iter().find_map(|item| match item.kind {
            RawMeasureItemKind::MultiMeasureRest(mm) => {
                Some((mm, item.line_span, item.element_span))
            }
            _ => None,
        }) {
            let mut state = MeasureLoweringState::new(attrs, &[]);
            if is_part_one
                && is_first_measure
                && let Some((ref tempo, ref prov)) = self.defaults.tempo
            {
                let dir = tempo_to_direction(tempo, Some(prov));
                state.measure_items.push(MeasureItem::Direction(dir));
            }
            for (idx, item) in seg.items.iter().enumerate() {
                if matches!(
                    item.kind,
                    RawMeasureItemKind::Meter(..)
                        | RawMeasureItemKind::UnitLength(..)
                        | RawMeasureItemKind::Key(..)
                        | RawMeasureItemKind::VoiceParams(..)
                        | RawMeasureItemKind::Propagation(..)
                        | RawMeasureItemKind::Tempo(..)
                        | RawMeasureItemKind::MergedVoice(..)
                ) {
                    self.lower_measure_item(idx, item, &mut state);
                }
            }
            self.push_multi_measure_rest(
                seg,
                mm_rest,
                l_span,
                e_span,
                state.attrs,
                state.measure_items,
            );
            return;
        }

        let item_mods = prepass_measure_mods(
            &seg.items,
            self.active_meter.as_ref(),
            &mut self.next_tuplet_number,
        );

        let mut state = MeasureLoweringState::new(attrs, &item_mods);
        if is_part_one
            && is_first_measure
            && let Some((ref tempo, ref prov)) = self.defaults.tempo
        {
            let dir = tempo_to_direction(tempo, Some(prov));
            state.measure_items.push(MeasureItem::Direction(dir));
        }

        for (idx, item) in seg.items.iter().enumerate() {
            self.lower_measure_item(idx, item, &mut state);
        }

        finish_beam_group(&state.active_beam_group, &mut state.measure_items);
        state.measure_duration = state.measure_duration.max(state.current_layer_duration);
        let expected_duration = self.active_meter.as_ref().and_then(meter_bar_duration);

        let measure_prov = seg.first_line_span.map(SourceProvenance::from_line);
        self.lowered_measures.push(LoweredMeasure {
            number: String::new(),
            implicit: false,
            provenance: measure_prov,
            left_barline: seg.left_barline.clone(),
            attributes: (!state.attrs.is_empty()).then_some(state.attrs),
            items: state.measure_items,
            right_barline: seg.right_barline.clone(),
            actual_duration: state.measure_duration,
            expected_duration,
            new_system: seg.new_system,
            new_page: seg.new_page,
        });
    }

    fn push_multi_measure_rest(
        &mut self,
        seg: &SegmentedMeasure,
        mm_rest: MultiMeasureRest,
        l_span: Span,
        e_span: Span,
        mut attrs: LoweredAttributes,
        leading_items: Vec<MeasureItem>,
    ) {
        let bar_dur = self
            .active_meter
            .as_ref()
            .and_then(meter_bar_duration)
            .unwrap_or(Duration::ONE);
        self.all_sounding_durations.push(bar_dur);
        let count = mm_rest.measures.max(1);
        let elem_prov = SourceProvenance::new(l_span, e_span);
        let measure_prov = seg.first_line_span.map(SourceProvenance::from_line);

        if count > 1 {
            attrs.multiple_rest = Some((count, Some(elem_prov.clone())));
        }

        let mut leading = Some(leading_items);
        for rest_idx in 0..count {
            let m_attrs = (rest_idx == 0 && !attrs.is_empty()).then(|| attrs.clone());
            let left_bar = (rest_idx == 0).then(|| seg.left_barline.clone()).flatten();
            let right_bar = (rest_idx + 1 == count)
                .then(|| seg.right_barline.clone())
                .flatten();
            let mut items = leading.take().unwrap_or_default();
            items.push(MeasureItem::Note(LoweredNote::measure_rest(
                !mm_rest.invisible,
                bar_dur,
                1,
                None,
                Some(elem_prov.clone()),
            )));
            self.lowered_measures.push(LoweredMeasure {
                number: String::new(),
                implicit: false,
                provenance: (rest_idx == 0).then(|| measure_prov.clone()).flatten(),
                left_barline: left_bar,
                attributes: m_attrs,
                items,
                right_barline: right_bar,
                actual_duration: bar_dur,
                expected_duration: Some(bar_dur),
                new_system: rest_idx == 0 && seg.new_system,
                new_page: rest_idx == 0 && seg.new_page,
            });
        }
    }

    fn lower_measure_item(
        &mut self,
        idx: usize,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
    ) {
        let voice_key = state.voice_layer_key();
        match &item.kind {
            RawMeasureItemKind::Meter(meter, prov) => {
                self.active_meter = Some(meter.clone());
                if state.current_voice_index == 1 {
                    state.attrs.time = Some((meter.clone(), Some(prov.clone())));
                }
            }
            RawMeasureItemKind::UnitLength(unit) => {
                self.layer_mut(voice_key).unit_length = *unit;
            }
            RawMeasureItemKind::Key(resolved, prov) => {
                self.layer_mut(voice_key).key = resolved.clone();
                if state.current_voice_index == 1 {
                    state.attrs.key = Some((resolved.kind.clone(), Some(prov.clone())));
                    if let Some(clef) = resolved.parameters.clef {
                        self.active_clef = clef;
                        state.attrs.clef = Some((clef, Some(prov.clone())));
                    }
                    if let Some(trans) = resolved.parameters.transpose {
                        self.active_transpose = Some(trans);
                        state.attrs.transpose = Some((trans, Some(prov.clone())));
                    }
                }
                if let Some(oct) = resolved.parameters.octave_shift {
                    self.layer_mut(voice_key).octave_shift = oct;
                }
            }
            RawMeasureItemKind::VoiceParams(info) => {
                if state.current_voice_index == 1 {
                    if let Some(clef) = info.clef {
                        self.active_clef = clef;
                        state.attrs.clef = Some((clef, info.provenance.clone()));
                    }
                    if let Some(trans) = info.transpose {
                        self.active_transpose = Some(trans);
                        state.attrs.transpose = Some((trans, info.provenance.clone()));
                    }
                }
                if let Some(oct) = info.octave_shift {
                    self.layer_mut(voice_key).octave_shift = oct;
                }
            }
            RawMeasureItemKind::Propagation(prop) => self.layer_mut(voice_key).propagation = *prop,
            RawMeasureItemKind::Tempo(tempo, prov) => {
                let dir = tempo_to_direction(tempo, Some(prov));
                state.measure_items.push(MeasureItem::Direction(dir));
            }
            RawMeasureItemKind::Annotation(ann, prov) => {
                let lowered =
                    lower_annotation(ann.placement, &ann.text, prov.clone(), self.warnings);
                state.measure_items.push(lowered);
            }
            RawMeasureItemKind::Decoration(dec, prov) => {
                if let Some(dir) = apply_decoration_or_direction(
                    &dec.name,
                    Some(prov.clone()),
                    &mut state.pending_notations,
                ) {
                    state.measure_items.push(MeasureItem::Direction(dir));
                }
            }
            RawMeasureItemKind::Slur(slur, prov) => self.apply_slur_item(*slur, prov, state),
            RawMeasureItemKind::BeamBreak => {
                finish_beam_group(&state.active_beam_group, &mut state.measure_items);
                state.active_beam_group.clear();
                state.break_next_beam = true;
            }
            RawMeasureItemKind::BeamContinuation => state.break_next_beam = false,
            RawMeasureItemKind::Grace(grace) => self.lower_grace_item(grace, item, state),
            RawMeasureItemKind::Overlay(Overlay::NextVoice) => {
                self.apply_overlay_next_voice(item, state, None);
            }
            RawMeasureItemKind::MergedVoice(layer) => {
                self.apply_overlay_next_voice(item, state, Some(*layer));
            }
            RawMeasureItemKind::Lyrics(field) => {
                let cutoff = self.layer_slot_count(state.primary_layer);
                self.collected_lyrics
                    .push((field.clone(), cutoff, state.primary_layer));
            }
            RawMeasureItemKind::Symbols(field) => {
                let cutoff = self.layer_slot_count(state.primary_layer);
                self.collected_symbols
                    .push((field.clone(), cutoff, state.primary_layer));
            }
            RawMeasureItemKind::Note(note) => {
                self.lower_note_item(note, &state.item_mods[idx], item, state);
            }
            RawMeasureItemKind::Rest(rest) => {
                self.lower_rest_item(*rest, &state.item_mods[idx], item, state);
            }
            RawMeasureItemKind::Chord(chord) => {
                self.lower_chord_item(chord, &state.item_mods[idx], item, state);
            }
            _ => {}
        }
    }

    fn layer_slot_count(&self, layer: usize) -> usize {
        self.slots.iter().filter(|s| s.layer == layer).count()
    }

    fn apply_slur_item(
        &mut self,
        slur: abc_parser::Slur,
        prov: &SourceProvenance,
        state: &mut MeasureLoweringState<'_>,
    ) {
        let voice_key = state.voice_layer_key();
        if slur.opening {
            let n = {
                let active_slurs = &self.layer(voice_key).active_slurs;
                (1..=16)
                    .find(|&s| {
                        !active_slurs.iter().any(|d| d.number == s)
                            && !state.pending_slurs_start.iter().any(|d| d.number == s)
                    })
                    .unwrap_or(1)
            };
            let desc = SlurDescriptor::new(n, slur.dotted, Some(prov.clone()));
            state.pending_slurs_start.push(desc.clone());
            self.layer_mut(voice_key).active_slurs.push(desc);
        } else if let Some(mut active) = self.layer_mut(voice_key).active_slurs.pop() {
            active.provenance = Some(prov.clone());
            attach_slur_stop(
                &mut state.measure_items,
                &mut self.lowered_measures,
                voice_key,
                &active,
            );
        }
    }

    fn apply_overlay_next_voice(
        &mut self,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
        merged_layer: Option<usize>,
    ) {
        finish_beam_group(&state.active_beam_group, &mut state.measure_items);
        state.active_beam_group.clear();
        state.break_next_beam = true;
        if !state.current_layer_duration.is_zero() {
            self.all_sounding_durations
                .push(state.current_layer_duration);
            state.measure_items.push(MeasureItem::Backup {
                duration: state.current_layer_duration,
                provenance: Some(SourceProvenance::new(item.line_span, item.element_span)),
            });
        }
        state.measure_duration = state.measure_duration.max(state.current_layer_duration);
        state.current_layer_duration = Duration::ZERO;
        state.current_voice_index += 1;
        if let Some(layer) = merged_layer {
            state.primary_layer = layer;
            state.primary_voice_index = state.current_voice_index;
        }
    }

    fn lower_grace_item(
        &self,
        grace: &abc_parser::GraceGroup,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
    ) {
        let voice_key = state.voice_layer_key();
        let layer = self.layer(voice_key);
        let mut grace_notes = lower_grace_group(
            grace,
            layer.unit_length,
            layer.octave_shift,
            &layer.key,
            &self.active_clef,
            layer.propagation,
            &mut state.measure_accidentals,
            state.current_voice_index,
            voice_key,
            item.line_span,
            item.element_span,
        );
        if let Some(first_g) = grace_notes.first_mut() {
            let nots = first_g.notations_mut();
            for desc in std::mem::take(&mut state.pending_slurs_start) {
                nots.slurs.push(LoweredSlur {
                    slur_type: SpannerEndpoint::Start,
                    number: desc.number,
                    dotted: desc.dotted,
                    provenance: desc.provenance,
                });
            }
            nots.merge(std::mem::take(&mut state.pending_notations));
            if nots.is_empty() {
                first_g.notations = None;
            }
        }
        for g in grace_notes {
            state.measure_items.push(MeasureItem::Note(g));
        }
    }

    fn resolve_pitched_note(
        &mut self,
        pitch: Pitch,
        mods: &TimedItemMods,
        state: &mut MeasureLoweringState<'_>,
    ) -> ResolvedChordMember {
        let voice_key = state.voice_layer_key();
        let (octave_shift, key_offsets, propagation) = {
            let layer = self.layer(voice_key);
            (layer.octave_shift, layer.key.offsets, layer.propagation)
        };
        let pitch_key = PitchOctaveKey::new(voice_key, class_index(pitch.class), pitch.octave);
        let incoming_tie = self.open_ties.remove(&pitch_key);
        let resolved = state.measure_accidentals.resolve_pitch(
            pitch,
            octave_shift,
            &key_offsets,
            propagation,
            incoming_tie.as_ref().map(|(alt, _)| *alt),
        );
        if let Some((_, ref tie_prov)) = mods.tie_after {
            self.open_ties
                .insert(pitch_key, (resolved.alter, Some(tie_prov.clone())));
        }
        ResolvedChordMember {
            resolved,
            incoming_tie,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_pitched_segment(
        &mut self,
        info: &ResolvedChordMember,
        is_chord_member: bool,
        seg_idx: usize,
        seg_count: usize,
        seg_part: crate::timing::GraphicSegment,
        mods: &TimedItemMods,
        stem_str: Option<&'static str>,
        elem_prov: &SourceProvenance,
        state: &mut MeasureLoweringState<'_>,
    ) -> usize {
        let is_first_seg = seg_idx == 0;
        let is_last_seg = seg_idx + 1 == seg_count;
        let tie_stop = !is_first_seg || info.incoming_tie.is_some();
        let tie_start = !is_last_seg || mods.tie_after.is_some();
        let tie_dotted = is_last_seg && mods.tie_after.as_ref().is_some_and(|(t, _)| t.dotted);
        let tie_prov = if is_last_seg {
            mods.tie_after
                .as_ref()
                .map(|(_, p)| p.clone())
                .or_else(|| info.incoming_tie.as_ref().and_then(|(_, p)| p.clone()))
        } else if is_first_seg {
            info.incoming_tie.as_ref().and_then(|(_, p)| p.clone())
        } else {
            None
        };

        let mut notations = build_segment_notations(
            tie_stop,
            tie_start,
            tie_dotted,
            tie_prov,
            (!is_chord_member && is_first_seg)
                .then_some(&mods.tuplet_start)
                .and_then(|o| o.as_ref()),
            (!is_chord_member && is_last_seg)
                .then_some(&mods.tuplet_stop)
                .and_then(|o| o.as_ref()),
        );
        if !is_chord_member && is_first_seg {
            attach_first_note_pending(&mut notations, state);
        }

        let note_idx = state.measure_items.len();
        if !is_chord_member
            && is_first_seg
            && state.current_voice_index == state.primary_voice_index
        {
            self.slots.push(NoteSlotCoord {
                layer: state.primary_layer,
                measure_idx: self.lowered_measures.len(),
                item_idx: note_idx,
            });
        }

        let kind = build_note_kind(&self.active_clef, info.resolved, is_first_seg);
        let lowered_note = LoweredNote {
            print_object: true,
            grace: None,
            is_chord_member,
            kind,
            sounding_duration: seg_part.sounding_duration,
            voice: state.current_voice_index,
            voice_key: state.voice_layer_key(),
            staff: None,
            note_type: Some(seg_part.note_type.as_str()),
            dots: seg_part.dots,
            time_modification: mods.tuplet_mod,
            stem: stem_str,
            beams: Vec::new(),
            notations: LoweredNote::from_notations(notations),
            lyrics: Vec::new(),
            provenance: Some(elem_prov.clone()),
        };
        state.measure_items.push(MeasureItem::Note(lowered_note));
        note_idx
    }

    fn lower_note_item(
        &mut self,
        note: &Note,
        mods: &TimedItemMods,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
    ) {
        let voice_key = state.voice_layer_key();
        let (unit_length, stem_policy) = {
            let layer = self.layer(voice_key);
            (layer.unit_length, layer.key.stem_policy)
        };
        let base_graphic = Duration::from_fraction(unit_length)
            .multiply(Duration::from_note_length(note.length))
            .multiply(mods.broken_factor);
        let total_sounding = base_graphic.multiply(mods.tuplet_factor);
        state.current_layer_duration = state.current_layer_duration.add(total_sounding);

        let segments = decompose_graphic_duration(base_graphic, total_sounding);
        let seg_count = segments.len();
        let elem_prov = SourceProvenance::new(item.line_span, item.element_span);
        let stem_str = match stem_policy {
            StemPolicy::Auto => None,
            StemPolicy::Bagpipe => Some("down"),
        };

        let resolved = self.resolve_pitched_note(note.pitch, mods, state);

        for (seg_idx, seg_part) in segments.into_iter().enumerate() {
            self.all_sounding_durations.push(seg_part.sounding_duration);
            let note_idx = self.emit_pitched_segment(
                &resolved, false, seg_idx, seg_count, seg_part, mods, stem_str, &elem_prov, state,
            );
            if seg_idx == 0 {
                update_beam_group(note_idx, state);
            }
        }
    }

    fn lower_rest_item(
        &mut self,
        rest: Rest,
        mods: &TimedItemMods,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
    ) {
        finish_beam_group(&state.active_beam_group, &mut state.measure_items);
        state.active_beam_group.clear();
        state.break_next_beam = true;

        let unit_length = self.layer(state.voice_layer_key()).unit_length;
        let base_graphic = Duration::from_fraction(unit_length)
            .multiply(Duration::from_note_length(rest.length))
            .multiply(mods.broken_factor);
        let total_sounding = base_graphic.multiply(mods.tuplet_factor);
        state.current_layer_duration = state.current_layer_duration.add(total_sounding);

        let segments = decompose_graphic_duration(base_graphic, total_sounding);
        let elem_prov = SourceProvenance::new(item.line_span, item.element_span);

        for (seg_idx, seg_part) in segments.into_iter().enumerate() {
            self.all_sounding_durations.push(seg_part.sounding_duration);
            let mut notations = LoweredNotations::default();
            if seg_idx == 0 && state.pending_notations.fermata.is_some() {
                notations.fermata = state.pending_notations.fermata.take();
            }
            state.pending_notations = LoweredNotations::default();

            let mut lowered = LoweredNote::timed_rest(
                rest.kind == RestKind::Visible,
                seg_part.sounding_duration,
                state.current_voice_index,
                Some(seg_part.note_type.as_str()),
                seg_part.dots,
                mods.tuplet_mod,
                Some(elem_prov.clone()),
            );
            lowered.voice_key = state.voice_layer_key();
            lowered.notations = LoweredNote::from_notations(notations);
            state.measure_items.push(MeasureItem::Note(lowered));
        }
    }

    fn resolve_chord_members(
        &mut self,
        chord: &Chord,
        mods: &TimedItemMods,
        state: &mut MeasureLoweringState<'_>,
    ) -> Vec<Option<ResolvedChordMember>> {
        chord
            .members
            .iter()
            .map(|member| match member {
                ChordMember::Note(note) => Some(self.resolve_pitched_note(note.pitch, mods, state)),
                ChordMember::Rest(_) => None,
            })
            .collect()
    }

    fn lower_chord_item(
        &mut self,
        chord: &Chord,
        mods: &TimedItemMods,
        item: &RawMeasureItem,
        state: &mut MeasureLoweringState<'_>,
    ) {
        let elem_prov = SourceProvenance::new(item.line_span, item.element_span);
        let voice_key = state.voice_layer_key();
        let (unit_length, stem_policy) = {
            let layer = self.layer(voice_key);
            (layer.unit_length, layer.key.stem_policy)
        };
        let stem_str = match stem_policy {
            StemPolicy::Auto => None,
            StemPolicy::Bagpipe => Some("down"),
        };

        let first_member_len = chord.members.first().map_or(
            NoteLength {
                numerator: 1,
                denominator: 1,
            },
            |m| match m {
                ChordMember::Note(n) => n.length,
                ChordMember::Rest(r) => r.length,
            },
        );
        let chord_graphic = Duration::from_fraction(unit_length)
            .multiply(Duration::from_note_length(first_member_len))
            .multiply(Duration::from_note_length(chord.length))
            .multiply(mods.broken_factor);
        let chord_sounding = chord_graphic.multiply(mods.tuplet_factor);
        state.current_layer_duration = state.current_layer_duration.add(chord_sounding);

        let chord_segments = decompose_graphic_duration(chord_graphic, chord_sounding);
        let seg_count = chord_segments.len().max(1);
        let resolved_members = self.resolve_chord_members(chord, mods, state);

        for (seg_idx, lead_seg) in chord_segments.into_iter().enumerate() {
            let mut first_note_idx = None;
            for (member_idx, (member, resolved_opt)) in
                chord.members.iter().zip(&resolved_members).enumerate()
            {
                let member_len = match member {
                    ChordMember::Note(n) => n.length,
                    ChordMember::Rest(r) => r.length,
                };
                let member_seg = if seg_count == 1 && member_len != first_member_len {
                    let member_graphic = Duration::from_fraction(unit_length)
                        .multiply(Duration::from_note_length(member_len))
                        .multiply(Duration::from_note_length(chord.length))
                        .multiply(mods.broken_factor);
                    let member_sounding = member_graphic.multiply(mods.tuplet_factor);
                    decompose_graphic_duration(member_graphic, member_sounding)
                        .into_iter()
                        .next()
                        .unwrap_or(lead_seg)
                } else {
                    lead_seg
                };
                self.all_sounding_durations
                    .push(member_seg.sounding_duration);

                self.emit_chord_member_segment(
                    member,
                    resolved_opt.as_ref(),
                    member_idx > 0,
                    seg_idx,
                    seg_count,
                    member_seg,
                    mods,
                    stem_str,
                    &elem_prov,
                    state,
                    &mut first_note_idx,
                );
            }
            if seg_idx == 0
                && let Some(note_idx) = first_note_idx
            {
                update_beam_group(note_idx, state);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_chord_member_segment(
        &mut self,
        member: &ChordMember,
        resolved_opt: Option<&ResolvedChordMember>,
        is_chord_member: bool,
        seg_idx: usize,
        seg_count: usize,
        member_seg: crate::timing::GraphicSegment,
        mods: &TimedItemMods,
        stem_str: Option<&'static str>,
        elem_prov: &SourceProvenance,
        state: &mut MeasureLoweringState<'_>,
        first_note_idx: &mut Option<usize>,
    ) {
        match (member, resolved_opt) {
            (ChordMember::Note(_), Some(info)) => {
                let note_idx = self.emit_pitched_segment(
                    info,
                    is_chord_member,
                    seg_idx,
                    seg_count,
                    member_seg,
                    mods,
                    stem_str,
                    elem_prov,
                    state,
                );
                if !is_chord_member {
                    *first_note_idx = Some(note_idx);
                }
            }
            (ChordMember::Rest(rest), _) => {
                let mut lowered = LoweredNote::timed_rest(
                    rest.kind == RestKind::Visible,
                    member_seg.sounding_duration,
                    state.current_voice_index,
                    Some(member_seg.note_type.as_str()),
                    member_seg.dots,
                    mods.tuplet_mod,
                    Some(elem_prov.clone()),
                );
                lowered.voice_key = state.voice_layer_key();
                lowered.is_chord_member = is_chord_member;
                state.measure_items.push(MeasureItem::Note(lowered));
            }
            _ => {}
        }
    }

    fn apply_lyrics(&mut self) {
        let mut primary_layers: Vec<usize> = self
            .collected_lyrics
            .iter()
            .map(|(_, _, layer)| *layer)
            .collect();
        primary_layers.sort_unstable();
        primary_layers.dedup();

        for primary_layer in primary_layers {
            let (layer_slots, slot_bars) = collect_layer_slots(&self.slots, primary_layer);
            let mut verses: Vec<u32> = self
                .collected_lyrics
                .iter()
                .filter(|(_, _, layer)| *layer == primary_layer)
                .map(|(f, _, _)| f.verse)
                .collect();
            verses.sort_unstable();
            verses.dedup();

            for verse in verses {
                let mut cursor = 0;
                for (field, note_cutoff, _) in self
                    .collected_lyrics
                    .iter()
                    .filter(|(f, _, layer)| *layer == primary_layer && f.verse == verse)
                {
                    let steps: Vec<Step> = field
                        .tokens
                        .iter()
                        .map(|t| match t.kind {
                            LyricKind::Bar => Step::Bar,
                            _ => Step::Note,
                        })
                        .collect();
                    let pairs = align(&steps, &slot_bars, cursor);
                    let mut extend_remaining = 0_usize;
                    for (slot_idx, token_idx) in &pairs {
                        let token = &field.tokens[*token_idx];
                        let prov = SourceProvenance::new(
                            field.line_span,
                            Span::from(token.span.start..token.span.end),
                        );
                        let coord = layer_slots[*slot_idx];
                        match &token.kind {
                            LyricKind::Syllable { text, syllabic } => {
                                let mut ext_count = 0;
                                for next_t in &field.tokens[token_idx + 1..] {
                                    if matches!(next_t.kind, LyricKind::Extend) {
                                        ext_count += 1;
                                    } else if !matches!(next_t.kind, LyricKind::Bar) {
                                        break;
                                    }
                                }
                                extend_remaining = ext_count;
                                let extend = (ext_count > 0).then_some(SpannerEndpoint::Start);
                                if let MeasureItem::Note(ref mut note) =
                                    self.lowered_measures[coord.measure_idx].items[coord.item_idx]
                                {
                                    note.lyrics.push(LoweredLyric {
                                        number: field.verse,
                                        syllabic: Some(*syllabic),
                                        text: Some(text.clone()),
                                        extend,
                                        provenance: Some(prov),
                                    });
                                }
                                if let Some(ref mut m_prov) =
                                    self.lowered_measures[coord.measure_idx].provenance
                                {
                                    m_prov.add_extra_line_span(field.line_span);
                                }
                            }
                            LyricKind::Extend => {
                                let extend = if extend_remaining <= 1 {
                                    extend_remaining = 0;
                                    Some(SpannerEndpoint::Stop)
                                } else {
                                    extend_remaining = extend_remaining.saturating_sub(1);
                                    Some(SpannerEndpoint::Continue)
                                };
                                if let MeasureItem::Note(ref mut note) =
                                    self.lowered_measures[coord.measure_idx].items[coord.item_idx]
                                {
                                    note.lyrics.push(LoweredLyric {
                                        number: field.verse,
                                        syllabic: None,
                                        text: None,
                                        extend,
                                        provenance: Some(prov),
                                    });
                                }
                            }
                            LyricKind::Skip | LyricKind::Hold | LyricKind::Bar => {}
                        }
                    }
                    let next_cursor = pairs.last().map_or(cursor, |(s, _)| s + 1);
                    cursor = next_cursor.max(*note_cutoff);
                }
            }
        }
    }

    fn apply_symbols(&mut self) {
        if self.collected_symbols.is_empty() {
            return;
        }
        let mut primary_layers: Vec<usize> = self
            .collected_symbols
            .iter()
            .map(|(_, _, layer)| *layer)
            .collect();
        primary_layers.sort_unstable();
        primary_layers.dedup();
        let mut symbol_insertions: Vec<(usize, usize, MeasureItem)> = Vec::new();

        for primary_layer in primary_layers {
            let (layer_slots, slot_bars) = collect_layer_slots(&self.slots, primary_layer);
            let mut sym_layers: Vec<usize> = self
                .collected_symbols
                .iter()
                .filter(|(_, _, layer)| *layer == primary_layer)
                .map(|(f, _, _)| f.layer)
                .collect();
            sym_layers.sort_unstable();
            sym_layers.dedup();

            for layer in sym_layers {
                let mut cursor = 0;
                for (field, note_cutoff, _) in self
                    .collected_symbols
                    .iter()
                    .filter(|(f, _, p_layer)| *p_layer == primary_layer && f.layer == layer)
                {
                    let steps: Vec<Step> = field
                        .tokens
                        .iter()
                        .map(|t| match t.kind {
                            SymbolKind::Bar => Step::Bar,
                            _ => Step::Note,
                        })
                        .collect();
                    let pairs = align(&steps, &slot_bars, cursor);
                    for (slot_idx, token_idx) in &pairs {
                        let token = &field.tokens[*token_idx];
                        if let SymbolKind::Element(ref text) = token.kind {
                            let prov = SourceProvenance::new(
                                field.line_span,
                                Span::from(token.span.start..token.span.end),
                            );
                            let coord = layer_slots[*slot_idx];
                            lower_symbol_element(
                                text,
                                prov,
                                coord.measure_idx,
                                coord.item_idx,
                                &mut self.lowered_measures,
                                &mut symbol_insertions,
                                self.warnings,
                            );
                            if let Some(ref mut m_prov) =
                                self.lowered_measures[coord.measure_idx].provenance
                            {
                                m_prov.add_extra_line_span(field.line_span);
                            }
                        }
                    }
                    let next_cursor = pairs.last().map_or(cursor, |(s, _)| s + 1);
                    cursor = next_cursor.max(*note_cutoff);
                }
            }
        }

        symbol_insertions.sort_by(|(m1, i1, _), (m2, i2, _)| m1.cmp(m2).then_with(|| i2.cmp(i1)));
        for (m_idx, i_idx, item) in symbol_insertions {
            self.lowered_measures[m_idx].items.insert(i_idx, item);
        }
    }
}

fn collect_layer_slots(slots: &[NoteSlotCoord], layer: usize) -> (Vec<&NoteSlotCoord>, Vec<usize>) {
    let mut layer_slots = Vec::new();
    let mut slot_bars = Vec::new();
    for slot in slots {
        if slot.layer == layer {
            layer_slots.push(slot);
            slot_bars.push(slot.measure_idx);
        }
    }
    (layer_slots, slot_bars)
}

fn prepass_measure_mods(
    items: &[RawMeasureItem],
    active_meter: Option<&Meter>,
    next_tuplet_number: &mut u8,
) -> Vec<TimedItemMods> {
    let mut item_mods = vec![TimedItemMods::default(); items.len()];
    let mut active_tuplets: Vec<ActiveTupletState> = Vec::new();
    let mut next_broken_factor = Duration::ONE;
    let mut last_timed_idx: Option<usize> = None;
    let mut current_meter = active_meter;

    for (idx, item) in items.iter().enumerate() {
        match &item.kind {
            RawMeasureItemKind::Meter(meter, _) => current_meter = Some(meter),
            RawMeasureItemKind::Overlay(_) | RawMeasureItemKind::MergedVoice(_) => {
                last_timed_idx = None;
            }
            RawMeasureItemKind::Tuplet(tuplet) => {
                let compound = current_meter.is_some_and(is_compound_meter);
                if let Some(resolved) = resolve_tuplet(*tuplet, compound) {
                    let num = (*next_tuplet_number).clamp(1, 6);
                    *next_tuplet_number = (*next_tuplet_number % 6) + 1;
                    active_tuplets.push(ActiveTupletState {
                        resolved,
                        remaining: resolved.affected,
                        started: false,
                        number: num,
                        provenance: SourceProvenance::new(item.line_span, item.element_span),
                    });
                }
            }
            RawMeasureItemKind::BrokenRhythm(rhythm) => {
                if let Some((first_factor, second_factor)) = broken_rhythm_factors(*rhythm)
                    && let Some(prev_idx) = last_timed_idx
                {
                    item_mods[prev_idx].broken_factor =
                        item_mods[prev_idx].broken_factor.multiply(first_factor);
                    next_broken_factor = second_factor;
                }
            }
            RawMeasureItemKind::Tie(tie) => {
                if let Some(prev_idx) = last_timed_idx {
                    item_mods[prev_idx].tie_after = Some((
                        *tie,
                        SourceProvenance::new(item.line_span, item.element_span),
                    ));
                }
            }
            RawMeasureItemKind::Note(_)
            | RawMeasureItemKind::Rest(_)
            | RawMeasureItemKind::Chord(_) => {
                last_timed_idx = Some(idx);
                item_mods[idx].broken_factor =
                    std::mem::replace(&mut next_broken_factor, Duration::ONE);
                for tup in &mut active_tuplets {
                    item_mods[idx].tuplet_factor =
                        item_mods[idx].tuplet_factor.multiply(tup.resolved.factor);
                    item_mods[idx].tuplet_mod = Some((tup.resolved.actual, tup.resolved.normal));
                    if !tup.started {
                        tup.started = true;
                        item_mods[idx].tuplet_start = Some((tup.number, tup.provenance.clone()));
                    }
                    tup.remaining = tup.remaining.saturating_sub(1);
                    if tup.remaining == 0 {
                        item_mods[idx].tuplet_stop = Some((tup.number, tup.provenance.clone()));
                    }
                }
                active_tuplets.retain(|t| t.remaining > 0);
            }
            _ => {}
        }
    }
    item_mods
}

fn build_segment_notations(
    tied_stop: bool,
    tied_start: bool,
    tied_dotted: bool,
    tie_provenance: Option<SourceProvenance>,
    tuplet_start: Option<&(u8, SourceProvenance)>,
    tuplet_stop: Option<&(u8, SourceProvenance)>,
) -> LoweredNotations {
    LoweredNotations {
        tied_stop,
        tied_start,
        tied_dotted,
        tie_provenance,
        slurs: Vec::new(),
        tuplets_start: tuplet_start.map_or_else(Vec::new, |(n, p)| vec![(*n, Some(p.clone()))]),
        tuplets_stop: tuplet_stop.map_or_else(Vec::new, |(n, p)| vec![(*n, Some(p.clone()))]),
        articulations: Vec::new(),
        ornaments: Vec::new(),
        technical: Vec::new(),
        fermata: None,
        arpeggiate: false,
        slide_start: false,
        slide_stop: false,
        other: Vec::new(),
    }
}

fn attach_first_note_pending(
    notations: &mut LoweredNotations,
    state: &mut MeasureLoweringState<'_>,
) {
    for desc in std::mem::take(&mut state.pending_slurs_start) {
        notations.slurs.push(LoweredSlur {
            slur_type: SpannerEndpoint::Start,
            number: desc.number,
            dotted: desc.dotted,
            provenance: desc.provenance,
        });
    }
    notations.merge(std::mem::take(&mut state.pending_notations));
    if state.pending_slide_stop {
        notations.slide_stop = true;
        state.pending_slide_stop = false;
    }
    if notations.slide_start {
        state.pending_slide_stop = true;
    }
}

fn update_beam_group(note_idx: usize, state: &mut MeasureLoweringState<'_>) {
    let beam_count = note_beam_level_count(match &state.measure_items[note_idx] {
        MeasureItem::Note(n) => n,
        _ => unreachable!(),
    });
    if beam_count > 0 {
        if state.break_next_beam {
            finish_beam_group(&state.active_beam_group, &mut state.measure_items);
            state.active_beam_group.clear();
            state.break_next_beam = false;
        }
        state.active_beam_group.push(note_idx);
    } else {
        finish_beam_group(&state.active_beam_group, &mut state.measure_items);
        state.active_beam_group.clear();
        state.break_next_beam = true;
    }
}

fn assign_measure_numbers(measures: &mut [LoweredMeasure], pickup_policy: BarDurationPickupPolicy) {
    if measures.is_empty() {
        return;
    }

    let first_is_pickup = match pickup_policy {
        BarDurationPickupPolicy::OpeningBar => {
            let first = &measures[0];
            first.expected_duration.is_some_and(|expected| {
                !first.actual_duration.is_zero() && first.actual_duration.is_shorter_than(expected)
            })
        }
        BarDurationPickupPolicy::FirstAndLast if measures.len() >= 2 => {
            let first = &measures[0];
            let last = &measures[measures.len() - 1];
            first.expected_duration.is_some_and(|expected| {
                last.expected_duration == Some(expected)
                    && !first.actual_duration.is_zero()
                    && !last.actual_duration.is_zero()
                    && first.actual_duration.add(last.actual_duration) == expected
            })
        }
        _ => false,
    };

    for (idx, measure) in measures.iter_mut().enumerate() {
        if idx == 0 && first_is_pickup {
            "0".clone_into(&mut measure.number);
            measure.implicit = true;
        } else {
            let num = if first_is_pickup { idx } else { idx + 1 };
            measure.number = num.to_string();
            measure.implicit = false;
        }
    }
}

fn synchronize_part_measure_counts(parts: &mut [LoweredPart], defaults: &VoiceDefaults) {
    let max_len = parts.iter().map(|p| p.measures.len()).max().unwrap_or(0);
    if max_len <= 1 {
        return;
    }
    let default_bar_dur = defaults
        .meter
        .as_ref()
        .and_then(|(m, _)| meter_bar_duration(m))
        .unwrap_or(Duration::ONE);

    let reference_numbers: Vec<(String, bool, Option<Duration>)> = parts
        .iter()
        .find(|p| p.measures.len() == max_len)
        .map(|p| {
            p.measures
                .iter()
                .map(|m| (m.number.clone(), m.implicit, m.expected_duration))
                .collect()
        })
        .unwrap_or_default();

    for part in parts.iter_mut() {
        let mut updated_divisions = part.divisions;
        while part.measures.len() < max_len {
            let idx = part.measures.len();
            let (number, implicit, expected) = reference_numbers
                .get(idx)
                .cloned()
                .unwrap_or_else(|| ((idx + 1).to_string(), false, Some(default_bar_dur)));
            let bar_dur = expected.unwrap_or(default_bar_dur);
            if !bar_dur.is_zero() {
                let q_den = bar_dur.quarter_note_multiple().denominator();
                updated_divisions = lcm(updated_divisions, q_den);
            }
            part.measures.push(LoweredMeasure {
                number,
                implicit,
                provenance: None,
                left_barline: None,
                attributes: None,
                items: vec![MeasureItem::Note(LoweredNote::measure_rest(
                    false, bar_dur, 1, None, None,
                ))],
                right_barline: None,
                actual_duration: bar_dur,
                expected_duration: Some(bar_dur),
                new_system: false,
                new_page: false,
            });
        }
        if updated_divisions != part.divisions {
            part.divisions = updated_divisions;
            if let Some(first_measure) = part.measures.first_mut()
                && let Some(ref mut attrs) = first_measure.attributes
                && let Some((ref mut divs, _)) = attrs.divisions
            {
                *divs = updated_divisions;
            }
        }
    }
}
