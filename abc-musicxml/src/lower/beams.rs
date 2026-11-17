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

//! Beam calculations for notes and grace groups in `MusicXML` 4.0.

use crate::lower::types::LoweredBeam;
use crate::lower::types::LoweredNote;
use crate::lower::types::MeasureItem;

/// Returns the number of beam levels required for a note based on its graphic type.
#[must_use]
pub fn note_beam_level_count(note: &LoweredNote) -> u8 {
    match note.note_type {
        Some("eighth") => 1,
        Some("16th") => 2,
        Some("32nd") => 3,
        Some("64th") => 4,
        Some("128th") => 5,
        Some("256th") => 6,
        Some("512th") => 7,
        Some("1024th") => 8,
        _ => 0,
    }
}

fn for_each_beam(level_counts: &[u8], mut apply: impl FnMut(usize, LoweredBeam)) {
    let k = level_counts.len();
    if k < 2 {
        return;
    }
    for (i, &count) in level_counts.iter().enumerate() {
        for l in 1..=count {
            let has_prev = i > 0 && level_counts[i - 1] >= l;
            let has_next = i + 1 < k && level_counts[i + 1] >= l;
            let value = match (has_prev, has_next) {
                (false, true) => "begin",
                (true, true) => "continue",
                (true, false) => "end",
                (false, false) => {
                    if i > 0 {
                        "backward hook"
                    } else {
                        "forward hook"
                    }
                }
            };
            apply(i, LoweredBeam { number: l, value });
        }
    }
}

/// Computes and assigns beam values (`begin`, `continue`, `end`, `forward hook`, `backward hook`)
/// across a contiguous slice of notes (e.g. within a beamed grace group).
pub fn apply_beams_to_notes(notes: &mut [&mut LoweredNote]) {
    if notes.len() < 2 {
        return;
    }
    let level_counts: Vec<u8> = notes.iter().map(|n| note_beam_level_count(n)).collect();
    for_each_beam(&level_counts, |i, beam| notes[i].beams.push(beam));
}

/// Finalizes a beam group by computing beam values for the notes at `indices` in `items`.
pub fn finish_beam_group(indices: &[usize], items: &mut [MeasureItem]) {
    if indices.len() < 2 {
        return;
    }
    let level_counts: Vec<u8> = indices
        .iter()
        .map(|&idx| match &items[idx] {
            MeasureItem::Note(n) => note_beam_level_count(n),
            _ => 0,
        })
        .collect();

    for_each_beam(&level_counts, |i, beam| {
        if let MeasureItem::Note(ref mut n) = items[indices[i]] {
            n.beams.push(beam);
        }
    });
}
