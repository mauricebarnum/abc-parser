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

//! Unit tests for ABC to `MusicXML` lowering.

use abc_parser::BarDurationPickupPolicy;
use abc_parser::IntoOwnedAst;
use abc_parser::parse;

use super::write_tune_musicxml;
use crate::AnnotationLevel;
use crate::MusicXmlOptions;

fn convert_first_tune(source: &str, options: &MusicXmlOptions) -> (String, usize) {
    let report = parse(source);
    assert!(report.is_valid(), "{:#?}", report.errors);
    let doc = report.output.unwrap().into_owned(source).unwrap();
    let tune = doc.tunes().next().unwrap();
    let mut xml = String::new();
    let warnings = write_tune_musicxml(&mut xml, tune, &doc.header, Some(source), options).unwrap();
    (xml, warnings.len())
}

#[test]
fn lowers_header_metadata_continuations_and_user_symbols() {
    let src = concat!(
        "H:First history\n",
        "+:second history\n",
        "U:T=!fermata!\n",
        "m: ~G2 = {A}G{F}G\n",
        "\n",
        "X:7\n",
        "T:Main Title\n",
        "T:Subtitle\n",
        "C:Trad.\n",
        "O:Ireland\n",
        "Z:Copyright 2026\n",
        "S:Session\n",
        "F:https://example.com/7.abc\n",
        "R:Reel\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:D\n",
        "TC D E F |\n",
        "W:Trailing words\n",
    );

    let options = MusicXmlOptions::new()
        .with_source_name("hdr.abc")
        .with_annotation_level(AnnotationLevel::Line);
    let (xml, warning_count) = convert_first_tune(src, &options);

    // Unexpanded `m:` macro emits 1 non-fatal advisory warning.
    assert_eq!(warning_count, 1);

    assert!(xml.contains("<movement-number>7</movement-number>"));
    assert!(xml.contains("<movement-title>Main Title</movement-title>"));
    assert!(xml.contains("<creator type=\"composer\">Trad.</creator>"));
    assert!(xml.contains("<rights>Copyright 2026</rights>"));
    assert!(xml.contains("<source>Session</source>"));
    assert!(xml.contains("<relation>https://example.com/7.abc</relation>"));
    assert!(xml.contains(
        "<!-- hdr.abc:1: H:First history -->\n      <!-- hdr.abc:2: +:second history -->\n      <miscellaneous-field name=\"history\">First history second history</miscellaneous-field>"
    ));
    assert!(xml.contains("<miscellaneous-field name=\"origin\">Ireland</miscellaneous-field>"));
    assert!(xml.contains("<miscellaneous-field name=\"rhythm\">Reel</miscellaneous-field>"));
    assert!(
        xml.contains("<miscellaneous-field name=\"words\">Trailing words</miscellaneous-field>")
    );
    assert!(xml.contains("<credit page=\"1\">\n    <credit-type>subtitle</credit-type>\n    <credit-words>Subtitle</credit-words>\n  </credit>"));
}

#[test]
fn segments_pickup_measures_repeats_variant_endings_and_multivoice_parts() {
    let src = concat!(
        "X:1\n",
        "T:Repeats and Voices\n",
        "M:4/4\n",
        "L:1/4\n",
        "V:1 name=\"Flute\" snm=\"Fl.\"\n",
        "V:2 name=\"Cello\" clef=bass\n",
        "K:G\n",
        "V:1\n",
        "D |: G A B c |1 d2 B2 :|2 g4 |]\n",
        "V:2\n",
        "z |: G,4 |1 D,4 :|2 G,,4 |]\n",
    );

    let options = MusicXmlOptions::new()
        .with_pickup_policy(BarDurationPickupPolicy::OpeningBar)
        .with_doctype(false);
    let (xml, warning_count) = convert_first_tune(src, &options);
    assert_eq!(warning_count, 0);

    // Pickup measure is implicit="yes" number="0", followed by measure 1 with forward repeat.
    assert!(xml.contains("<measure number=\"0\" implicit=\"yes\">"));
    assert!(xml.contains(
        "<measure number=\"1\">\n      <barline location=\"left\">\n        <bar-style>heavy-light</bar-style>\n        <repeat direction=\"forward\"/>\n      </barline>"
    ));
    // Ending 1 starts on left barline and stops with backward repeat on right barline.
    assert!(xml.contains("<ending number=\"1\" type=\"start\"/>"));
    assert!(xml.contains(
        "<barline location=\"right\">\n        <bar-style>light-heavy</bar-style>\n        <ending number=\"1\" type=\"stop\"/>\n        <repeat direction=\"backward\"/>\n      </barline>"
    ));
    // Ending 2 starts on left barline and discontinues at final thin-thick barline.
    assert!(xml.contains("<ending number=\"2\" type=\"start\"/>"));
    assert!(xml.contains(
        "<barline location=\"right\">\n        <bar-style>light-heavy</bar-style>\n        <ending number=\"2\" type=\"discontinue\"/>\n      </barline>"
    ));
    // Both parts P1 and P2 are present with their declared names and clefs.
    assert!(xml.contains("<score-part id=\"P1\">\n      <part-name>Flute</part-name>\n      <part-abbreviation>Fl.</part-abbreviation>\n    </score-part>"));
    assert!(
        xml.contains(
            "<score-part id=\"P2\">\n      <part-name>Cello</part-name>\n    </score-part>"
        )
    );
    assert!(xml.contains("<part id=\"P2\">"));
}

#[test]
fn lowers_grace_groups_with_acciaccatura_slashes_and_bagpipe_stems() {
    let src = concat!(
        "X:1\n",
        "T:Grace Groups\n",
        "M:4/4\n",
        "L:1/8\n",
        "K:HP\n",
        "{/g}a2 {cde}f2 {A>B}c4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Single acciaccatura has slash="yes", eighth type, bagpipe stem="up", and no duration.
    assert!(xml.contains("<grace slash=\"yes\"/>\n        <pitch>\n          <step>G</step>"));
    assert!(xml.contains("<type>eighth</type>\n        <stem>up</stem>"));

    // Multi-note grace group has <grace/> without slash, 16th type, and beams.
    assert!(xml.contains("<grace/>\n        <pitch>\n          <step>C</step>"));
    assert!(
        xml.contains("<beam number=\"1\">begin</beam>\n        <beam number=\"2\">begin</beam>")
    );
    assert!(xml.contains("<beam number=\"1\">end</beam>\n        <beam number=\"2\">end</beam>"));

    // Normal notes in bagpipe key have stem="down".
    assert!(xml.contains("<type>quarter</type>\n        <stem>down</stem>"));
}

#[test]
fn calculates_beams_across_consecutive_notes_and_breaks_on_whitespace() {
    let src = concat!(
        "X:1\n",
        "T:Beams\n",
        "M:4/4\n",
        "L:1/8\n",
        "K:C\n",
        "cdef cd ef c/d/ c3/2d/ |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // cdef has begin, continue, continue, end.
    assert!(xml.contains("<beam number=\"1\">begin</beam>"));
    assert!(xml.contains("<beam number=\"1\">continue</beam>"));
    assert!(xml.contains("<beam number=\"1\">end</beam>"));

    // c/d/ (16ths) has levels 1 and 2 begin/end.
    assert!(
        xml.contains("<beam number=\"1\">begin</beam>\n        <beam number=\"2\">begin</beam>")
    );

    // c3/2 d/ has beam 2 backward hook on d.
    assert!(xml.contains("<beam number=\"2\">backward hook</beam>"));
}

#[test]
fn tracks_nested_slurs_with_dotted_lines_and_lifo_closing() {
    let src = concat!(
        "X:1\n",
        "T:Slurs\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "(c .(d e.) f) |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Outer slur 1 starts on c and stops on f.
    assert!(xml.contains("<slur type=\"start\" number=\"1\"/>"));
    assert!(xml.contains("<slur type=\"stop\" number=\"1\"/>"));

    // Inner slur 2 is dotted, starts on d and stops on e.
    assert!(xml.contains("<slur type=\"start\" number=\"2\" line-type=\"dotted\"/>"));
    assert!(xml.contains("<slur type=\"stop\" number=\"2\" line-type=\"dotted\"/>"));
}

#[test]
fn maps_decorations_to_notations_and_directions() {
    let src = concat!(
        "X:1\n",
        "T:Decorations\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "!f! !trill! c !>! d !fermata! e2 !fermata! |]\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Dynamics emitted as direction before note c.
    assert!(xml.contains("<dynamics>\n            <f/>\n          </dynamics>"));
    // Ornaments and articulations inside notations.
    assert!(xml.contains("<trill-mark/>"));
    assert!(xml.contains("<accent/>"));
    assert!(xml.contains("<fermata type=\"upright\"/>"));
    // Fermata on the final barline.
    assert!(xml.contains(
        "<barline location=\"right\">\n        <bar-style>light-heavy</bar-style>\n        <fermata type=\"upright\"/>"
    ));
}

#[test]
fn lowers_chord_symbols_and_position_annotations_with_fallback() {
    let src = concat!(
        "X:1\n",
        "T:Harmonies and Annotations\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "\"Cm7/G\" c \"^Above\" d \"@10,20 Free\" e \"Slowly\" f |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));

    // One warning for unprefixed non-chord symbol "Slowly".
    assert_eq!(warnings, 1);

    // Harmony element emitted for Cm7/G.
    assert!(xml.contains(
        "<harmony>\n        <root>\n          <root-step>C</root-step>\n        </root>\n        <kind text=\"m7\">minor-seventh</kind>\n        <bass>\n          <bass-step>G</bass-step>\n        </bass>\n      </harmony>"
    ));

    // Positioned annotations emitted as directions.
    assert!(xml.contains(
        "<direction placement=\"above\">\n        <direction-type>\n          <words>Above</words>\n        </direction-type>\n      </direction>"
    ));
    assert!(xml.contains(
        "<direction-type>\n          <words relative-x=\"10\" relative-y=\"20\">Free</words>\n        </direction-type>"
    ));
    assert!(xml.contains(
        "<direction placement=\"above\">\n        <direction-type>\n          <words>Slowly</words>\n        </direction-type>\n      </direction>"
    ));
}

#[test]
fn lowers_tempo_metronome_marks_and_text_indications() {
    let src = concat!(
        "X:1\n",
        "T:Tempo\n",
        "Q:\"Allegro\" 1/4=120\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    assert!(xml.contains("<words>Allegro</words>"));
    assert!(xml.contains("<beat-unit>quarter</beat-unit>"));
    assert!(xml.contains("<per-minute>120</per-minute>"));
    assert!(xml.contains("<sound tempo=\"120\"/>"));
}

#[test]
fn lowers_lyrics_alignment_verses_and_melisma() {
    let src = concat!(
        "X:1\n",
        "T:Lyrics\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f | g a b c' |\n",
        "w: Sing- ing a song | _ to\\-day * all\n",
        "w: Verse two words~here _ | _ * end- ing\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Verse 1 syllable breakdown:
    // 'c': begin 'Sing'
    // 'd': end 'ing'
    // 'e': single 'a'
    // 'f': single 'song' with extend start
    // 'g': extend stop
    // 'a': single 'to-day'
    // 'b': skip (no lyric for verse 1)
    // 'c'': single 'all'
    assert!(xml.contains(
        "<lyric number=\"1\">\n          <syllabic>begin</syllabic>\n          <text>Sing</text>\n        </lyric>"
    ));
    assert!(xml.contains(
        "<lyric number=\"1\">\n          <syllabic>end</syllabic>\n          <text>ing</text>\n        </lyric>"
    ));
    assert!(xml.contains(
        "<lyric number=\"1\">\n          <syllabic>single</syllabic>\n          <text>a</text>\n        </lyric>"
    ));
    assert!(xml.contains(
        "<lyric number=\"1\">\n          <syllabic>single</syllabic>\n          <text>song</text>\n          <extend type=\"start\"/>\n        </lyric>"
    ));
    assert!(
        xml.contains("<lyric number=\"1\">\n          <extend type=\"stop\"/>\n        </lyric>")
    );
    assert!(xml.contains(
        "<lyric number=\"1\">\n          <syllabic>single</syllabic>\n          <text>to-day</text>\n        </lyric>"
    ));

    // Verse 2 non-breaking space and melisma continuation:
    // 'c': single 'Verse'
    // 'd': single 'two'
    // 'e': single 'words\u{a0}here' with extend start
    // 'f': extend continue
    // 'g': extend stop
    // 'b': begin 'end'
    // 'c'': end 'ing'
    assert!(xml.contains(
        "<lyric number=\"2\">\n          <syllabic>single</syllabic>\n          <text>words\u{a0}here</text>\n          <extend type=\"start\"/>\n        </lyric>"
    ));
    assert!(
        xml.contains(
            "<lyric number=\"2\">\n          <extend type=\"continue\"/>\n        </lyric>"
        )
    );
    assert!(
        xml.contains("<lyric number=\"2\">\n          <extend type=\"stop\"/>\n        </lyric>")
    );
    assert!(xml.contains(
        "<lyric number=\"2\">\n          <syllabic>begin</syllabic>\n          <text>end</text>\n        </lyric>"
    ));
    assert!(xml.contains(
        "<lyric number=\"2\">\n          <syllabic>end</syllabic>\n          <text>ing</text>\n        </lyric>"
    ));
}

#[test]
fn lowers_symbol_lines_decorations_and_chords() {
    let src = concat!(
        "X:1\n",
        "T:Symbols\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f |\n",
        "s: !f! . \"C\" H |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // 'c': !f! dynamic direction inserted before note
    assert!(xml.contains("<direction placement=\"below\">\n        <direction-type>\n          <dynamics>\n            <f/>\n          </dynamics>\n        </direction-type>\n      </direction>"));
    // 'd': . staccato articulation attached to note
    assert!(xml.contains("<notations>\n          <articulations>\n            <staccato/>\n          </articulations>\n        </notations>"));
    // 'e': "C" harmony inserted before note
    assert!(xml.contains("<harmony>\n        <root>\n          <root-step>C</root-step>\n        </root>\n        <kind>major</kind>\n      </harmony>"));
    // 'f': H fermata notation attached to note
    assert!(
        xml.contains("<notations>\n          <fermata type=\"upright\"/>\n        </notations>")
    );
}

#[test]
fn lowers_voice_overlays_with_backup_and_sequential_voices() {
    let src = concat!(
        "X:1\n",
        "T:Voice Overlays\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f & c2 e2 | g4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Voice 1 notes in Measure 1 have voice 1
    assert!(xml.contains("<voice>1</voice>"));
    // Backup rewinds duration: 4 quarter notes = 4 divisions
    assert!(xml.contains("<backup>\n        <duration>4</duration>\n      </backup>"));
    // Overlay notes have voice 2
    assert!(xml.contains("<voice>2</voice>"));
}

#[test]
fn preserves_lyrics_and_overlay_provenance() {
    let src = concat!(
        "X:1\n",
        "T:Lyrics and Overlays Provenance\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c d e f & c2 e2 |\n",
        "w: do re mi fa\n",
    );
    let options = MusicXmlOptions::new()
        .with_source_name("test.abc")
        .with_annotation_level(AnnotationLevel::Verbose)
        .with_doctype(false);
    let (xml, warnings) = convert_first_tune(src, &options);
    assert_eq!(warnings, 0);

    // Level 1: Measure emits line 6 (music line) and line 7 (lyrics line)
    assert!(xml.contains("<!-- test.abc:6: c d e f & c2 e2 | -->"));
    assert!(xml.contains("<!-- test.abc:7: w: do re mi fa -->"));

    // Level 2: Overlay backup emits span comment
    assert!(xml.contains(": & -->\n      <backup>"));

    // Level 2: Lyric elements emit sub-element span comments
    assert!(xml.contains(": do -->\n        <lyric number=\"1\">"));
}

#[test]
fn lowers_score_directive_single_staff_merged_voices() {
    let src = concat!(
        "X:1\n",
        "T:Merged Staff\n",
        "%%score (T1 T2)\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:T1 name=\"Tenore I\" snm=\"T.I\"\n",
        "c d e f |\n",
        "V:T2 name=\"Tenore II\" snm=\"T.II\"\n",
        "c2 e2 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // A single score-part should be emitted with merged name and abbreviation
    assert!(xml.contains("<score-part id=\"P1\">"));
    assert!(!xml.contains("<score-part id=\"P2\">"));
    assert!(xml.contains("<part-name>Tenore I / Tenore II</part-name>"));
    assert!(xml.contains("<part-abbreviation>T.I / T.II</part-abbreviation>"));

    // Single part element
    assert!(xml.contains("<part id=\"P1\">"));
    assert!(!xml.contains("<part id=\"P2\">"));

    // Measure contains T1 on voice 1, backup of 4 divisions, and T2 on voice 2
    assert!(xml.contains("<voice>1</voice>"));
    assert!(xml.contains("<backup>\n        <duration>4</duration>\n      </backup>"));
    assert!(xml.contains("<voice>2</voice>"));
}

#[test]
fn lowers_score_directive_part_groups_brackets_and_braces() {
    let src = concat!(
        "X:1\n",
        "T:Groups\n",
        "%%score Solo [(S A) (T B)] {RH | LH}\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:Solo\n",
        "c4 |\n",
        "V:S\n",
        "c4 |\n",
        "V:A\n",
        "e4 |\n",
        "V:T\n",
        "g4 |\n",
        "V:B\n",
        "c4 |\n",
        "V:RH\n",
        "g4 |\n",
        "V:LH\n",
        "c4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);

    // Solo is P1 outside groups
    // [(S A) (T B)] is bracket group number 1 around P2 (S / A) and P3 (T / B)
    // In %%score, no barline separator within bracket means group-barline is no
    // {RH | LH} is lowered into single keyboard multi-staff part P4 with <staves>2</staves> and <part-symbol>brace</part-symbol>
    assert!(xml.contains(
        "<part-group type=\"start\" number=\"1\">\n      <group-symbol>bracket</group-symbol>\n      <group-barline>no</group-barline>\n    </part-group>"
    ));
    assert!(xml.contains("<part-group type=\"stop\" number=\"1\"/>"));
    assert!(xml.contains("<staves>2</staves>"));
    assert!(xml.contains("<part-symbol>brace</part-symbol>"));
    assert!(xml.contains("<staff>1</staff>"));
    assert!(xml.contains("<staff>2</staff>"));
}

#[test]
fn lowers_staves_directive_barline_inversion() {
    // In %%staves, default barline connects staves (barline = yes), and '|' disconnects (barline = no)
    let src_connected = concat!(
        "X:1\n",
        "T:Staves Connected\n",
        "%%staves [V1 V2]\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c4 |\n",
        "V:V2\n",
        "g4 |\n",
    );
    let (xml_conn, warnings) =
        convert_first_tune(src_connected, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(xml_conn.contains("<group-barline>yes</group-barline>"));

    let src_disconnected = concat!(
        "X:1\n",
        "T:Staves Disconnected\n",
        "%%staves [V1 | V2]\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c4 |\n",
        "V:V2\n",
        "g4 |\n",
    );
    let (xml_disc, warnings) = convert_first_tune(
        src_disconnected,
        &MusicXmlOptions::new().with_doctype(false),
    );
    assert_eq!(warnings, 0);
    assert!(xml_disc.contains("<group-barline>no</group-barline>"));
}

#[test]
fn lowers_score_directive_omits_unlisted_voices_with_warning() {
    let src = concat!(
        "X:1\n",
        "T:Omit Unlisted\n",
        "%%score V1\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c4 |\n",
        "V:V2\n",
        "g4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 1);

    // V1 is present in part-list and part
    assert!(xml.contains("<score-part id=\"P1\">"));
    assert!(xml.contains("<part id=\"P1\">"));
    // V2 is omitted
    assert!(!xml.contains("P2"));
    assert!(!xml.contains("V2"));
}

#[test]
fn preserves_score_directive_provenance() {
    let src = concat!(
        "X:1\n",
        "T:Score Directive Provenance\n",
        "%%score (V1 V2)\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c4 |\n",
        "V:V2\n",
        "e4 |\n",
    );
    let options = MusicXmlOptions::new()
        .with_source_name("score.abc")
        .with_annotation_level(AnnotationLevel::Line)
        .with_doctype(false);
    let (xml, warnings) = convert_first_tune(src, &options);
    assert_eq!(warnings, 0);

    // Level 1: score directive line is emitted before part-list (percent-encoded in XML comments)
    assert!(xml.contains("<!-- score.abc:3: %25%25score (V1 V2) -->\n  <part-list>"));
}

#[test]
fn lowers_compound_duration_chords_as_tied_segments() {
    let src = "X:1\nT:Compound Chord\nM:5/8\nL:1/8\nK:C\n[CEG]5 |\n";
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // 5 eighths with divisions=2 (for quarter note) splits into half (4 ticks) tied to eighth (1 tick).
    assert!(xml.contains("<divisions>2</divisions>"));
    assert!(xml.contains("<type>half</type>"));
    assert!(xml.contains("<type>eighth</type>"));
    assert!(xml.contains("<tie type=\"start\"/>"));
    assert!(xml.contains("<tie type=\"stop\"/>"));
}

#[test]
fn preserves_grand_staff_duration_when_overlay_contains_rests() {
    let src = concat!(
        "X:1\n",
        "T:Overlay Rest Measure Duration\n",
        "%%score {V1 | V2}\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c2 z2 & e2 z2 |\n",
        "V:V2\n",
        "C4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // Backup between V1 and V2 must be 4 quarter-note ticks, not 6 (from double-counted rests).
    assert!(xml.contains("<backup>\n        <duration>4</duration>\n      </backup>"));
}

#[test]
fn updates_divisions_when_padding_trailing_measures() {
    let src = concat!(
        "X:1\n",
        "T:Sync Divisions\n",
        "M:3/8\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "c |\n",
        "V:V2\n",
        "C D E | F G A |\n",
    );
    let (xml, _warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    let p1 = xml.split("<part id=\"P2\">").next().unwrap();
    // V1 had only 1 quarter note (divisions=1), then padded with a 3/8 rest requiring divisions=2.
    assert!(p1.contains("<divisions>2</divisions>"));
    // Padded measure rest must have duration 3 (3 eighths at divisions=2), not 0!
    assert!(p1.contains("<rest measure=\"yes\"/>\n        <duration>3</duration>"));
}

#[test]
fn header_tempo_does_not_shift_first_measure_lyric_slots() {
    let src = concat!(
        "X:1\n",
        "T:Tempo And Lyrics\n",
        "M:4/4\n",
        "L:1/4\n",
        "Q:1/4=120\n",
        "K:C\n",
        "C D E F |\n",
        "w: Ho ly! is good\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(xml.contains("<text>Ho</text>"));
    assert!(xml.contains("<text>ly!</text>"));
    assert!(xml.contains("<text>is</text>"));
    assert!(xml.contains("<text>good</text>"));
}

#[test]
fn tied_notes_receive_lyric_slots_for_extend_and_skip() {
    let src_extend = concat!(
        "X:1\n",
        "T:Tied Extend\n",
        "M:3/4\n",
        "L:1/4\n",
        "K:C\n",
        "c- c d |\n",
        "w: one _ two\n",
    );
    let (xml_ext, warnings) =
        convert_first_tune(src_extend, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(xml_ext.contains("<text>one</text>"));
    assert!(xml_ext.contains("<extend type=\"start\"/>"));
    assert!(xml_ext.contains("<extend type=\"stop\"/>"));
    assert!(xml_ext.contains("<text>two</text>"));

    let src_skip = concat!(
        "X:1\n",
        "T:Tied Skip\n",
        "M:3/4\n",
        "L:1/4\n",
        "K:C\n",
        "c- c d |\n",
        "w: one * two\n",
    );
    let (xml_skip, warnings) =
        convert_first_tune(src_skip, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(xml_skip.contains("<text>one</text>"));
    assert!(xml_skip.contains("<text>two</text>"));
    // Second note `c` has the `*` skip, so `two` lands on `d` (`<step>D</step>`), not `c`.
    let d_pos = xml_skip.find("<step>D</step>").unwrap();
    let two_pos = xml_skip.find("<text>two</text>").unwrap();
    assert!(two_pos > d_pos);
}

#[test]
fn trailing_comments_are_excluded_from_lyrics_and_symbols() {
    let src = concat!(
        "X:1\n",
        "T:Trailing Comments\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "C D E F |\n",
        "w: do re mi fa % comment should not appear\n",
        "s: !f! * * * % comment symbol\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(!xml.contains("comment"));
    assert!(!xml.contains("should"));
    assert!(xml.contains("<text>fa</text>"));
    assert!(xml.contains("<f/>"));
}

#[test]
fn grace_notes_preserve_broken_rhythms_and_dots() {
    let src = concat!(
        "X:1\n",
        "T:Grace Dots\n",
        "M:4/4\n",
        "L:1/8\n",
        "K:C\n",
        "{A>B}c2 {A3/2B/2}d2 z4 |\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // Both grace groups have a dotted eighth followed by a 16th grace note.
    assert_eq!(xml.matches("<grace/>\n        <chord/>").count(), 0);
    assert!(xml.contains(
        "<grace/>\n        <pitch>\n          <step>A</step>\n          <octave>4</octave>\n        </pitch>\n        <voice>1</voice>\n        <type>eighth</type>\n        <dot/>"
    ));
    assert!(xml.contains(
        "<grace/>\n        <pitch>\n          <step>B</step>\n          <octave>4</octave>\n        </pitch>\n        <voice>1</voice>\n        <type>16th</type>"
    ));
}

#[test]
fn measure_leading_meter_and_key_apply_to_tuplets_and_multi_measure_rests() {
    let src_tuplet = concat!(
        "X:1\n",
        "T:Leading Meter Tuplet\n",
        "M:4/4\n",
        "L:1/8\n",
        "K:C\n",
        "c8 |\n",
        "[M:6/8] (5cdefg c |\n",
    );
    let (xml_tup, warnings) =
        convert_first_tune(src_tuplet, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // In 6/8 (compound meter), (5 uses 5:3.
    assert!(
        xml_tup
            .contains("<actual-notes>5</actual-notes>\n          <normal-notes>3</normal-notes>")
    );

    let src_mm = concat!(
        "X:1\n",
        "T:Leading Meter MMRest\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "c4 |\n",
        "[M:3/4] [K:G] Z2 |\n",
    );
    let (xml_mm, warnings) =
        convert_first_tune(src_mm, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    assert!(xml_mm.contains("<beats>3</beats>\n          <beat-type>4</beat-type>"));
    assert!(xml_mm.contains("<fifths>1</fifths>"));
    assert!(xml_mm.contains("<multiple-rest>2</multiple-rest>"));
    assert!(xml_mm.contains("<rest measure=\"yes\"/>\n        <duration>3</duration>"));
}

#[test]
fn merged_staff_isolates_ties_unit_length_and_lyrics_per_voice() {
    let src = concat!(
        "X:1\n",
        "T:Merged Voice Isolation\n",
        "%%score (V1 V2)\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "V:V1\n",
        "[L:1/8] c2 c2 c2 c c- | c8 |\n",
        "w: a b c d e | f\n",
        "V:V2\n",
        "c c c c | G4 |\n",
        "w: w x y z | q\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // Measure 1 has V1's tie start on the last eighth `c-` of bar 1; V2's `c` in bar 1
    // must NOT consume V1's tie across the barline; instead V1's `c8` in bar 2 stops the tie.
    let m2_start = xml.find("<measure number=\"2\">").unwrap();
    let m2 = &xml[m2_start..];
    assert!(m2.contains("<tie type=\"stop\"/>"));
    assert!(m2.contains("<type>whole</type>"));
    let g_pos = m2.find("<step>G</step>").unwrap();
    assert!(m2[g_pos..].contains("<type>whole</type>"));
    // V2's lyric `w` lands on `<voice>2</voice>` in measure 1, while `a` lands on `<voice>1</voice>`.
    let m1 = &xml[..m2_start];
    let v2_pos = m1.find("<voice>2</voice>").unwrap();
    let a_pos = m1.find("<text>a</text>").unwrap();
    let w_pos = m1.find("<text>w</text>").unwrap();
    assert!(a_pos < v2_pos);
    assert!(w_pos > v2_pos);
    assert!(m2.contains("<text>f</text>"));
    assert!(m2.contains("<text>q</text>"));
}

#[test]
fn single_barline_with_fermata_keeps_variant_ending_open() {
    let src = concat!(
        "X:1\n",
        "T:Fermata Ending\n",
        "M:4/4\n",
        "L:1/4\n",
        "K:C\n",
        "|: C4 |1 c4 !fermata! | d4 :|2 e4 |]\n",
    );
    let (xml, warnings) = convert_first_tune(src, &MusicXmlOptions::new().with_doctype(false));
    assert_eq!(warnings, 0);
    // Ending 1 starts at measure 2, stays open across the fermata barline, and stops at measure 3's :|.
    assert!(xml.contains("<ending number=\"1\" type=\"start\"/>"));
    assert!(!xml.contains("<ending number=\"1\" type=\"discontinue\"/>"));
    assert!(xml.contains("<ending number=\"1\" type=\"stop\"/>"));
    assert!(xml.contains("<ending number=\"2\" type=\"start\"/>"));
}
