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

//! Intermediate and lowered AST representation types for ABC to `MusicXML` conversion.

use std::collections::BTreeMap;

use abc_parser::Annotation;
use abc_parser::BrokenRhythm;
use abc_parser::Chord;
use abc_parser::Decoration;
use abc_parser::Fraction;
use abc_parser::GraceGroup;
use abc_parser::KeySignature;
use abc_parser::Meter;
use abc_parser::MultiMeasureRest;
use abc_parser::MusicElement;
use abc_parser::Note;
use abc_parser::Overlay;
use abc_parser::PitchClass;
use abc_parser::Rest;
use abc_parser::SimpleSpan;
use abc_parser::Slur;
use abc_parser::Tempo;
use abc_parser::Tie;
use abc_parser::Tuplet;
use abc_parser::pitch::Offset;

use crate::harmony::HarmonySpec;
use crate::lyrics::LyricToken;
use crate::lyrics::Syllabic;
use crate::lyrics::SymbolToken;
use crate::pitch::AccidentalPropagation;
use crate::pitch::ClefSpec;
use crate::pitch::KeyKind;
use crate::pitch::ResolvedKey;
use crate::pitch::TransposeSpec;
use crate::pitch::resolve_key_signature;
use crate::score_directive::PartListEntry;
use crate::score_directive::ScoreDirectiveKind;
use crate::source_map::SourceProvenance;
use crate::timing::Duration;

/// Span alias matching the ABC parser span type.
pub type Span = SimpleSpan<usize>;

/// Logical voice layer key isolating merged voices (`%%score (V1 V2)`) and `&` overlays.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct VoiceLayerKey {
    /// Zero-based index of the primary voice inside a merged staff (`0` for single voice or V1).
    pub primary_layer: usize,
    /// Zero-based `&` overlay subvoice offset within `primary_layer`.
    pub subvoice: u32,
}

impl VoiceLayerKey {
    /// Creates a new logical voice layer key.
    #[must_use]
    pub const fn new(primary_layer: usize, subvoice: u32) -> Self {
        Self {
            primary_layer,
            subvoice,
        }
    }
}

/// Pitch and octave lookup key for tracking open ties across measure and chord boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PitchOctaveKey {
    /// Owning logical voice layer.
    pub voice: VoiceLayerKey,
    /// Zero-based chromatic pitch class index (`0..=6`).
    pub class_idx: usize,
    /// Octave offset where `0` corresponds to middle octave.
    pub octave: i8,
}

impl PitchOctaveKey {
    /// Creates a new pitch and octave key.
    #[must_use]
    pub const fn new(voice: VoiceLayerKey, class_idx: usize, octave: i8) -> Self {
        Self {
            voice,
            class_idx,
            octave,
        }
    }
}

/// Endpoint state for spanners such as slurs, ties, and lyric melismas.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpannerEndpoint {
    /// Spanner begins at this element.
    Start,
    /// Spanner ends at this element.
    Stop,
    /// Spanner continues across this element.
    Continue,
}

impl SpannerEndpoint {
    /// Returns the standard `MusicXML` XML attribute value string.
    #[must_use]
    pub const fn as_xml_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Continue => "continue",
        }
    }
}

/// Descriptor identifying an active or pending slur spanner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlurDescriptor {
    /// Slur stack number (`1..=16`).
    pub number: u8,
    /// Whether the slur is styled as dotted/dashed.
    pub dotted: bool,
    /// Optional source provenance for the slur token.
    pub provenance: Option<SourceProvenance>,
}

impl SlurDescriptor {
    /// Creates a new slur descriptor.
    #[must_use]
    pub const fn new(number: u8, dotted: bool, provenance: Option<SourceProvenance>) -> Self {
        Self {
            number,
            dotted,
            provenance,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextMetadataItem {
    pub text: String,
    pub provenance: SourceProvenance,
}

#[derive(Clone, Debug)]
pub struct MiscFieldItem {
    pub name: &'static str,
    pub text: String,
    pub provenance: SourceProvenance,
}

#[derive(Clone, Debug)]
pub struct CreditItem {
    pub credit_type: Option<&'static str>,
    pub words: String,
    pub justify: Option<&'static str>,
    pub provenance: SourceProvenance,
}

#[derive(Clone, Debug, Default)]
pub struct LoweredHeader {
    pub reference: Option<(u32, SourceProvenance)>,
    pub movement_title: Option<TextMetadataItem>,
    pub composers: Vec<TextMetadataItem>,
    pub rights: Vec<TextMetadataItem>,
    pub encoders: Vec<TextMetadataItem>,
    pub sources: Vec<TextMetadataItem>,
    pub relations: Vec<TextMetadataItem>,
    pub misc_fields: Vec<MiscFieldItem>,
    pub credits: Vec<CreditItem>,
    pub typesetting_comments: Vec<String>,
}

/// `MusicXML` 4.0 `<midi-instrument>` specification inside `<score-part>`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MidiInstrumentSpec {
    /// 1-based MIDI channel (1..=16).
    pub channel: Option<u8>,
    /// 1-based MIDI patch/program number (1..=128).
    pub program: Option<u8>,
    /// Output volume (0..=127).
    pub volume: Option<u8>,
    /// Stereo panning (-180..=180 or -90..=90).
    pub pan: Option<i16>,
}

impl MidiInstrumentSpec {
    /// Returns true if no MIDI properties are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.channel.is_none()
            && self.program.is_none()
            && self.volume.is_none()
            && self.pan.is_none()
    }
}

#[derive(Clone, Debug)]
pub struct LoweredPart {
    pub part_id: String,
    pub part_name: String,
    pub part_abbreviation: Option<String>,
    pub declaration_provenance: Option<SourceProvenance>,
    pub divisions: u128,
    pub measures: Vec<LoweredMeasure>,
    pub midi_instrument: Option<MidiInstrumentSpec>,
}

#[derive(Clone, Debug)]
pub struct LoweredMeasure {
    pub number: String,
    pub implicit: bool,
    pub provenance: Option<SourceProvenance>,
    pub left_barline: Option<LoweredBarline>,
    pub attributes: Option<LoweredAttributes>,
    pub items: Vec<MeasureItem>,
    pub right_barline: Option<LoweredBarline>,
    pub actual_duration: Duration,
    pub expected_duration: Option<Duration>,
    pub new_system: bool,
    pub new_page: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LoweredAttributes {
    pub divisions: Option<(u128, Option<SourceProvenance>)>,
    pub key: Option<(KeyKind, Option<SourceProvenance>)>,
    pub time: Option<(Meter, Option<SourceProvenance>)>,
    pub clef: Option<(ClefSpec, Option<SourceProvenance>)>,
    pub clefs: Vec<(ClefSpec, Option<u8>, Option<SourceProvenance>)>,
    pub staves: Option<u8>,
    pub part_symbol: Option<&'static str>,
    pub transpose: Option<(TransposeSpec, Option<SourceProvenance>)>,
    pub multiple_rest: Option<(u32, Option<SourceProvenance>)>,
}

impl LoweredAttributes {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.divisions.is_none()
            && self.key.is_none()
            && self.time.is_none()
            && self.clef.is_none()
            && self.clefs.is_empty()
            && self.staves.is_none()
            && self.part_symbol.is_none()
            && self.transpose.is_none()
            && self.multiple_rest.is_none()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndingType {
    Start,
    Stop,
    Discontinue,
}

impl EndingType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Discontinue => "discontinue",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoweredEnding {
    pub number: String,
    pub ending_type: EndingType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeatDirection {
    Forward,
    Backward,
}

impl RepeatDirection {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LoweredBarline {
    pub location: &'static str,
    pub bar_style: Option<&'static str>,
    pub fermata: Option<(&'static str, Option<SourceProvenance>)>,
    pub ending: Option<LoweredEnding>,
    pub repeat: Option<(RepeatDirection, Option<u32>)>,
    pub provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug)]
pub struct LoweredBeam {
    pub number: u8,
    pub value: &'static str,
}

#[derive(Clone, Debug)]
pub struct LoweredSlur {
    pub slur_type: SpannerEndpoint,
    pub number: u8,
    pub dotted: bool,
    pub provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug)]
pub enum LoweredOrnament {
    TrillMark,
    TrillWithWavyStart,
    WavyStop,
    Mordent,
    InvertedMordent,
    Turn { slash: bool },
    InvertedTurn { slash: bool },
    TremoloSingle(u8),
}

#[derive(Clone, Debug)]
pub enum LoweredTechnical {
    UpBow,
    DownBow,
    OpenString,
    ThumbPosition,
    SnapPizzicato,
    Stopped,
    Fingering(String),
}

#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct LoweredNotations {
    pub tied_stop: bool,
    pub tied_start: bool,
    pub tied_dotted: bool,
    pub tie_provenance: Option<SourceProvenance>,
    pub slurs: Vec<LoweredSlur>,
    pub tuplets_start: Vec<(u8, Option<SourceProvenance>)>,
    pub tuplets_stop: Vec<(u8, Option<SourceProvenance>)>,
    pub articulations: Vec<&'static str>,
    pub ornaments: Vec<LoweredOrnament>,
    pub technical: Vec<LoweredTechnical>,
    pub fermata: Option<&'static str>,
    pub arpeggiate: bool,
    pub slide_start: bool,
    pub slide_stop: bool,
    pub other: Vec<String>,
}

impl LoweredNotations {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        !self.tied_stop
            && !self.tied_start
            && self.slurs.is_empty()
            && self.tuplets_start.is_empty()
            && self.tuplets_stop.is_empty()
            && self.articulations.is_empty()
            && self.ornaments.is_empty()
            && self.technical.is_empty()
            && self.fermata.is_none()
            && !self.arpeggiate
            && !self.slide_start
            && !self.slide_stop
            && self.other.is_empty()
    }

    pub fn merge(&mut self, other: Self) {
        if other.tied_stop {
            self.tied_stop = true;
        }
        if other.tied_start {
            self.tied_start = true;
        }
        if other.tied_dotted {
            self.tied_dotted = true;
        }
        if other.tie_provenance.is_some() {
            self.tie_provenance = other.tie_provenance;
        }
        self.slurs.extend(other.slurs);
        self.tuplets_start.extend(other.tuplets_start);
        self.tuplets_stop.extend(other.tuplets_stop);
        self.articulations.extend(other.articulations);
        self.ornaments.extend(other.ornaments);
        self.technical.extend(other.technical);
        if other.fermata.is_some() {
            self.fermata = other.fermata;
        }
        if other.arpeggiate {
            self.arpeggiate = true;
        }
        if other.slide_start {
            self.slide_start = true;
        }
        if other.slide_stop {
            self.slide_stop = true;
        }
        self.other.extend(other.other);
    }
}

#[derive(Clone, Debug)]
pub struct LoweredLyric {
    pub number: u32,
    pub syllabic: Option<Syllabic>,
    pub text: Option<String>,
    pub extend: Option<SpannerEndpoint>,
    pub provenance: Option<SourceProvenance>,
}

/// Style of a lowered grace note (`<grace slash="yes"/>` vs `<grace/>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraceNoteStyle {
    /// Acciaccatura (slashed grace note).
    Acciaccatura,
    /// Appoggiatura (unslashed grace note).
    Appoggiatura,
}

#[derive(Clone, Debug)]
pub struct LoweredNote {
    pub print_object: bool,
    pub grace: Option<GraceNoteStyle>,
    pub is_chord_member: bool,
    pub kind: LoweredNoteKind,
    pub sounding_duration: Duration,
    pub voice: u32,
    pub voice_key: VoiceLayerKey,
    pub staff: Option<u8>,
    pub note_type: Option<&'static str>,
    pub dots: u8,
    pub time_modification: Option<(u8, u8)>,
    pub stem: Option<&'static str>,
    pub beams: Vec<LoweredBeam>,
    pub notations: Option<Box<LoweredNotations>>,
    pub lyrics: Vec<LoweredLyric>,
    pub provenance: Option<SourceProvenance>,
}

impl LoweredNote {
    /// Returns `true` if this note is a grace note.
    #[must_use]
    pub const fn is_grace(&self) -> bool {
        self.grace.is_some()
    }

    /// Boxes `notations` only when non-empty so plain notes avoid `Vec` field overhead.
    #[must_use]
    pub fn from_notations(notations: LoweredNotations) -> Option<Box<LoweredNotations>> {
        (!notations.is_empty()).then(|| Box::new(notations))
    }

    /// Returns a reference to this note's [`LoweredNotations`], if non-empty.
    #[must_use]
    pub fn notations(&self) -> Option<&LoweredNotations> {
        self.notations.as_deref()
    }

    /// Returns a mutable handle to this note's [`LoweredNotations`], allocating if absent.
    pub fn notations_mut(&mut self) -> &mut LoweredNotations {
        self.notations
            .get_or_insert_with(|| Box::new(LoweredNotations::default()))
    }

    /// Constructs a full-measure rest note.
    #[must_use]
    pub const fn measure_rest(
        print_object: bool,
        sounding_duration: Duration,
        voice: u32,
        staff: Option<u8>,
        provenance: Option<SourceProvenance>,
    ) -> Self {
        Self {
            print_object,
            grace: None,
            is_chord_member: false,
            kind: LoweredNoteKind::Rest { measure: true },
            sounding_duration,
            voice,
            voice_key: VoiceLayerKey::new(0, 0),
            staff,
            note_type: None,
            dots: 0,
            time_modification: None,
            stem: None,
            beams: Vec::new(),
            notations: None,
            lyrics: Vec::new(),
            provenance,
        }
    }

    /// Constructs a timed (non-measure) rest note.
    #[must_use]
    pub const fn timed_rest(
        print_object: bool,
        sounding_duration: Duration,
        voice: u32,
        note_type: Option<&'static str>,
        dots: u8,
        time_modification: Option<(u8, u8)>,
        provenance: Option<SourceProvenance>,
    ) -> Self {
        Self {
            print_object,
            grace: None,
            is_chord_member: false,
            kind: LoweredNoteKind::Rest { measure: false },
            sounding_duration,
            voice,
            voice_key: VoiceLayerKey::new(0, 0),
            staff: None,
            note_type,
            dots,
            time_modification,
            stem: None,
            beams: Vec::new(),
            notations: None,
            lyrics: Vec::new(),
            provenance,
        }
    }
}

#[derive(Clone, Debug)]
pub enum LoweredNoteKind {
    Pitched {
        step: PitchClass,
        octave: i32,
        alter: Offset,
        accidental: Option<&'static str>,
    },
    Unpitched {
        display_step: PitchClass,
        display_octave: i32,
    },
    Rest {
        measure: bool,
    },
}

/// Optional `<sound .../>` metadata carried by a [`LoweredDirection`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoweredSound {
    Tempo(u32),
    Segno(&'static str),
    Coda(&'static str),
    DalSegno(&'static str),
    DaCapo,
    ToCoda(&'static str),
    Fine,
}

#[derive(Clone, Debug)]
pub struct LoweredDirection {
    pub placement: Option<&'static str>,
    pub kind: LoweredDirectionKind,
    pub sound: Option<LoweredSound>,
    pub provenance: Option<SourceProvenance>,
}

impl LoweredDirection {
    /// Creates a direction with no attached `<sound>` element.
    #[must_use]
    pub const fn new(
        placement: Option<&'static str>,
        kind: LoweredDirectionKind,
        provenance: Option<SourceProvenance>,
    ) -> Self {
        Self {
            placement,
            kind,
            sound: None,
            provenance,
        }
    }

    /// Attaches a `<sound>` specification to this direction.
    #[must_use]
    pub const fn with_sound(mut self, sound: LoweredSound) -> Self {
        self.sound = Some(sound);
        self
    }

    /// Convenience constructor for plain `<words>` text directions.
    #[must_use]
    pub const fn words(
        placement: Option<&'static str>,
        text: String,
        justify: Option<&'static str>,
        relative_x: Option<i32>,
        relative_y: Option<i32>,
        provenance: Option<SourceProvenance>,
    ) -> Self {
        Self::new(
            placement,
            LoweredDirectionKind::Words {
                text,
                justify,
                relative_x,
                relative_y,
            },
            provenance,
        )
    }
}

#[derive(Clone, Debug)]
pub enum LoweredDirectionKind {
    Dynamics(&'static str),
    Wedge(&'static str),
    Words {
        text: String,
        justify: Option<&'static str>,
        relative_x: Option<i32>,
        relative_y: Option<i32>,
    },
    Metronome {
        prelude: Option<String>,
        beat_unit: &'static str,
        dots: u8,
        bpm: u32,
        postlude: Option<String>,
    },
    Segno,
    Coda,
    OctaveShift {
        shift_type: &'static str,
        size: u8,
    },
}

#[derive(Clone, Debug)]
pub struct LoweredHarmony {
    pub spec: HarmonySpec,
    pub provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug)]
pub enum MeasureItem {
    Backup {
        duration: Duration,
        provenance: Option<SourceProvenance>,
    },
    Direction(LoweredDirection),
    Harmony(LoweredHarmony),
    Note(LoweredNote),
}

#[derive(Clone, Debug)]
pub struct LyricsField {
    pub verse: u32,
    pub tokens: Vec<LyricToken>,
    pub line_span: Span,
}

#[derive(Clone, Debug)]
pub struct SymbolsField {
    pub layer: usize,
    pub tokens: Vec<SymbolToken>,
    pub line_span: Span,
}

pub struct LoweredTune {
    pub header: LoweredHeader,
    pub part_list_entries: Vec<PartListEntry>,
    pub score_directive_provenance: Option<SourceProvenance>,
    pub parts: Vec<LoweredPart>,
}

#[derive(Clone, Debug)]
pub struct VoiceDefaults {
    pub meter: Option<(Meter, Option<SourceProvenance>)>,
    pub unit_length: (Fraction, Option<SourceProvenance>),
    pub explicit_unit_length: bool,
    pub key: (ResolvedKey, Option<SourceProvenance>),
    pub propagation: AccidentalPropagation,
    pub user_symbols: BTreeMap<char, Vec<MusicElement<String>>>,
    pub tempo: Option<(Tempo<String>, SourceProvenance)>,
    pub score_directive: Option<(ScoreDirectiveKind, String, SourceProvenance)>,
    pub midi: Option<MidiInstrumentSpec>,
}

impl Default for VoiceDefaults {
    fn default() -> Self {
        let default_key = KeySignature {
            tonic: Some(abc_parser::KeyTonic {
                class: PitchClass::C,
                accidental: None,
            }),
            mode: String::new(),
            parameters: Vec::new(),
        };
        Self {
            meter: None,
            unit_length: (Fraction::EIGHTH, None),
            explicit_unit_length: false,
            key: (resolve_key_signature(&default_key, None), None),
            propagation: AccidentalPropagation::Pitch,
            user_symbols: BTreeMap::new(),
            tempo: None,
            score_directive: None,
            midi: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct VoiceDeclInfo {
    pub id: String,
    pub name: Option<String>,
    pub subname: Option<String>,
    pub clef: Option<ClefSpec>,
    pub transpose: Option<TransposeSpec>,
    pub octave_shift: Option<i8>,
    pub midi: Option<MidiInstrumentSpec>,
    pub provenance: Option<SourceProvenance>,
}

#[derive(Clone, Debug)]
pub enum VoiceEvent {
    Meter(Meter, SourceProvenance),
    UnitLength(Fraction, SourceProvenance),
    Key(ResolvedKey, SourceProvenance),
    VoiceParams(VoiceDeclInfo),
    Propagation(AccidentalPropagation),
    Tempo(Tempo<String>, SourceProvenance),
    Lyrics(LyricsField),
    Symbols(SymbolsField),
    MusicElement {
        element: MusicElement<String>,
        line_span: Span,
        element_span: Span,
    },
}

#[derive(Clone, Debug)]
pub struct RawMeasureItem {
    pub line_span: Span,
    pub element_span: Span,
    pub kind: RawMeasureItemKind,
}

#[derive(Clone, Debug)]
pub enum RawMeasureItemKind {
    Meter(Meter, SourceProvenance),
    UnitLength(Fraction),
    Key(ResolvedKey, SourceProvenance),
    VoiceParams(VoiceDeclInfo),
    Propagation(AccidentalPropagation),
    Tempo(Tempo<String>, SourceProvenance),
    Note(Note),
    Rest(Rest),
    Chord(Chord),
    MultiMeasureRest(MultiMeasureRest),
    Grace(GraceGroup),
    Tuplet(Tuplet),
    BrokenRhythm(BrokenRhythm),
    Tie(Tie),
    Slur(Slur, SourceProvenance),
    Decoration(Decoration<String>, SourceProvenance),
    Annotation(Annotation<String>, SourceProvenance),
    BeamBreak,
    BeamContinuation,
    Overlay(Overlay),
    MergedVoice(usize),
    Lyrics(LyricsField),
    Symbols(SymbolsField),
}

#[derive(Clone, Debug, Default)]
pub struct SegmentedMeasure {
    pub first_line_span: Option<Span>,
    pub left_barline: Option<LoweredBarline>,
    pub right_barline: Option<LoweredBarline>,
    pub items: Vec<RawMeasureItem>,
    pub new_system: bool,
    pub new_page: bool,
}

impl SegmentedMeasure {
    #[must_use]
    pub fn has_musical_content(&self) -> bool {
        self.left_barline.is_some()
            || self.right_barline.is_some()
            || self.items.iter().any(|item| {
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

    pub const fn note_first_span(&mut self, line_span: Span) {
        if self.first_line_span.is_none() {
            self.first_line_span = Some(line_span);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NoteSlotCoord {
    pub layer: usize,
    pub measure_idx: usize,
    pub item_idx: usize,
}
