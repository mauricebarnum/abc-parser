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

//! Parses the kitchen-sink fixture and demonstrates an AST transformation.
//!
//! The transformation moves written pitches up one diatonic step. It preserves
//! accidentals, so it demonstrates AST traversal rather than key-aware
//! chromatic transposition.

use abc_parser::ChordMember;
use abc_parser::Document;
use abc_parser::Field;
use abc_parser::FieldValue;
use abc_parser::Line;
use abc_parser::MusicElement;
use abc_parser::Note;
use abc_parser::Pitch;
use abc_parser::PitchClass;
use abc_parser::Tune;
use abc_parser::parse_recovering;
use std::process::ExitCode;

const KITCHEN_SINK: &str = include_str!("../test_kitchen_sink.abc");

/// Parses, prints, transposes, and reprints the bundled ABC fixture.
fn main() -> ExitCode {
    let report = parse_recovering(KITCHEN_SINK);
    for error in &report.errors {
        eprintln!("{error}");
    }
    if !report.is_valid() {
        return ExitCode::FAILURE;
    }

    println!("=== ORIGINAL AST ===");
    println!("{:#?}", report.output);

    let mut transposed = report.output;
    transpose_document(&mut transposed);

    println!("=== TRANSPOSED UP ONE DIATONIC STEP ===");
    println!("{transposed:#?}");
    ExitCode::SUCCESS
}

/// Transposes field keys and every semantic note in a document in place.
fn transpose_document<S>(document: &mut Document<S, String>) {
    for line in &mut document.header {
        transpose_line(&mut line.value);
    }
    for tune in &mut document.tunes {
        transpose_tune(tune);
    }
}

/// Transposes all semantic pitches belonging to one tune.
fn transpose_tune<S>(tune: &mut Tune<S, String>) {
    for line in &mut tune.lines {
        transpose_line(&mut line.value);
    }
}

/// Dispatches transposition according to a physical line's syntax category.
fn transpose_line<S>(line: &mut Line<S, String>) {
    match line {
        Line::Field(field) => transpose_field(field),
        Line::Music(elements) => {
            for element in elements {
                transpose_music_element(&mut element.value);
            }
        }
        _ => {}
    }
}

/// Transposes key tonics while leaving non-key field values unchanged.
fn transpose_field(field: &mut Field<String>) {
    if let FieldValue::Key(key) = &mut field.value
        && let Some(tonic) = &mut key.tonic
    {
        tonic.class = next_pitch_class(tonic.class);
    }
}

/// Transposes every pitch-bearing form represented by a music element.
fn transpose_music_element(element: &mut MusicElement<String>) {
    match element {
        MusicElement::Note(note) => transpose_note(note),
        MusicElement::Chord(chord) => {
            for member in &mut chord.members {
                if let ChordMember::Note(note) = member {
                    transpose_note(note);
                }
            }
        }
        MusicElement::Grace(group) => {
            for note in &mut group.notes {
                transpose_note(note);
            }
        }
        MusicElement::InlineField(field) => transpose_field(field),
        _ => {}
    }
}

/// Moves a note up one diatonic class and crosses an octave after B.
fn transpose_note(note: &mut Note) {
    note.pitch = transpose_pitch(note.pitch);
}

/// Returns a pitch one diatonic step above the input pitch.
const fn transpose_pitch(pitch: Pitch) -> Pitch {
    Pitch {
        class: next_pitch_class(pitch.class),
        octave: if matches!(pitch.class, PitchClass::B) {
            pitch.octave.saturating_add(1)
        } else {
            pitch.octave
        },
        accidental: pitch.accidental,
    }
}

/// Returns the next diatonic pitch class, wrapping B to C.
const fn next_pitch_class(class: PitchClass) -> PitchClass {
    match class {
        PitchClass::C => PitchClass::D,
        PitchClass::D => PitchClass::E,
        PitchClass::E => PitchClass::F,
        PitchClass::F => PitchClass::G,
        PitchClass::G => PitchClass::A,
        PitchClass::A => PitchClass::B,
        PitchClass::B => PitchClass::C,
    }
}
