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

//! Spanner tracking for slurs, ties, and tuplets across measure and note boundaries.

use abc_parser::Tie;

use crate::lower::types::LoweredMeasure;
use crate::lower::types::LoweredNote;
use crate::lower::types::LoweredNoteKind;
use crate::lower::types::LoweredSlur;
use crate::lower::types::MeasureItem;
use crate::lower::types::SlurDescriptor;
use crate::lower::types::SpannerEndpoint;
use crate::lower::types::VoiceLayerKey;
use crate::source_map::SourceProvenance;
use crate::timing::Duration;
use crate::timing::ResolvedTuplet;

/// Active tuplet tracking state across consecutive notes within a tuplet bracket.
#[derive(Clone, Debug)]
pub struct ActiveTupletState {
    pub resolved: ResolvedTuplet,
    pub remaining: u8,
    pub started: bool,
    pub number: u8,
    pub provenance: SourceProvenance,
}

/// Timing and spanner modifications attached to an upcoming timed note or chord.
#[derive(Clone, Debug)]
pub struct TimedItemMods {
    pub broken_factor: Duration,
    pub tuplet_factor: Duration,
    pub tuplet_mod: Option<(u8, u8)>,
    pub tuplet_start: Option<(u8, SourceProvenance)>,
    pub tuplet_stop: Option<(u8, SourceProvenance)>,
    pub tie_after: Option<(Tie, SourceProvenance)>,
}

impl Default for TimedItemMods {
    fn default() -> Self {
        Self {
            broken_factor: Duration::ONE,
            tuplet_factor: Duration::ONE,
            tuplet_mod: None,
            tuplet_start: None,
            tuplet_stop: None,
            tie_after: None,
        }
    }
}

fn find_slur_target(
    items: &mut [MeasureItem],
    target_voice_key: VoiceLayerKey,
) -> Option<&mut LoweredNote> {
    items.iter_mut().rev().find_map(|item| match item {
        MeasureItem::Note(n)
            if !n.is_chord_member
                && n.voice_key == target_voice_key
                && !matches!(n.kind, LoweredNoteKind::Rest { .. }) =>
        {
            Some(n)
        }
        _ => None,
    })
}

/// Attaches a slur stop notation to the most recent pitched/unpitched note in
/// `target_voice_key`, searching backwards through `current_items` and then `previous_measures`.
pub fn attach_slur_stop(
    current_items: &mut [MeasureItem],
    previous_measures: &mut [LoweredMeasure],
    target_voice_key: VoiceLayerKey,
    descriptor: &SlurDescriptor,
) {
    let slur = LoweredSlur {
        slur_type: SpannerEndpoint::Stop,
        number: descriptor.number,
        dotted: descriptor.dotted,
        provenance: descriptor.provenance.clone(),
    };
    if let Some(n) = find_slur_target(current_items, target_voice_key) {
        n.notations_mut().slurs.push(slur);
        return;
    }
    for m in previous_measures.iter_mut().rev() {
        if let Some(n) = find_slur_target(&mut m.items, target_voice_key) {
            n.notations_mut().slurs.push(slur);
            return;
        }
    }
}
