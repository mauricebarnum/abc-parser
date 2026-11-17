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

//! Score header and body metadata field lowering, voice declarations, and user symbol expansion.

use std::collections::BTreeMap;

use abc_parser::Directive;
use abc_parser::ErrorKind;
use abc_parser::Field;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::KeySignature;
use abc_parser::Line;
use abc_parser::LineBreak;
use abc_parser::MusicElement;
use abc_parser::ParseWarning;
use abc_parser::Spanned;
use abc_parser::TypesetText;
use abc_parser::VoiceDefinition;
use abc_parser::parse_music_line;

use crate::lower::types::CreditItem;
use crate::lower::types::LoweredHeader;
use crate::lower::types::MidiInstrumentSpec;
use crate::lower::types::MiscFieldItem;
use crate::lower::types::Span;
use crate::lower::types::TextMetadataItem;
use crate::lower::types::VoiceDeclInfo;
use crate::lower::types::VoiceDefaults;
use crate::lower::types::VoiceEvent;
use crate::pitch::AccidentalPropagation;
use crate::pitch::ResolvedKey;
use crate::pitch::parse_propagate_accidentals;
use crate::pitch::parse_staff_parameters;
use crate::pitch::resolve_key_signature;
use crate::score_directive::ScoreDirectiveKind;
use crate::source_map::SourceProvenance;

pub struct ContinuationScan<'a> {
    pub next_idx: usize,
    pub extra_spans: Vec<Span>,
    pub continuation_lines: Vec<&'a Spanned<Line<Span, String>, Span>>,
}

/// Consumes any following `+:` continuation lines (and intervening comments) after `start_idx`.
pub fn consume_continuations<'a>(
    lines: &'a [Spanned<Line<Span, String>, Span>],
    start_idx: usize,
    text_buf: &mut Option<String>,
) -> ContinuationScan<'a> {
    let mut lookahead = start_idx + 1;
    let mut extra_spans = Vec::new();
    let mut continuation_lines = Vec::new();
    while lookahead < lines.len() {
        match &lines[lookahead].value {
            Line::FieldContinuation(cont) | Line::DeprecatedHistoryContinuation(cont) => {
                let cont = cont.strip_prefix("+:").unwrap_or(cont);
                extra_spans.push(lines[lookahead].span);
                continuation_lines.push(&lines[lookahead]);
                if let Some(buf) = text_buf {
                    if !buf.is_empty() && !buf.ends_with(' ') && !cont.starts_with(' ') {
                        buf.push(' ');
                    }
                    buf.push_str(cont.trim_end());
                }
                lookahead += 1;
            }
            Line::Comment(_) => {
                lookahead += 1;
            }
            _ => break,
        }
    }
    ContinuationScan {
        next_idx: lookahead,
        extra_spans,
        continuation_lines,
    }
}

/// Ensures that `voice_id` is present in `voice_order` and has an event entry in `voice_events`.
pub fn ensure_voice(
    voice_order: &mut Vec<String>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    voice_id: &str,
) {
    if !voice_events.contains_key(voice_id) {
        voice_order.push(voice_id.to_owned());
        voice_events.insert(voice_id.to_owned(), Vec::new());
    }
}

/// Processes a contiguous slice of header lines (file-level or tune-level header).
#[allow(clippy::too_many_lines)]
pub fn process_header_slice(
    lines: &[Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
    header: &mut LoweredHeader,
    defaults: &mut VoiceDefaults,
    voice_decls: &mut Vec<VoiceDeclInfo>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    let mut idx = 0;
    while idx < lines.len() {
        let line = &lines[idx];
        let mut extra_spans = Vec::new();
        let mut combined_text: Option<String> = None;

        if let Line::Field(field) = &line.value {
            let mut text_buf = match &field.value {
                FieldValue::Text(t) | FieldValue::Unparsed(t) => Some(t.clone()),
                _ => None,
            };
            let scan = consume_continuations(lines, idx, &mut text_buf);
            extra_spans = scan.extra_spans;
            combined_text = text_buf;
            idx = scan.next_idx;
        } else {
            idx += 1;
        }

        let prov = SourceProvenance::from_line(line.span).with_extra_line_spans(extra_spans);
        match &line.value {
            Line::Field(field) => {
                apply_header_field(
                    field,
                    combined_text.as_deref(),
                    &prov,
                    source,
                    header,
                    defaults,
                    voice_decls,
                    warnings,
                );
            }
            Line::Directive(directive) => {
                if directive.name.eq_ignore_ascii_case("propagate-accidentals")
                    && let Some(prop) = parse_propagate_accidentals(&directive.arguments)
                {
                    defaults.propagation = prop;
                } else if directive.name.eq_ignore_ascii_case("score") {
                    defaults.score_directive = Some((
                        ScoreDirectiveKind::Score,
                        directive.arguments.clone(),
                        prov.clone(),
                    ));
                } else if directive.name.eq_ignore_ascii_case("staves") {
                    defaults.score_directive = Some((
                        ScoreDirectiveKind::Staves,
                        directive.arguments.clone(),
                        prov.clone(),
                    ));
                } else if directive.name.eq_ignore_ascii_case("midi") {
                    update_midi_spec(defaults.midi.get_or_insert_default(), &directive.arguments);
                } else {
                    let name_lower = directive.name.to_ascii_lowercase();
                    if matches!(
                        name_lower.as_str(),
                        "scale"
                            | "pageheight"
                            | "pagewidth"
                            | "topmargin"
                            | "bottommargin"
                            | "leftmargin"
                            | "rightmargin"
                            | "staffwidth"
                            | "staffsep"
                            | "sysstaffsep"
                            | "titlefont"
                            | "subtitlefont"
                            | "composerfont"
                            | "partsfont"
                            | "tempofont"
                            | "vocalfont"
                            | "gchordfont"
                            | "wordsfont"
                            | "font"
                            | "measurebox"
                            | "measurenb"
                            | "barsperstaff"
                            | "landscape"
                    ) {
                        let text = if directive.arguments.trim().is_empty() {
                            format!("%%{}", directive.name)
                        } else {
                            format!("%%{} {}", directive.name, directive.arguments.trim())
                        };
                        header.typesetting_comments.push(text);
                    }
                }
            }
            Line::TypesetText(typeset) => {
                push_typeset_credit(&mut header.credits, typeset, prov);
            }
            _ => {}
        }
    }
}

fn field_text_value(field: &Field<String>, combined_text: Option<&str>) -> String {
    combined_text
        .unwrap_or(match &field.value {
            FieldValue::Text(t) | FieldValue::Unparsed(t) => t.as_str(),
            _ => "",
        })
        .trim()
        .to_owned()
}

fn apply_metadata_field(
    field: &Field<String>,
    combined_text: Option<&str>,
    prov: &SourceProvenance,
    header: &mut LoweredHeader,
) -> bool {
    let text_val = || field_text_value(field, combined_text);
    match field.kind {
        FieldKind::Title => {
            let text = text_val();
            if header.movement_title.is_none() {
                header.movement_title = Some(TextMetadataItem {
                    text,
                    provenance: prov.clone(),
                });
            } else {
                header.credits.push(CreditItem {
                    credit_type: Some("subtitle"),
                    words: text.clone(),
                    justify: None,
                    provenance: prov.clone(),
                });
                header.misc_fields.push(MiscFieldItem {
                    name: "subtitle",
                    text,
                    provenance: prov.clone(),
                });
            }
            true
        }
        FieldKind::Composer => {
            header.composers.push(TextMetadataItem {
                text: text_val(),
                provenance: prov.clone(),
            });
            true
        }
        FieldKind::Transcription => {
            let text = text_val();
            let lower = text.to_ascii_lowercase();
            if lower.starts_with("copyright") || text.starts_with('©') || lower.starts_with("(c)")
            {
                header.rights.push(TextMetadataItem {
                    text,
                    provenance: prov.clone(),
                });
            } else {
                header.encoders.push(TextMetadataItem {
                    text,
                    provenance: prov.clone(),
                });
            }
            true
        }
        FieldKind::Source => {
            header.sources.push(TextMetadataItem {
                text: text_val(),
                provenance: prov.clone(),
            });
            true
        }
        FieldKind::FileUrl => {
            header.relations.push(TextMetadataItem {
                text: text_val(),
                provenance: prov.clone(),
            });
            true
        }
        FieldKind::Origin
        | FieldKind::Area
        | FieldKind::Book
        | FieldKind::Discography
        | FieldKind::Group
        | FieldKind::History
        | FieldKind::Notes
        | FieldKind::Rhythm
        | FieldKind::Words => {
            let name = match field.kind {
                FieldKind::Origin | FieldKind::Area => "origin",
                FieldKind::Book => "book",
                FieldKind::Discography => "discography",
                FieldKind::Group => "group",
                FieldKind::History => "history",
                FieldKind::Notes => "notes",
                FieldKind::Rhythm => "rhythm",
                FieldKind::Words => "words",
                _ => unreachable!(),
            };
            header.misc_fields.push(MiscFieldItem {
                name,
                text: text_val(),
                provenance: prov.clone(),
            });
            true
        }
        _ => false,
    }
}

fn resolve_key_field(
    field: &Field<String>,
    prov: &SourceProvenance,
    source: Option<&str>,
) -> Option<ResolvedKey> {
    if let FieldValue::Key(key) = &field.value {
        let raw_slice = source.and_then(|s| s.get(prov.element_span().into_range()));
        Some(resolve_key_signature(key, raw_slice))
    } else if matches!(field.value, FieldValue::Empty) {
        let empty_key = KeySignature {
            tonic: None,
            mode: "none".to_owned(),
            parameters: Vec::new(),
        };
        Some(resolve_key_signature(&empty_key, None))
    } else {
        None
    }
}

fn upsert_voice_decl(
    voice_decls: &mut Vec<VoiceDeclInfo>,
    voice_def: &VoiceDefinition<String>,
    prov: &SourceProvenance,
) -> VoiceDeclInfo {
    let info = parse_voice_decl(voice_def, Some(prov.clone()));
    if let Some(existing) = voice_decls.iter_mut().find(|d| d.id == info.id) {
        merge_voice_decl(existing, info.clone());
    } else {
        voice_decls.push(info.clone());
    }
    info
}

fn push_macro_warning(warnings: &mut Vec<ParseWarning<Span>>, prov: &SourceProvenance) {
    warnings.push(ParseWarning {
        kind: ErrorKind::DeprecatedSyntax,
        message: "m: macro definitions are not expanded during MusicXML conversion".to_owned(),
        span: prov.element_span(),
        related: Vec::new(),
    });
}

fn parse_propagate_instruction(field: &Field<String>) -> Option<AccidentalPropagation> {
    if let FieldValue::Text(text) = &field.value
        && let Some(rest) = text.trim().strip_prefix("propagate-accidentals")
    {
        return parse_propagate_accidentals(rest);
    }
    None
}

/// Pre-parses a `U:` user-symbol replacement string into AST music elements.
#[must_use]
pub fn parse_user_symbol_replacement(replacement: &str) -> Vec<MusicElement<String>> {
    let trimmed = replacement.trim();
    if trimmed.is_empty() || trimmed == "!nil!" || trimmed == "!none!" {
        return Vec::new();
    }
    parse_music_line(trimmed)
        .output
        .map_or_else(Vec::new, |parsed| {
            parsed.into_iter().map(|s| s.value).collect()
        })
}

/// Applies a single header field to `header`, `defaults`, and `voice_decls`.
#[allow(clippy::too_many_arguments)]
pub fn apply_header_field(
    field: &Field<String>,
    combined_text: Option<&str>,
    prov: &SourceProvenance,
    source: Option<&str>,
    header: &mut LoweredHeader,
    defaults: &mut VoiceDefaults,
    voice_decls: &mut Vec<VoiceDeclInfo>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    if apply_metadata_field(field, combined_text, prov, header) {
        return;
    }

    match field.kind {
        FieldKind::Reference => {
            if let FieldValue::Reference(num) = field.value {
                header.reference = Some((num, prov.clone()));
            }
        }
        FieldKind::Meter => {
            if let FieldValue::Meter(meter) = &field.value {
                if !defaults.explicit_unit_length {
                    defaults.unit_length = (meter.default_unit_length(), Some(prov.clone()));
                }
                defaults.meter = Some((meter.clone(), Some(prov.clone())));
            }
        }
        FieldKind::UnitLength => {
            if let FieldValue::UnitLength(unit) = field.value {
                defaults.unit_length = (unit, Some(prov.clone()));
                defaults.explicit_unit_length = true;
            }
        }
        FieldKind::Key => {
            if let Some(resolved) = resolve_key_field(field, prov, source) {
                defaults.key = (resolved, Some(prov.clone()));
            }
        }
        FieldKind::Voice => {
            if let FieldValue::Voice(voice_def) = &field.value {
                upsert_voice_decl(voice_decls, voice_def, prov);
            }
        }
        FieldKind::UserSymbol => {
            if let FieldValue::UserSymbol(sym) = &field.value {
                defaults
                    .user_symbols
                    .insert(sym.symbol, parse_user_symbol_replacement(&sym.replacement));
            }
        }
        FieldKind::Tempo => {
            if let FieldValue::Tempo(tempo) = &field.value {
                defaults.tempo = Some((tempo.clone(), prov.clone()));
            }
        }
        FieldKind::Macro => push_macro_warning(warnings, prov),
        FieldKind::Instruction => {
            if let Some(prop) = parse_propagate_instruction(field) {
                defaults.propagation = prop;
            }
        }
        _ => {}
    }
}

/// Applies a tune-body inline field (e.g. `[V:1]`, `[K:G]`, `[M:6/8]`, `[L:1/8]`).
#[allow(clippy::too_many_arguments)]
pub fn apply_body_field(
    field: &Field<String>,
    combined_text: Option<&str>,
    prov: &SourceProvenance,
    source: Option<&str>,
    header: &mut LoweredHeader,
    active_user_symbols: &mut BTreeMap<char, Vec<MusicElement<String>>>,
    current_voice: &mut String,
    voice_order: &mut Vec<String>,
    voice_decls: &mut Vec<VoiceDeclInfo>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    if apply_metadata_field(field, combined_text, prov, header) {
        return;
    }

    match field.kind {
        FieldKind::Voice => {
            if let FieldValue::Voice(voice_def) = &field.value {
                current_voice.clone_from(&voice_def.id);
                ensure_voice(voice_order, voice_events, current_voice);
                let info = upsert_voice_decl(voice_decls, voice_def, prov);
                if info.clef.is_some() || info.transpose.is_some() || info.octave_shift.is_some() {
                    voice_events
                        .get_mut(current_voice)
                        .expect("voice initialized")
                        .push(VoiceEvent::VoiceParams(info));
                }
            }
        }
        FieldKind::Meter => {
            if let FieldValue::Meter(meter) = &field.value {
                ensure_voice(voice_order, voice_events, current_voice);
                voice_events
                    .get_mut(current_voice)
                    .expect("voice initialized")
                    .push(VoiceEvent::Meter(meter.clone(), prov.clone()));
            }
        }
        FieldKind::UnitLength => {
            if let FieldValue::UnitLength(unit) = field.value {
                ensure_voice(voice_order, voice_events, current_voice);
                voice_events
                    .get_mut(current_voice)
                    .expect("voice initialized")
                    .push(VoiceEvent::UnitLength(unit, prov.clone()));
            }
        }
        FieldKind::Key => {
            if let Some(resolved) = resolve_key_field(field, prov, source) {
                ensure_voice(voice_order, voice_events, current_voice);
                voice_events
                    .get_mut(current_voice)
                    .expect("voice initialized")
                    .push(VoiceEvent::Key(resolved, prov.clone()));
            }
        }
        FieldKind::Tempo => {
            if let FieldValue::Tempo(tempo) = &field.value {
                ensure_voice(voice_order, voice_events, current_voice);
                voice_events
                    .get_mut(current_voice)
                    .expect("voice initialized")
                    .push(VoiceEvent::Tempo(tempo.clone(), prov.clone()));
            }
        }
        FieldKind::UserSymbol => {
            if let FieldValue::UserSymbol(sym) = &field.value {
                active_user_symbols
                    .insert(sym.symbol, parse_user_symbol_replacement(&sym.replacement));
            }
        }
        FieldKind::Macro => push_macro_warning(warnings, prov),
        FieldKind::Instruction => {
            if let Some(prop) = parse_propagate_instruction(field) {
                ensure_voice(voice_order, voice_events, current_voice);
                voice_events
                    .get_mut(current_voice)
                    .expect("voice initialized")
                    .push(VoiceEvent::Propagation(prop));
            }
        }
        _ => {}
    }
}

/// Parses a `%%MIDI` directive line into updates for a [`MidiInstrumentSpec`].
///
/// Supports:
/// - `channel <1..16>`: MIDI playback channel.
/// - `program <1..128>`: General MIDI patch number (0 normalized to 1).
/// - `control 7 <0..127>` / `volume <0..127>`: MIDI volume controller, scaled to
///   percentage (0..=100) per `MusicXML` schema during serialization.
/// - `pan <-180..180>`: Pan degrees.
pub fn update_midi_spec(spec: &mut MidiInstrumentSpec, arguments: &str) {
    let mut tokens = arguments.split_whitespace();
    let Some(subcmd) = tokens.next() else { return };
    if subcmd.eq_ignore_ascii_case("channel") {
        if let Some(ch_str) = tokens.next()
            && let Ok(ch) = ch_str.parse::<u8>()
            && (1..=16).contains(&ch)
        {
            spec.channel = Some(ch);
        }
    } else if subcmd.eq_ignore_ascii_case("program") {
        if let Some(prog_str) = tokens.next()
            && let Ok(prog) = prog_str.parse::<u8>()
        {
            let midi_prog = if prog == 0 { 1 } else { prog.min(128) };
            spec.program = Some(midi_prog);
        }
    } else if subcmd.eq_ignore_ascii_case("control") {
        if let Some(ctrl_str) = tokens.next()
            && ctrl_str == "7"
            && let Some(vol_str) = tokens.next()
            && let Ok(vol) = vol_str.parse::<u8>()
        {
            spec.volume = Some(vol.min(127));
        }
    } else if subcmd.eq_ignore_ascii_case("volume") {
        if let Some(vol_str) = tokens.next()
            && let Ok(vol) = vol_str.parse::<u8>()
        {
            spec.volume = Some(vol.min(127));
        }
    } else if subcmd.eq_ignore_ascii_case("pan")
        && let Some(pan_str) = tokens.next()
        && let Ok(pan) = pan_str.parse::<i16>()
    {
        spec.pan = Some(pan);
    }
}

/// Applies body directives (e.g. `%%propagate-accidentals` or `%%MIDI`).
pub fn apply_body_directive(
    directive: &Directive<String>,
    prov: &SourceProvenance,
    current_voice: &str,
    voice_order: &mut Vec<String>,
    voice_events: &mut BTreeMap<String, Vec<VoiceEvent>>,
) {
    if directive.name.eq_ignore_ascii_case("propagate-accidentals")
        && let Some(prop) = parse_propagate_accidentals(&directive.arguments)
    {
        ensure_voice(voice_order, voice_events, current_voice);
        voice_events
            .get_mut(current_voice)
            .expect("voice initialized")
            .push(VoiceEvent::Propagation(prop));
    } else if directive.name.eq_ignore_ascii_case("midi") {
        let mut spec = MidiInstrumentSpec::default();
        update_midi_spec(&mut spec, &directive.arguments);
        if !spec.is_empty() {
            ensure_voice(voice_order, voice_events, current_voice);
            voice_events
                .get_mut(current_voice)
                .expect("voice initialized")
                .push(VoiceEvent::VoiceParams(VoiceDeclInfo {
                    id: current_voice.to_owned(),
                    name: None,
                    subname: None,
                    clef: None,
                    transpose: None,
                    octave_shift: None,
                    midi: Some(spec),
                    provenance: None,
                }));
        }
    } else if directive.name.eq_ignore_ascii_case("newpage") {
        ensure_voice(voice_order, voice_events, current_voice);
        voice_events
            .get_mut(current_voice)
            .expect("voice initialized")
            .push(VoiceEvent::MusicElement {
                element: MusicElement::LineBreak(LineBreak::Paragraph),
                line_span: prov.line_span(),
                element_span: prov.element_span(),
            });
    } else if directive.name.eq_ignore_ascii_case("staffbreak")
        || directive.name.eq_ignore_ascii_case("linebreak")
    {
        ensure_voice(voice_order, voice_events, current_voice);
        voice_events
            .get_mut(current_voice)
            .expect("voice initialized")
            .push(VoiceEvent::MusicElement {
                element: MusicElement::LineBreak(LineBreak::Break),
                line_span: prov.line_span(),
                element_span: prov.element_span(),
            });
    }
}

/// Adds a typeset text block or centered line to the score credits list.
pub fn push_typeset_credit(
    credits: &mut Vec<CreditItem>,
    typeset: &TypesetText<String>,
    provenance: SourceProvenance,
) {
    match typeset {
        TypesetText::Text(text) => credits.push(CreditItem {
            credit_type: None,
            words: text.trim().to_owned(),
            justify: Some("left"),
            provenance,
        }),
        TypesetText::Centered(text) => credits.push(CreditItem {
            credit_type: None,
            words: text.trim().to_owned(),
            justify: Some("center"),
            provenance,
        }),
        TypesetText::Block(lines) => {
            let joined = lines
                .iter()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join("\n");
            credits.push(CreditItem {
                credit_type: None,
                words: joined,
                justify: Some("left"),
                provenance,
            });
        }
        _ => {}
    }
}

/// Parses voice declaration properties (name, clef, transpose, octave-shift) from an ABC `V:` field.
#[must_use]
pub fn parse_voice_decl(
    voice_def: &VoiceDefinition<String>,
    provenance: Option<SourceProvenance>,
) -> VoiceDeclInfo {
    let mut name = None;
    let mut subname = None;
    for prop in &voice_def.properties {
        match prop.name.as_deref() {
            Some(key) if key.eq_ignore_ascii_case("name") || key.eq_ignore_ascii_case("nm") => {
                name = Some(prop.value.clone());
            }
            Some(key) if key.eq_ignore_ascii_case("subname") || key.eq_ignore_ascii_case("snm") => {
                subname = Some(prop.value.clone());
            }
            _ => {}
        }
    }
    let staff_params = parse_staff_parameters(&voice_def.properties);
    VoiceDeclInfo {
        id: voice_def.id.clone(),
        name,
        subname,
        clef: staff_params.clef,
        transpose: staff_params.transpose,
        octave_shift: staff_params.octave_shift,
        midi: None,
        provenance,
    }
}

/// Merges properties from `update` into an existing `target` [`VoiceDeclInfo`].
pub fn merge_voice_decl(target: &mut VoiceDeclInfo, update: VoiceDeclInfo) {
    if update.name.is_some() {
        target.name = update.name;
    }
    if update.subname.is_some() {
        target.subname = update.subname;
    }
    if update.midi.is_some() {
        target.midi = update.midi;
    }
    if update.clef.is_some() {
        target.clef = update.clef;
    }
    if update.transpose.is_some() {
        target.transpose = update.transpose;
    }
    if update.octave_shift.is_some() {
        target.octave_shift = update.octave_shift;
    }
    if target.provenance.is_none() {
        target.provenance = update.provenance;
    }
}

/// Extracts text slices and start offsets across a field and its `+:` continuation lines.
pub fn extract_field_segments<'a>(
    line: &'a Spanned<Line<Span, String>, Span>,
    cont_lines: &[&'a Spanned<Line<Span, String>, Span>],
    source: Option<&str>,
) -> Vec<(&'a str, usize)> {
    let mut segments = Vec::with_capacity(1 + cont_lines.len());
    let offset_after_colon = |span: Span| -> usize {
        if let Some(src) = source
            && let Some(slice) = src.get(span.start..span.end)
            && let Some(colon) = slice.find(':')
        {
            span.start + colon + 1
        } else {
            span.start + 2
        }
    };

    if let Line::Field(field) = &line.value
        && let FieldValue::Text(text) = &field.value
    {
        segments.push((text.as_str(), offset_after_colon(line.span)));
    }
    for cont in cont_lines {
        if let Line::FieldContinuation(cont_str) | Line::DeprecatedHistoryContinuation(cont_str) =
            &cont.value
        {
            let text = cont_str.strip_prefix("+:").unwrap_or(cont_str);
            segments.push((text, offset_after_colon(cont.span)));
        }
    }
    segments
}

/// Expands user symbol (`U:`) replacements on `elem` and pushes resulting events into `out`.
pub fn expand_and_push_music_element(
    elem: &Spanned<MusicElement<String>, Span>,
    line_span: Span,
    source: Option<&str>,
    user_symbols: &BTreeMap<char, Vec<MusicElement<String>>>,
    out: &mut Vec<VoiceEvent>,
) {
    if !user_symbols.is_empty()
        && let Some(sym_char) = detect_user_symbol_char(&elem.value, elem.span, source)
        && let Some(replacement) = user_symbols.get(&sym_char)
    {
        for sub in replacement {
            out.push(VoiceEvent::MusicElement {
                element: sub.clone(),
                line_span,
                element_span: elem.span,
            });
        }
        return;
    }

    out.push(VoiceEvent::MusicElement {
        element: elem.value.clone(),
        line_span,
        element_span: elem.span,
    });
}

/// Detects if an element is a single character representation of a user symbol (e.g. `~`, `H`, `T`, `u`, `v`).
#[must_use]
pub fn detect_user_symbol_char(
    element: &MusicElement<String>,
    span: Span,
    source: Option<&str>,
) -> Option<char> {
    if span.end.saturating_sub(span.start) != 1 {
        return None;
    }
    if let Some(src) = source
        && let Some(slice) = src.get(span.into_range())
    {
        let mut chars = slice.chars();
        if let Some(ch) = chars.next()
            && chars.next().is_none()
            && (ch == '~' || ('H'..='W').contains(&ch) || ('h'..='w').contains(&ch))
        {
            return Some(ch);
        }
    }
    match element {
        MusicElement::Decoration(dec) if !dec.legacy_delimiter => match dec.name.as_str() {
            "roll" => Some('~'),
            "fermata" => Some('H'),
            "accent" => Some('L'),
            "lowermordent" => Some('M'),
            "coda" => Some('O'),
            "uppermordent" => Some('P'),
            "segno" => Some('S'),
            "trill" => Some('T'),
            "upbow" => Some('u'),
            "downbow" => Some('v'),
            _ => None,
        },
        MusicElement::Extension(ext) if ext.len() == 1 => ext.chars().next(),
        _ => None,
    }
}
