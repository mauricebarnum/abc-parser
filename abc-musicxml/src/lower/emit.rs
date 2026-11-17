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

//! Streaming serialization from lowered model AST into `MusicXML` 4.0 XML.

use std::fmt;

use crate::build_info;
use crate::harmony::write_harmony_xml;
use crate::lower::types::GraceNoteStyle;
use crate::lower::types::LoweredAttributes;
use crate::lower::types::LoweredBarline;
use crate::lower::types::LoweredDirection;
use crate::lower::types::LoweredDirectionKind;
use crate::lower::types::LoweredHarmony;
use crate::lower::types::LoweredHeader;
use crate::lower::types::LoweredMeasure;
use crate::lower::types::LoweredNotations;
use crate::lower::types::LoweredNote;
use crate::lower::types::LoweredNoteKind;
use crate::lower::types::LoweredOrnament;
use crate::lower::types::LoweredPart;
use crate::lower::types::LoweredSound;
use crate::lower::types::LoweredTechnical;
use crate::lower::types::MeasureItem;
use crate::pitch::format_alter;
use crate::pitch::pitch_step_str;
use crate::pitch::write_clef_xml;
use crate::pitch::write_clef_xml_with_number;
use crate::pitch::write_key_xml;
use crate::pitch::write_transpose_xml;
use crate::score_directive::PartListEntry;
use crate::source_map::SourceProvenance;
use crate::timing::write_time_signature;
use crate::writer::ProvenanceEmitter;
use crate::writer::XmlWriter;

/// Serializes score-level identification, credits, and the `<part-list>` element.
pub fn emit_score_header<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    header: &LoweredHeader,
    parts: &[LoweredPart],
    entries: &[PartListEntry],
    score_directive_prov: Option<&SourceProvenance>,
) -> fmt::Result {
    for comment in &header.typesetting_comments {
        writer.write_comment(comment)?;
    }

    if let Some((num, ref prov)) = header.reference {
        emitter.emit_header_provenance(writer, Some(prov))?;
        writer.display_element("movement-number", num)?;
    }

    if let Some(ref title) = header.movement_title {
        emitter.emit_header_provenance(writer, Some(&title.provenance))?;
        writer.text_element("movement-title", &title.text)?;
    }

    writer.open_tag("identification")?;
    for composer in &header.composers {
        emitter.emit_header_provenance(writer, Some(&composer.provenance))?;
        writer.text_element_with_attrs("creator", &[("type", "composer")], &composer.text)?;
    }
    for rights in &header.rights {
        emitter.emit_header_provenance(writer, Some(&rights.provenance))?;
        writer.text_element("rights", &rights.text)?;
    }
    writer.open_tag("encoding")?;
    let software = format!("abc-musicxml {}", build_info::BUILD_INFO.version);
    writer.text_element("software", &software)?;
    for encoder in &header.encoders {
        emitter.emit_header_provenance(writer, Some(&encoder.provenance))?;
        writer.text_element("encoder", &encoder.text)?;
    }
    writer.close_tag("encoding")?;

    if !header.sources.is_empty() {
        let first_prov = &header.sources[0].provenance;
        let mut extra = first_prov.extra_line_spans().to_vec();
        for src_item in &header.sources[1..] {
            extra.push(src_item.provenance.line_span());
            extra.extend_from_slice(src_item.provenance.extra_line_spans());
        }
        let merged_prov = first_prov.clone().with_extra_line_spans(extra);
        let joined_source = header
            .sources
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        emitter.emit_header_provenance(writer, Some(&merged_prov))?;
        writer.text_element("source", &joined_source)?;
    }

    for relation in &header.relations {
        emitter.emit_header_provenance(writer, Some(&relation.provenance))?;
        writer.text_element("relation", &relation.text)?;
    }

    if !header.misc_fields.is_empty() {
        writer.open_tag("miscellaneous")?;
        for misc in &header.misc_fields {
            emitter.emit_header_provenance(writer, Some(&misc.provenance))?;
            writer.text_element_with_attrs(
                "miscellaneous-field",
                &[("name", misc.name)],
                &misc.text,
            )?;
        }
        writer.close_tag("miscellaneous")?;
    }
    writer.close_tag("identification")?;

    for credit in &header.credits {
        emitter.emit_header_provenance(writer, Some(&credit.provenance))?;
        writer.open_tag_with_attrs("credit", &[("page", "1")])?;
        if let Some(ctype) = credit.credit_type {
            writer.text_element("credit-type", ctype)?;
        }
        if let Some(justify) = credit.justify {
            writer.text_element_with_attrs(
                "credit-words",
                &[("justify", justify)],
                &credit.words,
            )?;
        } else {
            writer.text_element("credit-words", &credit.words)?;
        }
        writer.close_tag("credit")?;
    }

    emit_part_list(writer, emitter, parts, entries, score_directive_prov)
}

/// Serializes the `<part-list>` element and all its child `<score-part>` and `<part-group>` entries.
pub fn emit_part_list<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    parts: &[LoweredPart],
    entries: &[PartListEntry],
    score_directive_prov: Option<&SourceProvenance>,
) -> fmt::Result {
    emitter.emit_header_provenance(writer, score_directive_prov)?;
    writer.open_tag("part-list")?;

    for entry in entries {
        match entry {
            PartListEntry::PartGroupStart {
                number,
                symbol,
                barline,
            } => {
                let mut num_buf = [0_u8; 40];
                let num_str = XmlWriter::<W>::format_u128(u128::from(*number), &mut num_buf);
                writer
                    .open_tag_with_attrs("part-group", &[("type", "start"), ("number", num_str)])?;
                if let Some(sym) = symbol {
                    writer.text_element("group-symbol", sym)?;
                }
                let bar_str = if *barline { "yes" } else { "no" };
                writer.text_element("group-barline", bar_str)?;
                writer.close_tag("part-group")?;
            }
            PartListEntry::ScorePart(part_idx) => {
                let part = &parts[*part_idx];
                emitter.emit_header_provenance(writer, part.declaration_provenance.as_ref())?;
                writer.open_tag_with_attrs("score-part", &[("id", &part.part_id)])?;
                writer.text_element("part-name", &part.part_name)?;
                if let Some(ref abbr) = part.part_abbreviation {
                    writer.text_element("part-abbreviation", abbr)?;
                }
                if let Some(ref midi) = part.midi_instrument
                    && !midi.is_empty()
                {
                    let inst_id = format!("{}-I1", part.part_id);
                    writer.open_tag_with_attrs("score-instrument", &[("id", &inst_id)])?;
                    writer.text_element("instrument-name", &part.part_name)?;
                    writer.close_tag("score-instrument")?;

                    writer.open_tag_with_attrs("midi-instrument", &[("id", &inst_id)])?;
                    if let Some(ch) = midi.channel {
                        writer.display_element("midi-channel", ch)?;
                    }
                    if let Some(prog) = midi.program {
                        writer.display_element("midi-program", prog)?;
                    }
                    if let Some(vol) = midi.volume {
                        let vol_pct = ((u32::from(vol) * 100 + 63) / 127).min(100);
                        writer.display_element("volume", vol_pct)?;
                    }
                    if let Some(pan) = midi.pan {
                        writer.display_element("pan", pan.clamp(-180, 180))?;
                    }
                    writer.close_tag("midi-instrument")?;
                }
                writer.close_tag("score-part")?;
            }
            PartListEntry::PartGroupStop { number } => {
                let mut num_buf = [0_u8; 40];
                let num_str = XmlWriter::<W>::format_u128(u128::from(*number), &mut num_buf);
                writer
                    .empty_tag_with_attrs("part-group", &[("type", "stop"), ("number", num_str)])?;
            }
        }
    }
    writer.close_tag("part-list")
}

/// Serializes a single `<measure>` element, including attributes, directions, notes, harmonies, and barlines.
pub fn emit_measure<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    measure: &LoweredMeasure,
    part_divisions: u128,
) -> fmt::Result {
    emitter.emit_music_provenance(writer, measure.provenance.as_ref())?;
    if measure.implicit {
        writer.open_tag_with_attrs(
            "measure",
            &[("number", &measure.number), ("implicit", "yes")],
        )?;
    } else {
        writer.open_tag_with_attrs("measure", &[("number", &measure.number)])?;
    }

    if measure.new_page {
        writer.empty_tag_with_attrs("print", &[("new-page", "yes")])?;
    } else if measure.new_system {
        writer.empty_tag_with_attrs("print", &[("new-system", "yes")])?;
    }

    if let Some(ref left_bar) = measure.left_barline {
        emit_barline(writer, emitter, left_bar)?;
    }

    if let Some(ref attrs) = measure.attributes {
        emit_attributes(writer, emitter, attrs)?;
    }

    for item in &measure.items {
        match item {
            MeasureItem::Backup {
                duration,
                provenance,
            } => {
                emitter.emit_music_provenance(writer, provenance.as_ref())?;
                writer.open_tag("backup")?;
                let ticks = duration.to_divisions(part_divisions);
                writer.display_element("duration", ticks)?;
                writer.close_tag("backup")?;
            }
            MeasureItem::Direction(dir) => emit_direction(writer, emitter, dir)?,
            MeasureItem::Harmony(harm) => emit_harmony(writer, emitter, harm)?,
            MeasureItem::Note(note) => emit_note(writer, emitter, note, part_divisions)?,
        }
    }

    if let Some(ref right_bar) = measure.right_barline {
        emit_barline(writer, emitter, right_bar)?;
    }

    writer.close_tag("measure")
}

/// Serializes an `<attributes>` element (divisions, key, time signature, clef, transpose, multiple-rest).
pub fn emit_attributes<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    attrs: &LoweredAttributes,
) -> fmt::Result {
    emitter.reset_header_cursor();
    writer.open_tag("attributes")?;

    if let Some((divs, ref prov)) = attrs.divisions {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        writer.display_element("divisions", divs)?;
    }

    if let Some((ref key, ref prov)) = attrs.key {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        write_key_xml(writer, key)?;
    }

    if let Some((ref meter, ref prov)) = attrs.time {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        write_time_signature(writer, meter)?;
    }

    if let Some(staves) = attrs.staves {
        writer.display_element("staves", staves)?;
    }

    if let Some(symbol) = attrs.part_symbol {
        writer.text_element("part-symbol", symbol)?;
    }

    if !attrs.clefs.is_empty() {
        for (clef, number, prov) in &attrs.clefs {
            emitter.emit_header_provenance(writer, prov.as_ref())?;
            write_clef_xml_with_number(writer, clef, *number)?;
        }
    } else if let Some((ref clef, ref prov)) = attrs.clef {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        write_clef_xml(writer, clef)?;
    }

    if let Some((ref transpose, ref prov)) = attrs.transpose {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        write_transpose_xml(writer, transpose)?;
    }

    if let Some((count, ref prov)) = attrs.multiple_rest {
        emitter.emit_header_provenance(writer, prov.as_ref())?;
        writer.open_tag("measure-style")?;
        writer.display_element("multiple-rest", count)?;
        writer.close_tag("measure-style")?;
    }

    writer.close_tag("attributes")
}

/// Serializes a `<barline>` element with style, fermata, ending, and repeat markers.
pub fn emit_barline<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    barline: &LoweredBarline,
) -> fmt::Result {
    emitter.emit_music_provenance(writer, barline.provenance.as_ref())?;
    writer.open_tag_with_attrs("barline", &[("location", barline.location)])?;
    if let Some(style) = barline.bar_style {
        writer.text_element("bar-style", style)?;
    }
    if let Some((ferm_type, ref prov)) = barline.fermata {
        emitter.emit_subelement_provenance(writer, prov.as_ref())?;
        writer.empty_tag_with_attrs("fermata", &[("type", ferm_type)])?;
    }
    if let Some(ref ending) = barline.ending {
        writer.empty_tag_with_attrs(
            "ending",
            &[
                ("number", &ending.number),
                ("type", ending.ending_type.as_str()),
            ],
        )?;
    }
    if let Some((direction, times)) = barline.repeat {
        if let Some(count) = times {
            let mut count_buf = [0_u8; 40];
            let count_str = XmlWriter::<W>::format_u128(u128::from(count), &mut count_buf);
            writer.empty_tag_with_attrs(
                "repeat",
                &[("direction", direction.as_str()), ("times", count_str)],
            )?;
        } else {
            writer.empty_tag_with_attrs("repeat", &[("direction", direction.as_str())])?;
        }
    }
    writer.close_tag("barline")
}

/// Serializes a `<harmony>` element from a chord symbol specification.
pub fn emit_harmony<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    harmony: &LoweredHarmony,
) -> fmt::Result {
    emitter.emit_music_provenance(writer, harmony.provenance.as_ref())?;
    write_harmony_xml(writer, &harmony.spec)
}

/// Serializes a `<direction>` element (dynamics, wedge, words, metronome, segno, coda, octave-shift).
pub fn emit_direction<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    dir: &LoweredDirection,
) -> fmt::Result {
    emitter.emit_music_provenance(writer, dir.provenance.as_ref())?;
    if let Some(placement) = dir.placement {
        writer.open_tag_with_attrs("direction", &[("placement", placement)])?;
    } else {
        writer.open_tag("direction")?;
    }

    match &dir.kind {
        LoweredDirectionKind::Dynamics(dyn_str) => {
            writer.open_tag("direction-type")?;
            writer.open_tag("dynamics")?;
            writer.empty_tag(dyn_str)?;
            writer.close_tag("dynamics")?;
            writer.close_tag("direction-type")?;
        }
        LoweredDirectionKind::Wedge(wedge_type) => {
            writer.open_tag("direction-type")?;
            writer.empty_tag_with_attrs("wedge", &[("type", wedge_type)])?;
            writer.close_tag("direction-type")?;
        }
        LoweredDirectionKind::Words {
            text,
            justify,
            relative_x,
            relative_y,
        } => emit_words_direction(writer, text, *justify, *relative_x, *relative_y)?,
        LoweredDirectionKind::Metronome {
            prelude,
            beat_unit,
            dots,
            bpm,
            postlude,
        } => {
            if let Some(pre) = prelude {
                writer.open_tag("direction-type")?;
                writer.text_element("words", pre)?;
                writer.close_tag("direction-type")?;
            }
            writer.open_tag("direction-type")?;
            writer.open_tag("metronome")?;
            writer.text_element("beat-unit", beat_unit)?;
            for _ in 0..*dots {
                writer.empty_tag("beat-unit-dot")?;
            }
            writer.display_element("per-minute", *bpm)?;
            writer.close_tag("metronome")?;
            writer.close_tag("direction-type")?;
            if let Some(post) = postlude {
                writer.open_tag("direction-type")?;
                writer.text_element("words", post)?;
                writer.close_tag("direction-type")?;
            }
        }
        LoweredDirectionKind::Segno => {
            writer.open_tag("direction-type")?;
            writer.empty_tag("segno")?;
            writer.close_tag("direction-type")?;
        }
        LoweredDirectionKind::Coda => {
            writer.open_tag("direction-type")?;
            writer.empty_tag("coda")?;
            writer.close_tag("direction-type")?;
        }
        LoweredDirectionKind::OctaveShift { shift_type, size } => {
            writer.open_tag("direction-type")?;
            let mut size_buf = [0_u8; 40];
            let size_str = XmlWriter::<W>::format_u128(u128::from(*size), &mut size_buf);
            writer.empty_tag_with_attrs(
                "octave-shift",
                &[("type", shift_type), ("size", size_str)],
            )?;
            writer.close_tag("direction-type")?;
        }
    }

    if let Some(sound) = dir.sound {
        emit_direction_sound(writer, sound)?;
    }

    writer.close_tag("direction")
}

fn emit_words_direction<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    text: &str,
    justify: Option<&'static str>,
    relative_x: Option<i32>,
    relative_y: Option<i32>,
) -> fmt::Result {
    writer.open_tag("direction-type")?;
    if justify.is_none() && relative_x.is_none() && relative_y.is_none() {
        writer.text_element("words", text)?;
    } else {
        let x_attr = relative_x.map(|v| v.to_string());
        let y_attr = relative_y.map(|v| v.to_string());
        let mut attrs: [(&str, &str); 3] = [("", ""); 3];
        let mut attr_len = 0;
        if let Some(j) = justify {
            attrs[attr_len] = ("justify", j);
            attr_len += 1;
        }
        if let Some(ref x) = x_attr {
            attrs[attr_len] = ("relative-x", x);
            attr_len += 1;
        }
        if let Some(ref y) = y_attr {
            attrs[attr_len] = ("relative-y", y);
            attr_len += 1;
        }
        writer.text_element_with_attrs("words", &attrs[..attr_len], text)?;
    }
    writer.close_tag("direction-type")
}

fn emit_direction_sound<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    sound: LoweredSound,
) -> fmt::Result {
    let mut num_buf = [0_u8; 40];
    let (attr, val) = match sound {
        LoweredSound::Tempo(tempo) => (
            "tempo",
            XmlWriter::<W>::format_u128(u128::from(tempo), &mut num_buf),
        ),
        LoweredSound::Segno(v) => ("segno", v),
        LoweredSound::Coda(v) => ("coda", v),
        LoweredSound::DalSegno(v) => ("dalsegno", v),
        LoweredSound::DaCapo => ("dacapo", "yes"),
        LoweredSound::ToCoda(v) => ("tocoda", v),
        LoweredSound::Fine => ("fine", "yes"),
    };
    writer.empty_tag_with_attrs("sound", &[(attr, val)])
}

/// Serializes a `<note>` element (pitch/rest, duration, voice, type, notations, lyrics).
pub fn emit_note<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &mut ProvenanceEmitter<'_>,
    note: &LoweredNote,
    part_divisions: u128,
) -> fmt::Result {
    emitter.emit_music_provenance(writer, note.provenance.as_ref())?;
    if note.print_object {
        writer.open_tag("note")?;
    } else {
        writer.open_tag_with_attrs("note", &[("print-object", "no")])?;
    }

    match note.grace {
        Some(GraceNoteStyle::Acciaccatura) => {
            writer.empty_tag_with_attrs("grace", &[("slash", "yes")])?;
        }
        Some(GraceNoteStyle::Appoggiatura) => {
            writer.empty_tag("grace")?;
        }
        None => {}
    }

    if note.is_chord_member {
        writer.empty_tag("chord")?;
    }

    emit_note_kind(writer, &note.kind)?;

    if !note.is_grace() {
        let dur_ticks = note.sounding_duration.to_divisions(part_divisions);
        writer.display_element("duration", dur_ticks)?;
    }

    if let Some(notations) = note.notations() {
        if notations.tied_stop {
            emitter.emit_subelement_provenance(writer, notations.tie_provenance.as_ref())?;
            writer.empty_tag_with_attrs("tie", &[("type", "stop")])?;
        }
        if notations.tied_start {
            emitter.emit_subelement_provenance(writer, notations.tie_provenance.as_ref())?;
            writer.empty_tag_with_attrs("tie", &[("type", "start")])?;
        }
    }

    writer.display_element("voice", note.voice)?;

    if let Some(note_type) = note.note_type {
        writer.text_element("type", note_type)?;
    }
    for _ in 0..note.dots {
        writer.empty_tag("dot")?;
    }

    if let LoweredNoteKind::Pitched {
        accidental: Some(acc),
        ..
    } = note.kind
    {
        writer.text_element("accidental", acc)?;
    }

    if let Some((actual, normal)) = note.time_modification {
        writer.open_tag("time-modification")?;
        writer.display_element("actual-notes", actual)?;
        writer.display_element("normal-notes", normal)?;
        writer.close_tag("time-modification")?;
    }

    if let Some(stem) = note.stem {
        writer.text_element("stem", stem)?;
    }

    if let Some(staff) = note.staff {
        writer.display_element("staff", staff)?;
    }

    let mut num_buf = [0_u8; 40];
    for beam in &note.beams {
        let num_str = XmlWriter::<W>::format_u128(u128::from(beam.number), &mut num_buf);
        writer.text_element_with_attrs("beam", &[("number", num_str)], beam.value)?;
    }

    if let Some(notations) = note.notations()
        && !notations.is_empty()
    {
        emit_notations(writer, emitter, notations)?;
    }

    emit_note_lyrics(writer, emitter, &note.lyrics)?;
    writer.close_tag("note")
}

fn emit_note_kind<W: fmt::Write>(writer: &mut XmlWriter<W>, kind: &LoweredNoteKind) -> fmt::Result {
    match *kind {
        LoweredNoteKind::Pitched {
            step,
            octave,
            alter,
            ..
        } => {
            writer.open_tag("pitch")?;
            writer.text_element("step", pitch_step_str(step))?;
            if let Some(alter_str) = format_alter(alter) {
                writer.text_element("alter", &alter_str)?;
            }
            writer.display_element("octave", octave)?;
            writer.close_tag("pitch")
        }
        LoweredNoteKind::Rest { measure } => {
            if measure {
                writer.empty_tag_with_attrs("rest", &[("measure", "yes")])
            } else {
                writer.empty_tag("rest")
            }
        }
        LoweredNoteKind::Unpitched {
            display_step,
            display_octave,
        } => {
            writer.open_tag("unpitched")?;
            writer.text_element("display-step", pitch_step_str(display_step))?;
            writer.display_element("display-octave", display_octave)?;
            writer.close_tag("unpitched")
        }
    }
}

fn emit_note_lyrics<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &ProvenanceEmitter<'_>,
    lyrics: &[crate::lower::types::LoweredLyric],
) -> fmt::Result {
    let mut num_buf = [0_u8; 40];
    for lyric in lyrics {
        emitter.emit_subelement_provenance(writer, lyric.provenance.as_ref())?;
        let num_str = XmlWriter::<W>::format_u128(u128::from(lyric.number), &mut num_buf);
        writer.open_tag_with_attrs("lyric", &[("number", num_str)])?;
        if let Some(syllabic) = lyric.syllabic {
            writer.text_element("syllabic", syllabic.as_str())?;
        }
        if let Some(ref text) = lyric.text {
            writer.text_element("text", text)?;
        }
        if let Some(extend) = lyric.extend {
            writer.empty_tag_with_attrs("extend", &[("type", extend.as_xml_str())])?;
        }
        writer.close_tag("lyric")?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn emit_notations<W: fmt::Write>(
    writer: &mut XmlWriter<W>,
    emitter: &ProvenanceEmitter<'_>,
    notations: &LoweredNotations,
) -> fmt::Result {
    let mut num_buf = [0_u8; 40];
    writer.open_tag("notations")?;
    if notations.tied_stop {
        emitter.emit_subelement_provenance(writer, notations.tie_provenance.as_ref())?;
        writer.empty_tag_with_attrs("tied", &[("type", "stop")])?;
    }
    if notations.tied_start {
        emitter.emit_subelement_provenance(writer, notations.tie_provenance.as_ref())?;
        if notations.tied_dotted {
            writer.empty_tag_with_attrs("tied", &[("type", "start"), ("line-type", "dotted")])?;
        } else {
            writer.empty_tag_with_attrs("tied", &[("type", "start")])?;
        }
    }
    for slur in &notations.slurs {
        emitter.emit_subelement_provenance(writer, slur.provenance.as_ref())?;
        let num_str = XmlWriter::<W>::format_u128(u128::from(slur.number), &mut num_buf);
        if slur.dotted {
            writer.empty_tag_with_attrs(
                "slur",
                &[
                    ("type", slur.slur_type.as_xml_str()),
                    ("number", num_str),
                    ("line-type", "dotted"),
                ],
            )?;
        } else {
            writer.empty_tag_with_attrs(
                "slur",
                &[("type", slur.slur_type.as_xml_str()), ("number", num_str)],
            )?;
        }
    }
    for (num, prov) in &notations.tuplets_start {
        emitter.emit_subelement_provenance(writer, prov.as_ref())?;
        let num_str = XmlWriter::<W>::format_u128(u128::from(*num), &mut num_buf);
        writer.empty_tag_with_attrs(
            "tuplet",
            &[("type", "start"), ("number", num_str), ("bracket", "yes")],
        )?;
    }
    for (num, prov) in &notations.tuplets_stop {
        emitter.emit_subelement_provenance(writer, prov.as_ref())?;
        let num_str = XmlWriter::<W>::format_u128(u128::from(*num), &mut num_buf);
        writer.empty_tag_with_attrs("tuplet", &[("type", "stop"), ("number", num_str)])?;
    }
    if notations.slide_start {
        writer.empty_tag_with_attrs("slide", &[("type", "start")])?;
    }
    if notations.slide_stop {
        writer.empty_tag_with_attrs("slide", &[("type", "stop")])?;
    }
    if !notations.ornaments.is_empty() {
        writer.open_tag("ornaments")?;
        for orn in &notations.ornaments {
            match orn {
                LoweredOrnament::TrillMark => writer.empty_tag("trill-mark")?,
                LoweredOrnament::TrillWithWavyStart => {
                    writer.empty_tag("trill-mark")?;
                    writer.empty_tag_with_attrs("wavy-line", &[("type", "start")])?;
                }
                LoweredOrnament::WavyStop => {
                    writer.empty_tag_with_attrs("wavy-line", &[("type", "stop")])?;
                }
                LoweredOrnament::Mordent => writer.empty_tag("mordent")?,
                LoweredOrnament::InvertedMordent => writer.empty_tag("inverted-mordent")?,
                LoweredOrnament::Turn { slash } => {
                    if *slash {
                        writer.empty_tag_with_attrs("turn", &[("slash", "yes")])?;
                    } else {
                        writer.empty_tag("turn")?;
                    }
                }
                LoweredOrnament::InvertedTurn { slash } => {
                    if *slash {
                        writer.empty_tag_with_attrs("inverted-turn", &[("slash", "yes")])?;
                    } else {
                        writer.empty_tag("inverted-turn")?;
                    }
                }
                LoweredOrnament::TremoloSingle(n) => {
                    writer.display_element_with_attrs("tremolo", &[("type", "single")], *n)?;
                }
            }
        }
        writer.close_tag("ornaments")?;
    }
    if !notations.technical.is_empty() {
        writer.open_tag("technical")?;
        for tech in &notations.technical {
            match tech {
                LoweredTechnical::UpBow => writer.empty_tag("up-bow")?,
                LoweredTechnical::DownBow => writer.empty_tag("down-bow")?,
                LoweredTechnical::OpenString => writer.empty_tag("open-string")?,
                LoweredTechnical::ThumbPosition => writer.empty_tag("thumb-position")?,
                LoweredTechnical::SnapPizzicato => writer.empty_tag("snap-pizzicato")?,
                LoweredTechnical::Stopped => writer.empty_tag("stopped")?,
                LoweredTechnical::Fingering(f) => writer.text_element("fingering", f)?,
            }
        }
        writer.close_tag("technical")?;
    }
    if !notations.articulations.is_empty() {
        writer.open_tag("articulations")?;
        for art in &notations.articulations {
            if *art == "strong-accent" {
                writer.empty_tag_with_attrs("strong-accent", &[("type", "up")])?;
            } else {
                writer.empty_tag(art)?;
            }
        }
        writer.close_tag("articulations")?;
    }
    if let Some(ferm) = notations.fermata {
        writer.empty_tag_with_attrs("fermata", &[("type", ferm)])?;
    }
    if notations.arpeggiate {
        writer.empty_tag("arpeggiate")?;
    }
    for other in &notations.other {
        writer.text_element_with_attrs("other-notation", &[("type", "single")], other)?;
    }
    writer.close_tag("notations")
}
