# Incremental Language Server Review & Refactoring Plan

**Target Branch:** `incremental-language-server`  
**Base Branch:** `main`  
**Commits Reviewed:** 15 commits (`bc6664e` through `b35c619`)  
**Diffstat:** 17 files changed, 4843 insertions(+), 328 deletions(-)

---

## 1. Executive Summary

This branch implements the incremental parsing and language server synchronization architecture for ABC 2.1 files, spanning Tracks 0, A, and B of the design document [`incremental-languge-server.md`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/incremental-languge-server.md).

Key achievements on this branch:
- **Foundations (Track 0)**: Multi-line terminator support (CRLF, LF, CR-only) in both the parser (`DiagnosticRenderer`) and language server (`LineIndex`), block-scoped parsing API [`parse_blocks`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs#L2948-L2975), and strict version marker parsing.
- **Diagnostics (Track A)**: Strict-mode warnings for unrecognized information field identifiers and misplaced tune-body constructs.
- **Incremental Engine (Track B)**: Full and regional document models ([`DocumentModel`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L157-L220)), incremental LSP synchronization (`TextDocumentSyncKind::INCREMENTAL`), forward cascade handling (D4), configuration updates without re-parsing (T12), and an equivalence test harness with fuzz testing (T11).

While the overall architecture is sound and all 160 workspace tests currently pass, this deep-dive review identified **three high-severity bugs** in LSP change order, UTF-8 BOM offset accounting, and synthetic AST source resolution, **one medium-severity edge case** causing unnecessary full rebuilds on leading separator edits, several **commit hygiene opportunities**, and **performance refactoring targets**.

---

## 2. Prioritized Findings

Findings are categorized and prioritized by severity (**High**, **Medium**, **Low**).

```
========================================================================================
SEVERITY   CATEGORY            FINDING
========================================================================================
HIGH       Bug / Correctness   1. Inverted Sequential LSP Content Changes & Stale Index
HIGH       Bug / Correctness   2. UTF-8 BOM Span Offset Shift in `parse_blocks`
HIGH       Bug / Correctness   3. Corrupted File-Header Spans via `OffsetResolver`
MEDIUM     Bug / Correctness   4. Premature Full-Rebuild on Leading Separator Edits
MEDIUM     Refactoring         5. Redundant Tune Re-Analysis on Severity Level Changes
MEDIUM     Git Hygiene         6. Squashing Standalone Plan Updates into Task Commits
LOW        Git Hygiene         7. Consolidating Fixup Leakage across T8, T10, and T11
LOW        Refactoring         8. Deduplicate `LineIndex` in Integration Tests
LOW        Refactoring         9. Out-of-Order Publish Guard in `Backend::publish`
========================================================================================
```

---

### Finding 1: Inverted Sequential LSP Content Changes & Stale `previous.index` Range Translation
- **Severity**: **HIGH**
- **Category**: Bug / Correctness
- **Files & Lines**:
  - [`abc-language-server/src/backend.rs:754-772`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L754-L772) (`apply_content_changes`)
  - [`abc-language-server/src/document.rs:408-420`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L408-L420) (`DocumentModel::apply_changes`)
  - [`abc-language-server/src/backend.rs:1235-1307`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L1235-L1307) (`tests::did_change_applies_multiple_ranged_edits_in_order`)
- **Description**:
  1. According to the LSP 3.17 Specification (`textDocument/didChange`), the `contentChanges` array contains sequential state changes: edit 0 applies to document state $D_0$ to yield $D_1$; edit 1 applies to $D_1$ to yield $D_2$, and so forth.
  2. In [`backend.rs:763`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L763), `apply_content_changes` iterates over `changes.iter().rev()`, assuming later array entries must be applied first.
  3. In [`document.rs:410-417`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L410-L417), `DocumentModel::apply_changes` converts *all* change ranges into byte ranges against `previous.index` ($D_0$). When multiple edits are delivered in a single notification (e.g. multi-cursor edits, snippet expansions, or format-on-paste), edit 1's position is evaluated against $D_0$ instead of $D_1$. If edit 0 altered line counts or character offsets, edit 1's translated byte range will be misaligned, leading to dirty region miscalculation, corrupted text slicing, or panics.
  4. The existing unit test `did_change_applies_multiple_ranged_edits_in_order` codified this reversed behavior by expecting reverse application.
- **Benefit**:
  - Full compliance with LSP standard. Eliminates text corruption, misaligned diagnostics, and crash risks on multi-cursor edits.
- **Risk**:
  - Low risk. The existing test `did_change_applies_multiple_ranged_edits_in_order` must be updated to expect sequential application order.

---

### Finding 2: UTF-8 BOM Span Offset Shift in `parse_blocks`
- **Severity**: **HIGH**
- **Category**: Bug / Correctness
- **Files & Lines**:
  - [`abc-parser/src/combinators.rs:2948-2975`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs#L2948-L2975) (`parse_blocks`)
  - [`abc-parser/src/combinators.rs:3000-3011`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs#L3000-L3011) (`strip_prologue`)
- **Description**:
  - When an ABC document begins with a UTF-8 Byte Order Mark (`\u{feff}`, 3 bytes: `0xEF 0xBB 0xBF`), `strip_prologue` strips the BOM via `input.strip_prefix('\u{feff}')` and passes the 3-byte-shortened slice to Chumsky.
  - All block line spans, body spans, field spans, syntax errors, and warnings returned by Chumsky are indexed relative to `slice` (starting at offset 0).
  - The doc comment of [`parse_blocks`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs#L2940-L2947) states: *"Diagnostics and spans are relative to `input` (the caller re-bases them to the document-absolute coordinate space)"*. However, `parse_blocks` never rebases output spans when a BOM is stripped.
  - In contrast, [`document_parser`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs#L2867) consumes the BOM within Chumsky (`just('\u{feff}').or_not()`), ensuring spans are 0-indexed relative to `input`.
  - In `DocumentModel::full` and `DocumentModel::apply_changes`, this causes every diagnostic, symbol, and fold in BOM-prefixed files to be shifted 3 bytes to the left.
- **Benefit**:
  - Accurately positions diagnostics and symbols for UTF-8 files with BOM, restoring consistency between `parse_blocks` and `document_parser`.
- **Risk**:
  - Very low risk. Either consume the BOM inside Chumsky in `blocks_combinator` or rebase spans by 3 bytes when BOM is present.

---

### Finding 3: Corrupted File-Header Spans via `OffsetResolver` in `analyze_tune_block`
- **Severity**: **HIGH**
- **Category**: Bug / Correctness
- **Files & Lines**:
  - [`abc-language-server/src/document.rs:640-680`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L640-L680) (`OffsetResolver`)
  - [`abc-language-server/src/document.rs:704-728`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L704-L728) (`analyze_tune_block`)
- **Description**:
  - In `analyze_tune_block`, a synthetic [`ParsedDocument`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L715-L721) is constructed by combining `file_header` (whose spans are document-absolute, 0-indexed) and `tune` (whose spans are slice-relative, 0-indexed within the tune).
  - To convert this synthetic document to an owned AST, `synthetic.into_owned(&resolver)` is called with `resolver = OffsetResolver::new(base, source)`, where `base` is `record.base` (e.g. byte 5000 for Tune 10).
  - `OffsetResolver` adds `base` to *every* span it resolves. Consequently, it adds 5000 to the spans in `synthetic.header`!
  - If the file header contains comments (`% comment`), text fields (`T:`, `C:`, `O:`), or directives, `resolver` attempts to resolve `source[5000 + start..5000 + end]`.
  - If `source.len() < 5000 + end`, `source.resolve()` returns `Err(ResolveError::OutOfBounds)`. `synthetic.into_owned()` fails, causing `analyze_tune_block` to silently return `None`—completely suppressing bar-duration analysis for that tune.
  - If `source` is long enough, the header fields are populated with corrupt strings extracted from arbitrary text inside the tune body.
- **Benefit**:
  - Prevents silent failure of bar-duration validation on multi-tune files with rich headers.
  - Prevents AST string corruption during header inheritance.
- **Risk**:
  - Very low risk. The file header should either be converted to owned prior to synthesis or resolved with `base = 0`.

---

### Finding 4: Premature Full-Rebuild Fallback on Leading Separator Edits
- **Severity**: **MEDIUM**
- **Category**: Bug / Correctness
- **Files & Lines**:
  - [`abc-language-server/src/document.rs:426-444`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L426-L444) (`DocumentModel::apply_changes`)
  - [`abc-language-server/src/document.rs:923-947`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L923-L947) (`touched_block_indices`)
- **Description**:
  - [`incremental-languge-server.md`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/incremental-languge-server.md) §1 specifies: *"An edit touching leading separators or byte 0 touches block 0."*
  - In `touched_block_indices`, edits are tested against `block.lines`, inter-block separators, and EOF after the last block. But an edit occurring inside leading separators before block 0 (e.g. typing a blank line or comment before block 0) matches none of these conditions.
  - `touched_block_indices` returns an empty vector.
  - In `apply_changes`:
    ```rust
    let touched = touched_block_indices(&previous.blocks, &edits);
    if touched.is_empty() {
        return Err(EditError::FullRebuildRequired);
    }
    ```
  - It bails with `EditError::FullRebuildRequired`. The expansion check at line 436 (`edits.iter().any(|e| e.start <= previous.blocks[0].lines.start)`) is dead code when the leading edit is the only edit.
- **Benefit**:
  - Preserves incremental parsing performance when users edit file headers, leading comments, or initial blank lines.
- **Risk**:
  - Minimal. Including index 0 when `edit.start <= blocks[0].lines.start` directly matches the design specification.

---

### Finding 5: Redundant Tune Re-Analysis on Severity Downgrade/Upgrade in `reconfigure`
- **Severity**: **MEDIUM**
- **Category**: Performance / Refactoring
- **Files & Lines**:
  - [`abc-language-server/src/document.rs:313-351`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs#L313-L351) (`DocumentModel::reconfigure`)
  - [`abc-language-server/src/backend.rs:211-231`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L211-L231) (`replace_config`)
- **Description**:
  - In `DocumentModel::reconfigure`, if `self.config.validation.bar_duration != config.validation.bar_duration`, `analyze_tune_block` is re-run on *every* tune block in the document.
  - However, `analyze_tune_block` returns pure bar duration discrepancies; it does not depend on the configured severity (`Warning`, `Information`, `Hint`, `Error`). The severity level is only mapped during `assemble_diagnostics`.
  - If a user changes settings between `Warning`, `Information`, `Hint`, or `Error` (all non-`Off`), re-running analysis is completely redundant—`record.tune_analysis` already contains the required data.
  - In `backend.rs:replace_config`, `reconfigure` is executed while holding `state.write().await`, blocking all language server operations during this redundant analysis.
- **Benefit**:
  - Near instantaneous configuration updates across large tunebooks.
  - Reduces lock contention on backend state.
- **Risk**:
  - Very low risk. Only re-run analysis when transitioning to/from `DiagnosticLevel::Off`.

---

### Finding 6: Squashing Standalone Plan Updates into Task Commits
- **Severity**: **MEDIUM**
- **Category**: Git Hygiene / Commit Consolidation
- **Files & Lines**: Commits `b35c619`, `20a337f`, `de70004`, `f409559`.
- **Description**:
  - Four separate commits on the branch do nothing except toggle checkboxes in `incremental-languge-server.md`:
    - `b35c619`: "Update plan: mark T12 complete (commit 6731013)"
    - `20a337f`: "Update plan: mark T11 complete (commit 0017b71)"
    - `de70004`: "Mark Track A tasks complete"
    - `f409559`: "Mark Track 0 tasks complete"
  - Having standalone commits for markdown checkbox updates pollutes `git log` and separates the task's documentation from the code that implemented it.
- **Benefit**:
  - Cleaner, atomic git history where each commit represents a complete task with its accompanying plan update.
- **Risk**:
  - Requires git rebase/squash before merging to `main`.

---

### Finding 7: Consolidating Fixup Leakage across T8, T10, and T11
- **Severity**: **LOW**
- **Category**: Git Hygiene / Commit Consolidation
- **Files & Lines**: Commit `0017b71` (`abc-language-server/src/lib.rs`, `Cargo.toml`, `combinators.rs`, `document.rs`).
- **Description**:
  - Commit `0017b71` ("Equivalence and fuzz harness for incremental engine (T11)") modified 12 files (+1023/-446 lines). In addition to adding integration tests, it created the library target (`lib.rs`, `Cargo.toml`) and resolved numerous driver bugs and clippy warnings in `document.rs` and `combinators.rs`.
  - Logically, exporting the library crate target belongs with T8 (introduction of `DocumentModel`), and parser/driver bug fixes belong with T8/T10.
- **Benefit**:
  - Isolates test harness additions from core engine modifications.
- **Risk**:
  - Optional; mainly relevant if interactive rebase is preferred before squashing to `main`.

---

### Finding 8: Deduplicate `LineIndex` in Integration Tests
- **Severity**: **LOW**
- **Category**: Code Quality / Refactoring
- **Files & Lines**:
  - [`abc-language-server/tests/incremental_equivalence.rs:490-540`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/tests/incremental_equivalence.rs#L490-L540)
  - [`abc-language-server/src/position.rs:18-120`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/position.rs#L18-L120)
- **Description**:
  - `tests/incremental_equivalence.rs` implements a private, duplicate version of `LineIndex` for mapping character positions in the test harness.
  - Since `abc-language-server` now exposes a public library target, `LineIndex` can be exported (under `pub` or `#[doc(hidden)] pub`) and reused directly in integration tests.
- **Benefit**:
  - Removes 50 lines of duplicate line index logic, ensuring tests always exercise the canonical implementation.
- **Risk**:
  - Negligible.

---

### Finding 9: Out-of-Order Publish Guard in `Backend::publish`
- **Severity**: **LOW**
- **Category**: Robustness / Concurrency
- **Files & Lines**:
  - [`abc-language-server/src/backend.rs:181-189`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L181-L189) (`publish`)
  - [`abc-language-server/src/backend.rs:501-504`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs#L501-L504) (`apply_update`)
- **Description**:
  - In `apply_update`, state lock is dropped before `self.publish(uri, &document).await`.
  - If a subsequent update finishes slightly faster or is scheduled before the earlier publish completes, the LSP client could receive newer diagnostics followed by older diagnostics, leaving stale errors on screen.
- **Benefit**:
  - Prevents out-of-order diagnostic flashes during rapid typing.
- **Risk**:
  - Negligible.

---

## 3. Commit Consolidation Plan

Before merging branch `incremental-language-server` to `main`, the 15 commits should be structured into clean, cohesive units.

### Recommended Rebase Structure (6 Logical Commits)

```
[Current Branch: 15 Commits]
  bc6664e Parse CR-only line endings in DiagnosticRenderer
  44aa8bc Support CR-only line endings in LS position layer
  acc4f24 Document consistent indented-field semantics
  746eeb2 Add block-scoped parse_blocks API and version_marker helper
  f409559 Mark Track 0 tasks complete                      <-- SQUASH into 746eeb2
  c8b6921 Warn on unrecognized information field letters
  36306a4 Warn on strict-mode misplaced constructs in tune bodies
  de70004 Mark Track A tasks complete                      <-- SQUASH into 36306a4
  0349ef9 Introduce DocumentModel with full-rebuild path (T8)
  c97088e Advertise INCREMENTAL sync and apply ranged didChange (T9)
  866f3c4 Implement incremental analysis driver (T10)
  0017b71 Equivalence and fuzz harness for incremental engine (T11)
  20a337f Update plan: mark T11 complete                   <-- SQUASH into 0017b71
  6731013 Reconfigure model and wire config changes (T12)
  b35c619 Update plan: mark T12 complete                   <-- SQUASH into 6731013

[Proposed Consolidated Target: 6 Clean Commits]
  Commit 1: "Add line terminator support across parser and language server (T1, T2)"
            Combines bc6664e, 44aa8bc, acc4f24.
  Commit 2: "Add block-scoped parse_blocks API and complete Track 0 (T3)"
            Combines 746eeb2, f409559.
  Commit 3: "Add strict-mode diagnostics for fields and misplaced constructs (T6, T7)"
            Combines c8b6921, 36306a4, de70004.
  Commit 4: "Implement DocumentModel, incremental sync, and regional analysis (T8, T9, T10)"
            Combines 0349ef9, c97088e, 866f3c4.
  Commit 5: "Add equivalence integration test suite and fuzz harness (T11)"
            Combines 0017b71, 20a337f.
  Commit 6: "Reconfigure model and wire config changes without reparsing (T12)"
            Combines 6731013, b35c619.
```

---

## 4. Actionable Refactoring & Bugfix Plan

Each plan item provides concrete instructions and unambiguous acceptance criteria designed for immediate implementation by any coding agent or engineer.

---

### Task 1: Fix Sequential LSP Change Application & Range Translation (Finding 1)

#### Objective
Apply `TextDocumentContentChangeEvent` sequentially from index `0` to `len - 1`, and update document model range translations to evaluate each edit against its corresponding intermediate state.

#### Files Affected
- [`abc-language-server/src/backend.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs)
- [`abc-language-server/src/document.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs)

#### Implementation Steps
1. In `backend.rs::apply_content_changes`, replace `for change in changes.iter().rev()` with `for change in changes`.
2. In `DocumentModel::apply_changes`, when processing multiple changes:
   - Either translate and accumulate the dirty byte region sequentially through intermediate `LineIndex` states, OR
   - For multi-change batches (`changes.len() > 1`), delegate directly to `DocumentModel::full(new_text, ...)` (full rebuild is always safe and guaranteed correct for rare multi-cursor batches), OR
   - Map each sequential LSP change into a document-relative byte range against the intermediate text and merge the bounding dirty byte region.
3. Update unit test `did_change_applies_multiple_ranged_edits_in_order` in `backend.rs`:
   - Change the expected output string to match sequential execution: Edit 0 (`" GABc"` at char 4) followed by Edit 1 (`" | "` at char 4) produces `"ABCD |  GABc|"`.

#### Acceptance Criteria
- [ ] In `backend.rs`, `apply_content_changes` processes `changes` in forward slice order (`0..changes.len()`).
- [ ] Applying sequential edits `[insert "A" at 0..0, insert "B" at 1..1]` to empty document results in `"AB"`.
- [ ] Multi-edit `didChange` notifications correctly update `DocumentModel` text and diagnostics without panicking or corrupting line spans.
- [ ] `cargo nextest run did_change_applies_multiple_ranged_edits_in_order` passes.

---

### Task 2: Fix UTF-8 BOM Span Accounting in `parse_blocks` (Finding 2)

#### Objective
Ensure that all spans returned by `parse_blocks` (blocks, lines, errors, warnings) are indexed relative to `input` (offset 0 = start of document), even when a UTF-8 BOM is present.

#### Files Affected
- [`abc-parser/src/combinators.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/src/combinators.rs)
- [`abc-parser/tests/blocks.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-parser/tests/blocks.rs)

#### Implementation Steps
1. In `combinators.rs::parse_blocks`:
   - Note whether `input.starts_with('\u{feff}')`.
   - If BOM is present, the BOM consumes 3 bytes. Add 3 to the start and end of all returned `output` blocks, lines, `first_field` spans, `errors`, and `warnings`.
   - Alternatively, update `blocks_combinator` to consume `just('\u{feff}').or_not().ignored()` at the start of Chumsky parsing, identical to `document_parser`.
2. Add a test in `abc-parser/tests/blocks.rs` verifying that parsing `"\u{feff}X:1\nK:C\nC |"` yields block 0 with `lines.start == 3` (or covers the BOM from 0) and `first_field == Some(3..6)`.

#### Acceptance Criteria
- [ ] When `input` begins with `\u{feff}`, `report.output` block line spans start at the true byte offset in `input`.
- [ ] Diagnostics emitted for BOM-prefixed documents align with the exact byte positions in `input`.
- [ ] `cargo nextest run -p abc-parser --test blocks` passes.

---

### Task 3: Fix Header Inheritance in `analyze_tune_block` (Finding 3)

#### Objective
Prevent `OffsetResolver` from applying the tune's `base` offset to file header spans when generating owned ASTs for bar-duration validation.

#### Files Affected
- [`abc-language-server/src/document.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs)
- [`abc-language-server/tests/incremental_equivalence.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/tests/incremental_equivalence.rs)

#### Implementation Steps
1. In `document.rs::analyze_tune_block`:
   - Note that `header_lines` from `file_header` already have document-absolute spans (base = 0).
   - Convert `header_lines` into owned `Line<SimpleSpan<usize>, String>` using `source` directly with base 0, OR
   - Update `OffsetResolver` to accept a base offset specifically for the tune, leaving spans $< base$ untouched (or pass base 0 for header lines).
2. Add a corpus entry to `incremental_equivalence.rs` containing a multi-line file header with comments (`% comment`) followed by multiple tunes, verifying that bar duration analysis succeeds on all tunes.

#### Acceptance Criteria
- [ ] A document with a file header containing `% header comment\nM:4/4\nL:1/8\n\n` followed by a tune at byte offset > 200 executes bar duration analysis without returning `None` from `into_owned`.
- [ ] Bar duration warnings on subsequent tunes accurately point to note spans in the tune.
- [ ] `cargo nextest run -p abc-language-server` passes.

---

### Task 4: Include Leading Separators in `touched_block_indices` (Finding 4)

#### Objective
Ensure that edits landing in leading whitespace or separators before block 0 touch block 0 and proceed with regional parsing.

#### Files Affected
- [`abc-language-server/src/document.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs)

#### Implementation Steps
1. In `document.rs::touched_block_indices`:
   - Add a check for leading edits:
     ```rust
     if edit.start <= blocks[0].lines.start {
         touched.push(0);
     }
     ```
2. Verify that typing comments or newlines at line 0 before block 0 produces `touched == [0]` and avoids `EditError::FullRebuildRequired`.

#### Acceptance Criteria
- [ ] An edit inserting text at `0..0` when block 0 begins at byte 5 does not return `EditError::FullRebuildRequired`.
- [ ] `parse_count` for a single-character insertion at line 0 is $\le 1$.
- [ ] All existing document model tests continue to pass.

---

### Task 5: Optimize Severity-Only Level Changes in `reconfigure` (Finding 5)

#### Objective
Skip re-running `analyze_tune_block` and `legacy_decorations` when transitioning between non-`Off` diagnostic severity levels (`Warning`, `Information`, `Hint`, `Error`).

#### Files Affected
- [`abc-language-server/src/document.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/document.rs)

#### Implementation Steps
1. In `document.rs::DocumentModel::reconfigure`:
   - Inspect `bar_duration_changed`:
     ```rust
     let old_level = self.config.validation.bar_duration;
     let new_level = config.validation.bar_duration;
     let bar_duration_needs_reanalysis = match (old_level, new_level) {
         (DiagnosticLevel::Off, DiagnosticLevel::Off) => false,
         (DiagnosticLevel::Off, _) | (_, DiagnosticLevel::Off) => true,
         _ => false, // Both enabled; cached TuneAnalysis remains valid
     };
     ```
   - Only iterate and call `analyze_tune_block` if `bar_duration_needs_reanalysis` is true.
   - Apply the equivalent logic to `legacy_decoration`.
2. Add a test in `document.rs::tests` asserting that reconfiguring between `DiagnosticLevel::Warning` and `DiagnosticLevel::Error` sets `bar_duration_count == 0` while updating published diagnostic severities.

#### Acceptance Criteria
- [ ] Changing `bar_duration` from `Warning` to `Error` results in `bar_duration_count == 0`.
- [ ] Changing `bar_duration` from `Off` to `Warning` runs analysis and populates warnings.
- [ ] Changing `bar_duration` from `Warning` to `Off` clears all bar duration diagnostics.
- [ ] All published diagnostics reflect the updated LSP `DiagnosticSeverity`.

---

### Task 6: Add Version Guard to `Backend::publish` (Finding 9)

#### Objective
Prevent out-of-order diagnostic publication when concurrent updates complete out of sequence.

#### Files Affected
- [`abc-language-server/src/backend.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/backend.rs)

#### Implementation Steps
1. In `Backend::publish`:
   - Read `state.documents.get(&uri)`.
   - If `current.version > document.version`, return early without calling `self.client.publish_diagnostics`.

#### Acceptance Criteria
- [ ] If `publish` is called with `version: 1` when `state.documents` already holds `version: 2`, no diagnostics are sent to the client.
- [ ] Diagnostics for current or newer versions are published normally.

---

### Task 7: Deduplicate `LineIndex` in Equivalence Tests (Finding 8)

#### Objective
Export `LineIndex` from `abc-language-server` and use it in `incremental_equivalence.rs`, removing the duplicate implementation.

#### Files Affected
- [`abc-language-server/src/lib.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/lib.rs)
- [`abc-language-server/src/position.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/src/position.rs)
- [`abc-language-server/tests/incremental_equivalence.rs`](file:///Users/pixi/src/mauricebarnum/abc-parser/incremental-language-server/abc-language-server/tests/incremental_equivalence.rs)

#### Implementation Steps
1. In `abc-language-server/src/lib.rs`, export `pub use position::LineIndex;`.
2. In `tests/incremental_equivalence.rs`, remove the locally defined `struct LineIndex` and its `impl` block, importing `abc_language_server::LineIndex`.

#### Acceptance Criteria
- [ ] `tests/incremental_equivalence.rs` compiles and passes using `abc_language_server::LineIndex`.
- [ ] Duplicate definition in `tests/incremental_equivalence.rs` is eliminated.
- [ ] `cargo nextest run -p abc-language-server --test incremental_equivalence` passes.

---

## 5. Verification Commands

Upon completing the implementation steps in Section 4, run the following verification suite:

```bash
# 1. Format check
cargo +nightly fmt --check

# 2. Strict Clippy check
CARGO_BUILD_WARNINGS=deny cargo clippy --all-targets

# 3. Nextest full workspace run
cargo nextest run --workspace

# 4. Doc tests run
cargo test --doc
```
