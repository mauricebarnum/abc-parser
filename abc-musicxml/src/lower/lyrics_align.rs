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

//! Lyric (`w:`) and symbol line (`s:`) note-slot alignment and lowering.

use abc_parser::AnnotationPlacement;
use abc_parser::ParseWarning;

use crate::lower::note::apply_decoration_or_direction;
use crate::lower::note::lower_annotation;
use crate::lower::types::LoweredMeasure;
use crate::lower::types::LoweredNotations;
use crate::lower::types::MeasureItem;
use crate::lower::types::Span;
use crate::source_map::SourceProvenance;

/// Lowers an aligned symbol token string (such as `!f!`, `"Am"`, `^"pizz"`, or `.`)
/// targeting the note at `lowered_measures[measure_idx].items[item_idx]`.
pub fn lower_symbol_element(
    text: &str,
    prov: SourceProvenance,
    measure_idx: usize,
    item_idx: usize,
    lowered_measures: &mut [LoweredMeasure],
    insertions: &mut Vec<(usize, usize, MeasureItem)>,
    warnings: &mut Vec<ParseWarning<Span>>,
) {
    if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
        let inner = &text[1..text.len() - 1];
        let (placement, content) = match inner.chars().next() {
            Some('^') => (AnnotationPlacement::Above, &inner[1..]),
            Some('_') => (AnnotationPlacement::Below, &inner[1..]),
            Some('<') => (AnnotationPlacement::Left, &inner[1..]),
            Some('>') => (AnnotationPlacement::Right, &inner[1..]),
            Some('@') => (AnnotationPlacement::Free, &inner[1..]),
            _ => (AnnotationPlacement::ChordSymbol, inner),
        };
        let item = lower_annotation(placement, content, prov, warnings);
        insertions.push((measure_idx, item_idx, item));
    } else {
        let name = text
            .strip_prefix('!')
            .and_then(|s| s.strip_suffix('!'))
            .unwrap_or(text);
        apply_symbol_decoration(
            name,
            Some(prov),
            measure_idx,
            item_idx,
            lowered_measures,
            insertions,
        );
    }
}

fn apply_symbol_decoration(
    name: &str,
    prov: Option<SourceProvenance>,
    measure_idx: usize,
    item_idx: usize,
    lowered_measures: &mut [LoweredMeasure],
    insertions: &mut Vec<(usize, usize, MeasureItem)>,
) {
    let mut temp_notations = LoweredNotations::default();
    if let Some(dir) = apply_decoration_or_direction(name, prov, &mut temp_notations) {
        insertions.push((measure_idx, item_idx, MeasureItem::Direction(dir)));
    } else if !temp_notations.is_empty()
        && let MeasureItem::Note(ref mut note) = lowered_measures[measure_idx].items[item_idx]
    {
        note.notations_mut().merge(temp_notations);
    }
}
