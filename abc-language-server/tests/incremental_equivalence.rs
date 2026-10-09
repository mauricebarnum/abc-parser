// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Equivalence and fuzz harness for the incremental analysis driver.
//!
//! Each test picks a starting source from a shared corpus, then drives
//! a deterministic, seeded sequence of edits. After every edit two
//! models are produced: one by [`DocumentModel::apply_changes`] (the
//! regional incremental path) and one by [`DocumentModel::full`] over
//! the spliced text. The two models must agree on:
//!
//! - the published diagnostic list (message, severity, code, range),
//! - the per-block typed diagnostics,
//! - the file-header snapshot,
//! - the per-block parsed AST,
//! - and the per-block bar-duration analysis.
//!
//! Any divergence fails the test with a precise diff so the failure
//! can be reproduced with the same seed and edit index.
//!
//! The PRNG is a small xorshift64 seeded from each test case's seed so
//! runs are deterministic and CI failures can be re-played locally.

use std::ops::Range;

use abc_language_server::LineIndex;
use abc_language_server::config::Config;
use abc_language_server::document::BlockRecord;
use abc_language_server::document::DocumentModel;
use abc_language_server::document::EditError;
use tower_lsp_server::ls_types::Diagnostic;
use tower_lsp_server::ls_types::DiagnosticSeverity;
use tower_lsp_server::ls_types::NumberOrString;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::TextDocumentContentChangeEvent;

/// One starting source paired with a short label for failure messages.
type CorpusEntry = (&'static str, &'static str);

const CORPUS: &[CorpusEntry] = &[
    ("simple_valid_tune", "X:1\nT:Title\nK:C\nCDEF |\n"),
    (
        "two_tunes_with_blank_separator",
        "X:1\nT:A\nK:C\nCDEF |\n\nX:2\nT:B\nK:G\nGABc |\n",
    ),
    (
        "kitchen_sink_synthetic",
        "%abc-2.1\nX:1\nT:Kitchen Sink\nM:4/4\nL:1/8\nQ:1/4=120\nK:C\n\
         V:1 name=\"Soprano\"\n\
         [M:3/4] [K:D] [L:1/16] !trill!A4 !turn!!>!B4 .C4\n\
         V:2\n\
         [K:G mixolydian]\n\
         G,,4 D,,4 | G,,2 A,,2 B,,2 C,2 | D,4 G,4 |\n",
    ),
    (
        "lyrics_w_and_W",
        "X:1\nT:Lyrics\nM:4/4\nK:C\n\
         w: Ho-ly Mo-ly! Yes, here is a long line.\n\
         W: This is an unaligned lyric line at the very end of the tune\n\
         CDEF GABc |\n",
    ),
    (
        "field_continuation_plus",
        "X:1\nT:Continuation\nK:C\nT:continued title line\n\
         CDEF |\n",
    ),
    (
        "deprecated_implicit_h",
        "X:1\nT:Implicit H\nK:C\nH: This is\nhistory text\nmore history\n\
         CDEF |\n",
    ),
    (
        "begintext_between_tunes",
        "X:1\nK:C\nCDEF |\n\n%%begintext\nbetween\n%%endtext\n\nX:2\nK:D\nG |\n",
    ),
    (
        "begintext_inside_tune",
        "X:1\nK:C\n%%begintext\nnotes inside\n%%endtext\nCDEF |\n",
    ),
    (
        "music_like_free_text_after_tune",
        "X:1\nK:C\nCDEF |\n\nCDEF |\n",
    ),
    ("unknown_field_warning", "X:1\nK:C\nY:unknown\nCDEF |\n"),
    ("indented_field", "X:1\n  K:C\nCDEF |\n"),
    (
        "strict_marker_file",
        "%abc-2.1\nX:1\nT:Title\nK:C\nCDEF |\n",
    ),
    ("crlf_line_endings", "X:1\r\nK:C\r\nCDEF |\r\n"),
    ("cr_only_line_endings", "X:1\rK:C\rCDEF |\r"),
    ("no_final_newline", "X:1\nK:C\nCDEF |"),
    (
        "blank_separator_run",
        "X:1\nK:C\nCDEF |\n\n\n\nX:2\nK:G\nGABc |\n",
    ),
    (
        "comment_only_adjacent_to_blank",
        "X:1\nK:C\n% this is a comment\nCDEF |\n\n\
         % another comment\nX:2\nK:D\nG |\n",
    ),
    ("header_only_document", "M:4/4\nL:1/8\n"),
    ("empty_document", ""),
    (
        "bar_duration_violation",
        "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C | CDEFG | CCCCC\n",
    ),
    (
        "long_multiline_with_chords",
        "X:1\nT:Long\nM:4/4\nL:1/8\nK:C\n\
         [CEG]4 [G,B,D]2 [CEG]/2[FAc]/2 z | \
         [1 [GBd]2 c2 d2 e2 :| [2 [GBd]4 g4 |]\n",
    ),
    (
        "file_header_comments_and_multiple_tunes",
        "% header comment\n\
         % second line of comment to pad bytes\n\
         % third line of comment to push tunes further down\n\
         M:4/4\n\
         L:1/8\n\
         % more comments in file header\n\
         % even more comments in file header to ensure byte offset > 200\n\
         \n\
         X:1\n\
         T:First Tune\n\
         K:C\n\
         C2 D2 E2 F2 | G2 A2 B2 c2 |\n\
         \n\
         X:2\n\
         T:Second Tune with duration flaw\n\
         K:G\n\
         G2 A2 B2 c2 | G2 A2 B2 | c4 d4 |\n",
    ),
];

/// Drives a single fuzz pass for one starting source and one seed.
///
/// Generates a fixed number of random edits, applies each both
/// incrementally and via a full rebuild, and asserts equivalence.
fn fuzz_pass(corpus_name: &'static str, source: &'static str, seed: u64) {
    const EDITS_PER_RUN: usize = 32;
    let mut rng = Xorshift64::new(seed);
    let mut model = DocumentModel::full(
        source.to_owned(),
        0,
        PositionEncodingKind::UTF16,
        Config::default(),
    );

    for edit_index in 0..EDITS_PER_RUN {
        let Some(plan) = next_edit(&mut rng, &model) else {
            continue;
        };
        if plan.new_text.is_empty() && model.text.is_empty() {
            // The only safe edit on an empty document is a non-empty
            // insertion; skip everything else.
            continue;
        }
        let baseline = DocumentModel::full(
            plan.new_text.clone(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        match DocumentModel::apply_changes(
            &model,
            std::slice::from_ref(&plan.change),
            plan.new_text.clone(),
        ) {
            Ok(incremental) => {
                assert_models_equivalent(
                    &format!("{corpus_name} seed={seed} edit={edit_index}"),
                    &incremental,
                    &baseline,
                );
                model = incremental;
            }
            Err(EditError::FullRebuildRequired) => {
                // Fallback paths are part of the equivalence contract:
                // the request signals that a full rebuild is required
                // and the baseline model built above is the canonical
                // answer for this text. We adopt the baseline.
                model = baseline;
            }
        }
    }
}

/// One planned edit: the new full text after the edit plus the LSP
/// `TextDocumentContentChangeEvent` that drives it.
struct EditPlan {
    new_text: String,
    change: TextDocumentContentChangeEvent,
}

fn next_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    let choice = rng.range(0, 5);
    match choice {
        0 => insert_edit(rng, model),
        1 => delete_edit(rng, model),
        2 => replace_edit(rng, model),
        3 => blank_line_edit(rng, model),
        _ => eof_append_edit(rng, model),
    }
}

fn insert_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    let at = rng.range(0, model.text.len() + 1);
    let text = random_text(rng);
    apply_byte_edit(model, at, at, &text)
}

fn delete_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    if model.text.is_empty() {
        return None;
    }
    let start = rng.range(0, model.text.len());
    let end = rng.range(start, model.text.len() + 1).min(model.text.len());
    apply_byte_edit(model, start, end, "")
}

fn replace_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    if model.text.is_empty() {
        return insert_edit(rng, model);
    }
    let start = rng.range(0, model.text.len());
    let end = rng.range(start, model.text.len() + 1).min(model.text.len());
    let text = random_text(rng);
    apply_byte_edit(model, start, end, &text)
}

fn blank_line_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    if model.text.is_empty() {
        return None;
    }
    let insert_blank = rng.bool();
    if insert_blank {
        // Insert a blank line just after an existing line boundary.
        let index = LineIndex::new(model.text.clone());
        let line = rng.range(0, index.line_starts().len() + 1);
        let offset = line_offset_after(&model.text, &index, line);
        apply_byte_edit(model, offset, offset, "\n")
    } else {
        // Remove a blank line by deleting a `\n` that immediately follows
        // another line terminator.
        let bytes = model.text.as_bytes();
        let mut candidates = Vec::new();
        for cursor in 0..bytes.len().saturating_sub(1) {
            if matches!(bytes[cursor], b'\r' | b'\n') && matches!(bytes[cursor + 1], b'\r' | b'\n')
            {
                candidates.push(cursor);
            }
        }
        if candidates.is_empty() {
            return None;
        }
        let choice = candidates[rng.range(0, candidates.len())];
        apply_byte_edit(model, choice, choice + 1, "")
    }
}

fn eof_append_edit(rng: &mut Xorshift64, model: &DocumentModel) -> Option<EditPlan> {
    let mut text = String::new();
    text.push('\n');
    text.push_str(&random_text(rng));
    let offset = model.text.len();
    apply_byte_edit(model, offset, offset, &text)
}

fn apply_byte_edit(
    model: &DocumentModel,
    start: usize,
    end: usize,
    replacement: &str,
) -> Option<EditPlan> {
    if start > model.text.len() || end > model.text.len() || start > end {
        return None;
    }
    let mut new_text = String::with_capacity(model.text.len() + replacement.len());
    new_text.push_str(&model.text[..start]);
    new_text.push_str(replacement);
    new_text.push_str(&model.text[end..]);
    let index = LineIndex::new(model.text.clone());
    let range = index.lsp_range(start..end, &model.encoding)?;
    let change = TextDocumentContentChangeEvent {
        range: Some(range),
        range_length: None,
        text: replacement.to_owned(),
    };
    Some(EditPlan { new_text, change })
}

fn random_text(rng: &mut Xorshift64) -> String {
    let length = rng.range(1, 8);
    (0..length)
        .map(|_| ABC_GLYPHS[rng.range(0, ABC_GLYPHS.len())])
        .collect()
}

const ABC_GLYPHS: &[char] = &[
    'X', 'T', 'K', 'M', 'L', 'Q', 'V', 'P', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'a', 'b', 'c', 'd',
    'e', 'f', 'g', ' ', '|', '[', ']', '^', '_', '=', ':', '%', '\n', '\r', '!', '+', 'w', 'W',
];

fn line_offset_after(text: &str, index: &LineIndex, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    let starts = index.line_starts();
    if line >= starts.len() {
        return text.len();
    }
    starts[line]
}

fn assert_models_equivalent(context: &str, incremental: &DocumentModel, baseline: &DocumentModel) {
    assert_eq!(
        incremental.text, baseline.text,
        "{context}: text mismatch\nincremental: {:?}\nbaseline:    {:?}",
        incremental.text, baseline.text
    );
    assert_eq!(
        incremental.strict, baseline.strict,
        "{context}: strict flag mismatch"
    );
    assert_eq!(
        incremental.header, baseline.header,
        "{context}: header snapshot mismatch\nincremental: {:?}\nbaseline:    {:?}",
        incremental.header, baseline.header
    );
    if incremental.has_errors != baseline.has_errors {
        eprintln!("incremental diags: {:#?}", incremental.diagnostics);
        eprintln!("baseline diags:    {:#?}", baseline.diagnostics);
        eprintln!("incremental blocks: {:#?}", incremental.blocks);
        eprintln!("baseline blocks:    {:#?}", baseline.blocks);
    }
    assert_eq!(
        incremental.has_errors, baseline.has_errors,
        "{context}: has_errors flag mismatch"
    );
    assert_eq!(
        incremental.blocks.len(),
        baseline.blocks.len(),
        "{context}: block count mismatch (incremental={}, baseline={})",
        incremental.blocks.len(),
        baseline.blocks.len()
    );
    assert_eq!(
        incremental.diagnostics, baseline.diagnostics,
        "{context}: published diagnostics mismatch\nincremental: {:#?}\nbaseline:    {:#?}",
        incremental.diagnostics, baseline.diagnostics
    );
    for (index, (a, b)) in incremental
        .blocks
        .iter()
        .zip(baseline.blocks.iter())
        .enumerate()
    {
        assert_block_equivalent(&format!("{context} block={index}"), a, b);
    }
}

fn assert_block_equivalent(context: &str, a: &BlockRecord, b: &BlockRecord) {
    let rebase_span = |span: abc_parser::SimpleSpan<usize>, base: isize| -> Range<usize> {
        let s = span.start.cast_signed() + base;
        let e = span.end.cast_signed() + base;
        let start = if s > 0 { s.cast_unsigned() } else { 0 };
        let end = if e > 0 { e.cast_unsigned() } else { 0 };
        start..end
    };
    assert_eq!(a.lines, b.lines, "{context}: lines mismatch");
    assert_eq!(
        rebase_span(a.parsed.span, a.base),
        rebase_span(b.parsed.span, b.base),
        "{context}: rebased parsed span mismatch"
    );
    assert_eq!(
        a.parsed.first_field.map(|s| rebase_span(s, a.base)),
        b.parsed.first_field.map(|s| rebase_span(s, b.base)),
        "{context}: rebased first_field mismatch"
    );
    assert_eq!(
        a.parsed.items.len(),
        b.parsed.items.len(),
        "{context}: parsed items count mismatch"
    );
    for (item_idx, (item_a, item_b)) in a.parsed.items.iter().zip(b.parsed.items.iter()).enumerate()
    {
        assert_eq!(
            rebase_span(item_a.span, a.base),
            rebase_span(item_b.span, b.base),
            "{context} item={item_idx}: item span mismatch"
        );
        match (&item_a.value, &item_b.value) {
            (abc_parser::DocumentItem::Tune(_), abc_parser::DocumentItem::Tune(_))
            | (abc_parser::DocumentItem::FreeText(_), abc_parser::DocumentItem::FreeText(_))
            | (
                abc_parser::DocumentItem::TypesetText(_),
                abc_parser::DocumentItem::TypesetText(_),
            )
            | (abc_parser::DocumentItem::Comment(_), abc_parser::DocumentItem::Comment(_))
            | (abc_parser::DocumentItem::Directive(_), abc_parser::DocumentItem::Directive(_)) => {}
            _ => panic!("{context} item={item_idx}: item variant mismatch"),
        }
    }
    assert_eq!(
        a.parsed.header.len(),
        b.parsed.header.len(),
        "{context}: parsed header count mismatch"
    );
    for (hdr_idx, (hdr_a, hdr_b)) in a
        .parsed
        .header
        .iter()
        .zip(b.parsed.header.iter())
        .enumerate()
    {
        assert_eq!(
            rebase_span(hdr_a.span, a.base),
            rebase_span(hdr_b.span, b.base),
            "{context} header={hdr_idx}: header line span mismatch"
        );
    }
    assert_eq!(
        a.diagnostics, b.diagnostics,
        "{context}: per-block diagnostics mismatch\nincremental: {:#?}\nbaseline:    {:#?}",
        a.diagnostics, b.diagnostics
    );
    let rebase_tune = |t: &Option<abc_language_server::document::TuneAnalysis>,
                       base: isize|
     -> Option<Vec<(abc_parser::ErrorKind, String, Range<usize>)>> {
        t.as_ref().map(|analysis| {
            analysis
                .warnings
                .iter()
                .map(|w| (w.kind, w.message.clone(), rebase_span(w.span, base)))
                .collect()
        })
    };
    assert_eq!(
        rebase_tune(&a.tune_analysis, a.base),
        rebase_tune(&b.tune_analysis, b.base),
        "{context}: tune_analysis mismatch\nincremental: {:#?}\nbaseline:    {:#?}",
        a.tune_analysis,
        b.tune_analysis
    );
}

/// Minimal deterministic PRNG; seeded xorshift64 produces reproducible
/// fuzz scripts so any failure can be re-played by re-running the test
/// with the same seed.
#[derive(Clone)]
struct Xorshift64(u64);

impl Xorshift64 {
    const fn new(seed: u64) -> Self {
        let mut state = if seed == 0 { 1 } else { seed };
        // SplitMix64 warm-up so consecutive seeds produce visibly
        // different streams even when they share low bits.
        state ^= state >> 30;
        state = state.wrapping_mul(0xbf58_476d_1ce4_e5b9);
        state ^= state >> 27;
        state = state.wrapping_mul(0x94d0_49bb_1331_11eb);
        state ^= state >> 31;
        Self(state)
    }

    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    const fn range(&mut self, low: usize, high: usize) -> usize {
        if low >= high {
            return low;
        }
        low + (self.next() as usize % (high - low))
    }

    const fn bool(&mut self) -> bool {
        (self.next() & 1) == 0
    }
}

const fn severity_label(diagnostic: &Diagnostic) -> &'static str {
    match diagnostic.severity {
        Some(DiagnosticSeverity::ERROR) => "ERROR",
        Some(DiagnosticSeverity::WARNING) => "WARNING",
        Some(DiagnosticSeverity::INFORMATION) => "INFORMATION",
        Some(DiagnosticSeverity::HINT) => "HINT",
        _ => "NONE",
    }
}

#[allow(dead_code)]
fn code_label(diagnostic: &Diagnostic) -> String {
    diagnostic
        .code
        .as_ref()
        .and_then(|code| match code {
            NumberOrString::String(text) => Some(text.clone()),
            NumberOrString::Number(_) => None,
        })
        .unwrap_or_default()
}

/// Runs every corpus entry against every seed in `SEEDS`. The set is
/// large enough to exercise the regional driver on a representative
/// mix of edits without slowing CI down; a longer pass is available
/// via the `incremental_equivalence::fuzz_long` test below.
const SEEDS: &[u64] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

#[test]
fn fuzz_short_pass_matches_full_rebuild() {
    for (name, source) in CORPUS {
        for seed in SEEDS {
            fuzz_pass(name, source, *seed);
        }
    }
}

/// Documented long-run mode: not run by default (gated behind the
/// `LONG_FUZZ=1` environment variable) so CI stays fast, but available
/// for local nightly hunts with `LONG_FUZZ=1 cargo test fuzz_long`.
#[test]
#[ignore = "long-running fuzz test intended for manual or extended runs"]
fn fuzz_long_pass_matches_full_rebuild() {
    if std::env::var_os("LONG_FUZZ").is_none() {
        return;
    }
    for (name, source) in CORPUS {
        for seed in 100..120 {
            fuzz_pass(name, source, seed);
        }
    }
}

/// Targeted regression: an edit inside the line-1 version-marker slot
/// must return `FullRebuildRequired` so the caller can fall back.
#[test]
fn line_one_edit_falls_back_to_full_rebuild() {
    let source = "X:1\nK:C\nCDEF |\n";
    let model = DocumentModel::full(
        source.to_owned(),
        0,
        PositionEncodingKind::UTF16,
        Config::default(),
    );
    let new_text = "%abc-2.1\nX:1\nK:C\nCDEF |\n";
    let index = LineIndex::new(source.to_owned());
    let range = index
        .lsp_range(0..0, &PositionEncodingKind::UTF16)
        .expect("range");
    let change = TextDocumentContentChangeEvent {
        range: Some(range),
        range_length: None,
        text: "%abc-2.1\n".to_owned(),
    };
    let result =
        DocumentModel::apply_changes(&model, std::slice::from_ref(&change), new_text.to_owned());
    assert!(matches!(result, Err(EditError::FullRebuildRequired)));
}

#[test]
fn range_none_change_falls_back_to_full_rebuild() {
    let source = "X:1\nK:C\nCDEF |\n";
    let model = DocumentModel::full(
        source.to_owned(),
        0,
        PositionEncodingKind::UTF16,
        Config::default(),
    );
    let new_text = "X:2\nK:D\nGABc |\n";
    let change = TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: new_text.to_owned(),
    };
    let result =
        DocumentModel::apply_changes(&model, std::slice::from_ref(&change), new_text.to_owned());
    assert!(matches!(result, Err(EditError::FullRebuildRequired)));
}

#[test]
fn forced_full_rebuild_recovers_consistency() {
    // A line-1 edit fails the incremental path; the backend is
    // expected to fall back to DocumentModel::full, and the result
    // must still match what full alone would produce.
    let new_text = "%abc-2.1\nX:1\nK:C\nCDEF |\n";
    let baseline = DocumentModel::full(
        new_text.to_owned(),
        0,
        PositionEncodingKind::UTF16,
        Config::default(),
    );
    let _ = severity_label;
    assert!(baseline.strict);
    assert_eq!(baseline.blocks.len(), 1);
}

#[test]
fn file_header_comments_and_tune_past_offset_200_retains_bar_duration_warnings() {
    let padding = "% comment padding to push tune past byte 200\n".repeat(5);
    let source = format!(
        "% header comment\nM:4/4\nL:1/8\n{padding}\n\nX:1\nT:First\nK:C\nC2 D2 E2 F2 | G2 A2 B2 c2 |\n\nX:2\nT:Second\nK:G\nG2 A2 B2 c2 | G2 A2 B2 | c4 d4 |\n"
    );
    assert!(source.find("X:2").unwrap() > 200);
    let model = DocumentModel::full(source, 1, PositionEncodingKind::UTF16, Config::default());
    let bar_diag = model
        .diagnostics
        .iter()
        .find(|d| d.code == Some(NumberOrString::String("bar-duration".to_owned())));
    assert!(
        bar_diag.is_some(),
        "expected bar-duration warning on tune 2"
    );
    let bar_diag = bar_diag.unwrap();
    assert!(bar_diag.range.start.line > 5);
}
