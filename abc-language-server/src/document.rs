// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! Block-oriented document model backing the language server.
//!
//! [`DocumentModel`] owns the full per-document block structure produced by
//! [`abc_parser::parse_blocks`]: the parsed AST for each blank-line-delimited
//! block, the file-header timing snapshot, and the typed diagnostics ready to
//! be converted to LSP [`Diagnostic`]s at publish time.

use std::borrow::Cow;
use std::ops::Range;

use abc_parser::BarDurationOptions;
use abc_parser::BarDurationPickupPolicy;
use abc_parser::Block as ParsedBlock;
use abc_parser::BlocksContext;
use abc_parser::Document as ParsedDocument;
use abc_parser::DocumentItem;
use abc_parser::FieldValue;
use abc_parser::Fraction;
use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::Meter;
use abc_parser::ParseError;
use abc_parser::ParseWarning;
use abc_parser::ParserOptions;
use abc_parser::ResolveError;
use abc_parser::SimpleSpan;
use abc_parser::SourceResolver;
use abc_parser::SourceText;
use abc_parser::Spanned;
use abc_parser::bar_duration_warnings;
use abc_parser::parse_blocks;
use abc_parser::version_marker;
use tower_lsp_server::ls_types::Diagnostic;
use tower_lsp_server::ls_types::DiagnosticSeverity;
use tower_lsp_server::ls_types::DiagnosticTag;
use tower_lsp_server::ls_types::NumberOrString;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::TextDocumentContentChangeEvent;

use crate::analysis::error_kind_code;
use crate::analysis::legacy_decorations;
use crate::config::Config;
use crate::config::DiagnosticLevel;
use crate::position::LineIndex;

/// Per-document timing defaults captured from the file header block.
///
/// The file header supplies default meter and unit-note-length values that
/// every tune inherits unless its own header overrides them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeaderSnapshot {
    /// `M:` meter value declared in the file header.
    pub meter: Option<Meter>,
    /// `L:` unit-note-length value declared in the file header.
    pub unit_length: Option<Fraction>,
}

/// Bar-duration analysis results for one tune block.
///
/// Spans are slice-relative; the document model rebases them by adding
/// [`BlockRecord::base`] before publication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuneAnalysis {
    /// Warnings emitted by [`abc_parser::bar_duration_warnings`] over
    /// the synthetic single-tune document built for this block. Spans are
    /// slice-relative.
    pub warnings: Vec<ParseWarning<SimpleSpan<usize>>>,
}

/// One source-backed line in the file header (or any header block).
type HeaderLine =
    Spanned<Line<SimpleSpan<usize>, SourceText<SimpleSpan<usize>>>, SimpleSpan<usize>>;

/// Typed diagnostic data retained per block in document-absolute
/// coordinates.
///
/// Storing the diagnostics here (instead of baking them into LSP
/// [`Diagnostic`]s at parse time) lets the incremental driver shift the
/// spans of blocks that follow the edited region without having to
/// re-parse those blocks. The publish step converts these records into
/// [`Diagnostic`]s at the end of every update.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockDiagnostics {
    /// Parser errors in document-absolute byte ranges, paired with their
    /// LSP code and message.
    pub errors: Vec<TypedError>,
    /// Parser warnings in document-absolute byte ranges.
    pub warnings: Vec<TypedWarning>,
    /// Legacy-decoration matches in document-absolute byte ranges.
    pub legacy_decoration_ranges: Vec<Range<usize>>,
}

/// One parser error with a document-absolute byte span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedError {
    /// Document-absolute byte range that the diagnostic highlights.
    pub span: Range<usize>,
    /// LSP code emitted at publish time.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// One parser warning with a document-absolute byte span and related
/// spans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedWarning {
    /// Document-absolute byte range that the diagnostic highlights.
    pub span: Range<usize>,
    /// LSP code emitted at publish time.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Related spans that give the warning extra context (for example,
    /// the preceding field-led block for a `MissingReference` hint).
    pub related: Vec<TypedRelatedSpan>,
}

/// One related span attached to a [`TypedWarning`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedRelatedSpan {
    /// Document-absolute byte range of the related location.
    pub span: Range<usize>,
    /// Human-readable explanation of why this location matters.
    pub message: String,
}

/// One blank-line-delimited block retained by the model.
///
/// Spans inside [`Self::parsed`] are slice-relative; the document model
/// rebases them to document-absolute coordinates by adding [`Self::base`]
/// to each offset. [`Self::tune_analysis`] reuses that offset: the
/// warnings it carries are slice-relative and must be rebased before
/// publication. [`Self::diagnostics`] holds document-absolute spans and
/// is what the publish step walks to build the published list.
#[derive(Clone, Debug)]
pub struct BlockRecord {
    /// Document-absolute byte range covering the block's content lines,
    /// excluding any blank-line separators that border the block.
    pub lines: Range<usize>,
    /// Offset that converts the parsed block's slice-relative spans into
    /// document-absolute spans.
    pub base: isize,
    /// Slice-relative parse output for this block.
    pub parsed: ParsedBlock,
    /// Typed diagnostics produced by parsing this block, in
    /// document-absolute coordinates.
    pub diagnostics: BlockDiagnostics,
    /// Bar-duration analysis for the block's tune, when present.
    pub tune_analysis: Option<TuneAnalysis>,
}

/// One immutable synchronized document version.
#[derive(Clone, Debug)]
pub struct DocumentModel {
    /// Document version supplied by the LSP client.
    pub version: i32,
    /// Position encoding negotiated during initialization.
    pub encoding: PositionEncodingKind,
    /// Configuration effective for this document version.
    pub config: Config,
    /// Effective strict interpretation flag (config or version marker).
    pub strict: bool,
    /// Complete document text.
    pub text: String,
    /// Line index used for all LSP position conversions.
    pub index: LineIndex,
    /// File-header timing snapshot (when block 0 resolved as a header).
    pub header: HeaderSnapshot,
    /// All parsed blocks in source order.
    pub blocks: Vec<BlockRecord>,
    /// Diagnostics ready to publish.
    pub diagnostics: Vec<Diagnostic>,
    /// True if any parse error was emitted.
    pub has_errors: bool,
    /// Number of blocks parsed by the most recent update operation
    /// (full rebuild or incremental). Used by tests to assert that
    /// incremental edits touch only the blocks the invalidation rules
    /// say they must touch.
    pub parse_count: u32,
    /// Number of bar-duration analyses re-run by the most recent update
    /// operation. Used by tests to assert D3 / D6 invalidation.
    pub bar_duration_count: u32,
}

impl DocumentModel {
    /// Rebuilds the model from `text`, parsing the entire document in one
    /// [`abc_parser::parse_blocks`] call.
    ///
    /// This is the safe, full-fidelity path used at document open, on
    /// configuration changes, and as the defensive fallback whenever a
    /// more selective update is unavailable.
    pub fn full(
        text: String,
        version: i32,
        encoding: PositionEncodingKind,
        config: Config,
    ) -> Self {
        let index = LineIndex::new(text.clone());
        let first_line = text.split(['\r', '\n']).next().unwrap_or("");
        let marker_strict = version_marker(first_line) == Some(true);
        let strict = marker_strict || config.validation.strict;
        let options = ParserOptions::new().strict(strict);
        let context = BlocksContext {
            at_document_start: true,
            previous_field_led: None,
        };
        let mut report = parse_blocks(&text, options, context);
        report
            .errors
            .sort_by_key(|error| (error.span.start, error.span.end));

        let mut blocks = build_region_blocks(
            report.output.as_deref().unwrap_or_default(),
            0,
            &text,
            config.validation.bar_duration != DiagnosticLevel::Off,
        );
        let parse_count = blocks.len() as u32;

        let header = blocks
            .first()
            .filter(|record| !record.parsed.header.is_empty())
            .map(|record| header_snapshot(&record.parsed.header))
            .unwrap_or_default();

        // Per-block typed diagnostics. Each block contributes its own
        // bucket; the publish step concatenates them in the order
        // Analysis::new produced today (errors, warnings, bar-duration,
        // legacy-decoration).
        populate_block_diagnostics(&mut blocks, &report.errors, &report.warnings, None);
        if config.validation.legacy_decoration != DiagnosticLevel::Off {
            populate_legacy_decorations(&mut blocks, &text);
        }

        // Per-tune bar-duration warnings. The synthetic single-tune document
        // reuses the file-header lines from block 0 when present so each
        // tune is analysed with the correct defaults.
        let file_header = blocks
            .first()
            .filter(|record| !record.parsed.header.is_empty())
            .map(|record| record.parsed.header.clone());
        let mut bar_duration_count = 0;
        if config.validation.bar_duration != DiagnosticLevel::Off {
            for record in &mut blocks {
                if let Some(warnings) =
                    analyze_tune_block(&record.parsed, record.base, file_header.as_ref(), &text)
                {
                    bar_duration_count += 1;
                    record.tune_analysis = Some(TuneAnalysis { warnings });
                }
            }
        }

        let has_errors = blocks
            .iter()
            .any(|record| !record.diagnostics.errors.is_empty());
        let diagnostics = assemble_diagnostics(&blocks, &index, &encoding, config);

        Self {
            version,
            encoding,
            config,
            strict,
            text,
            index,
            header,
            blocks,
            diagnostics,
            has_errors,
            parse_count,
            bar_duration_count,
        }
    }

    /// Returns whether any block contains a tune.
    pub fn has_tunes(&self) -> bool {
        self.blocks.iter().any(|record| {
            record
                .parsed
                .items
                .iter()
                .any(|item| matches!(item.value, DocumentItem::Tune(_)))
        })
    }

    /// Applies a batch of LSP `didChange` events to `previous`,
    /// returning the next document model.
    ///
    /// The invalidation rules follow the section "Invalidation rules"
    /// of the incremental design: the touched blocks are re-parsed in
    /// one `parse_blocks` call over the dirty region; if the region's
    /// last block's `first_field` changed (including a `Some`/`None`
    /// flip or a different span value) the region is extended by exactly
    /// one block and re-parsed, terminating the cascade. Following
    /// blocks shift their document-absolute spans by the text-length
    /// delta. Bar-duration analysis is re-run on every tune in the
    /// region, and on every tune when the file header's `M:`/`L:`
    /// defaults changed. Legacy-decoration scans are re-run on every
    /// block in the region.
    ///
    /// Returns [`EditError::FullRebuildRequired`] when any edit touches
    /// line 1 (the `%abc-` version marker) or has `range: None`, when
    /// the previous text is unavailable, or when the regional parse
    /// does not consume the entire slice. Callers should rebuild with
    /// [`Self::full`] in that case.
    #[allow(clippy::too_many_lines)]
    pub fn apply_changes(
        previous: &Self,
        changes: &[TextDocumentContentChangeEvent],
        text: String,
    ) -> Result<Self, EditError> {
        // Fallback conditions.
        if changes.iter().any(|change| change.range.is_none()) {
            return Err(EditError::FullRebuildRequired);
        }
        if changes.len() > 1 {
            return Ok(Self::full(
                text,
                previous.version,
                previous.encoding.clone(),
                previous.config,
            ));
        }
        if line_one_edited(previous, changes, &text) {
            return Err(EditError::FullRebuildRequired);
        }

        // Convert LSP ranges to byte ranges. Each edit covers zero or more
        // existing blocks.
        let edits: Vec<Range<usize>> = changes
            .iter()
            .filter_map(|change| {
                change
                    .range
                    .and_then(|range| previous.index.byte_range(range, &previous.encoding))
            })
            .collect();
        if edits.len() != changes.len() {
            return Err(EditError::FullRebuildRequired);
        }

        if previous.blocks.is_empty() {
            return Err(EditError::FullRebuildRequired);
        }

        // 1. Region selection.
        let touched = touched_block_indices(&previous.blocks, &edits);
        if touched.is_empty() {
            return Err(EditError::FullRebuildRequired);
        }
        let mut first_touched = touched[0];
        let mut last_touched = touched[touched.len() - 1];

        // An edit touching leading separators or byte 0 starts region at 0.
        let region_start = if first_touched == 0
            || edits
                .iter()
                .any(|e| e.start <= previous.blocks[0].lines.start)
        {
            first_touched = 0;
            0
        } else {
            previous.blocks[first_touched].lines.start
        };

        let length_delta = text.len().cast_signed() - previous.text.len().cast_signed();
        let mut region_end = if last_touched + 1 < previous.blocks.len() {
            (previous.blocks[last_touched + 1].lines.start.cast_signed() + length_delta)
                .max(region_start.cast_signed())
                .min(text.len().cast_signed())
                .cast_unsigned()
        } else {
            text.len()
        };

        // 2. Context seeding.
        let previous_field_led = if first_touched > 0 {
            previous.blocks[first_touched - 1]
                .parsed
                .first_field
                .map(|span| {
                    let base = previous.blocks[first_touched - 1].base;
                    let start = (span.start.cast_signed() + base).max(0).cast_unsigned();
                    let end = (span.end.cast_signed() + base).max(0).cast_unsigned();
                    SimpleSpan::from(start..end)
                })
        } else {
            None
        };
        let at_document_start = region_start == 0;
        let context = BlocksContext {
            at_document_start,
            previous_field_led,
        };

        let options = ParserOptions::new().strict(previous.strict);

        // 3. Region parse.
        let slice = &text[region_start..region_end];
        let mut report = parse_blocks(slice, options, context);
        report
            .errors
            .sort_by_key(|error| (error.span.start, error.span.end));

        let mut parsed_region = report
            .output
            .clone()
            .ok_or(EditError::FullRebuildRequired)?;
        let mut parse_count = parsed_region.len() as u32;

        // 4. Forward cascade (D4).
        let previous_last_first_field =
            previous.blocks[last_touched]
                .parsed
                .first_field
                .map(|span| {
                    let base = previous.blocks[last_touched].base;
                    let start = (span.start.cast_signed() + base).max(0).cast_unsigned();
                    let end = (span.end.cast_signed() + base).max(0).cast_unsigned();
                    SimpleSpan::from(start..end)
                });
        let new_last_first_field = parsed_region.last().and_then(|block| {
            block.first_field.map(|span| {
                SimpleSpan::from((span.start + region_start)..(span.end + region_start))
            })
        });
        if new_last_first_field != previous_last_first_field
            && last_touched + 1 < previous.blocks.len()
        {
            last_touched += 1;
            region_end = if last_touched + 1 < previous.blocks.len() {
                (previous.blocks[last_touched + 1].lines.start.cast_signed() + length_delta)
                    .max(region_start.cast_signed())
                    .min(text.len().cast_signed())
                    .cast_unsigned()
            } else {
                text.len()
            };
            let extended_slice = &text[region_start..region_end];
            let mut extended_report = parse_blocks(extended_slice, options, context);
            extended_report
                .errors
                .sort_by_key(|error| (error.span.start, error.span.end));
            parsed_region = extended_report
                .output
                .clone()
                .ok_or(EditError::FullRebuildRequired)?;
            parse_count = parsed_region.len() as u32;
            report = extended_report;
        }

        // 5. Splice.
        let mut new_region_blocks = build_region_blocks(
            &parsed_region,
            region_start,
            &text,
            previous.config.validation.bar_duration != DiagnosticLevel::Off,
        );
        populate_block_diagnostics_for_region(
            &mut new_region_blocks,
            &report.errors,
            &report.warnings,
            context.previous_field_led,
        );
        if previous.config.validation.legacy_decoration != DiagnosticLevel::Off {
            populate_legacy_decorations(&mut new_region_blocks, &text);
        }

        let mut blocks = previous.blocks.clone();
        blocks.splice(first_touched..=last_touched, new_region_blocks);

        let replace_count = parsed_region.len();
        let region_end_idx = first_touched + replace_count;
        let delta = length_delta;
        let edit_start = edits.iter().map(|e| e.start).min().unwrap_or(0);
        for record in blocks.iter_mut().skip(region_end_idx) {
            record.lines.start = (record.lines.start.cast_signed() + delta)
                .max(0)
                .cast_unsigned();
            record.lines.end = (record.lines.end.cast_signed() + delta)
                .max(0)
                .cast_unsigned();
            record.base += delta;
            shift_block_diagnostics(&mut record.diagnostics, delta, edit_start);
        }

        let header_changed = header_defaults_changed(&previous.header, &blocks);

        let file_header = blocks
            .first()
            .filter(|record| !record.parsed.header.is_empty())
            .map(|record| record.parsed.header.clone());
        let mut bar_duration_count = 0;
        if previous.config.validation.bar_duration != DiagnosticLevel::Off {
            for (idx, record) in blocks.iter_mut().enumerate() {
                let in_reparsed_region = idx >= first_touched && idx < region_end_idx;
                if in_reparsed_region || header_changed {
                    if let Some(warnings) =
                        analyze_tune_block(&record.parsed, record.base, file_header.as_ref(), &text)
                    {
                        bar_duration_count += 1;
                        record.tune_analysis = Some(TuneAnalysis { warnings });
                    } else {
                        record.tune_analysis = None;
                    }
                }
            }
        }

        let index = LineIndex::new(text.clone());
        let first_line = text.split(['\r', '\n']).next().unwrap_or("");
        let marker_strict = version_marker(first_line) == Some(true);
        let strict = marker_strict || previous.config.validation.strict;
        let header = blocks
            .first()
            .filter(|record| !record.parsed.header.is_empty())
            .map(|record| header_snapshot(&record.parsed.header))
            .unwrap_or_default();
        let has_errors = blocks
            .iter()
            .any(|record| !record.diagnostics.errors.is_empty());
        let diagnostics =
            assemble_diagnostics(&blocks, &index, &previous.encoding, previous.config);

        Ok(Self {
            version: previous.version,
            encoding: previous.encoding.clone(),
            config: previous.config,
            strict,
            text,
            index,
            header,
            blocks,
            diagnostics,
            has_errors,
            parse_count,
            bar_duration_count,
        })
    }
}

/// Outcome of [`DocumentModel::apply_changes`] when a more selective
/// update is not possible. Callers should rebuild with
/// [`DocumentModel::full`] in this case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditError {
    /// The edit could not be applied incrementally; rebuild the model
    /// from scratch.
    FullRebuildRequired,
}

/// Resolver that re-bases slice-relative spans by a fixed offset and
/// resolves them against the full document.
///
/// Used by [`abc_parser::IntoOwnedAst::into_owned`] when materialising a
/// synthetic single-tune [`abc_parser::Document`] for bar-duration
/// analysis. The synthetic document carries slice-relative spans; the
/// resolver re-bases them onto the full source so the owned document's
/// text values can be materialised from the original bytes.
pub struct OffsetResolver<'a> {
    /// Offset that converts slice-relative spans into document-absolute
    /// spans.
    pub base: isize,
    /// Full document text.
    pub source: &'a str,
}

impl<'a> OffsetResolver<'a> {
    /// Creates a resolver that adds `base` to every span before delegating
    /// to `source`.
    pub const fn new(base: isize, source: &'a str) -> Self {
        Self { base, source }
    }

    fn rebase(&self, span: &SimpleSpan<usize>) -> SimpleSpan<usize> {
        let start = (span.start.cast_signed() + self.base)
            .max(0)
            .cast_unsigned();
        let end = (span.end.cast_signed() + self.base).max(0).cast_unsigned();
        SimpleSpan::from(start..end)
    }
}

impl SourceResolver<SimpleSpan<usize>> for OffsetResolver<'_> {
    type Error = ResolveError;

    fn resolve<'src>(&'src self, span: &SimpleSpan<usize>) -> Result<Cow<'src, str>, Self::Error> {
        let rebased = self.rebase(span);
        self.source.resolve(&rebased)
    }

    fn full_source(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.source))
    }

    fn diagnostic_range(&self, span: &SimpleSpan<usize>) -> Option<Range<usize>> {
        let rebased = self.rebase(span);
        self.source.diagnostic_range(&rebased)
    }
}

fn header_snapshot(header_lines: &[HeaderLine]) -> HeaderSnapshot {
    let mut snapshot = HeaderSnapshot::default();
    for line in header_lines {
        if let Line::Field(field) = &line.value {
            match &field.value {
                FieldValue::Meter(meter) => snapshot.meter = Some(meter.clone()),
                FieldValue::UnitLength(unit) => snapshot.unit_length = Some(*unit),
                _ => {}
            }
        }
    }
    snapshot
}

/// Runs bar-duration analysis over the tune contained in `block`, if any.
///
/// Builds a synthetic single-tune [`ParsedDocument`] carrying the file
/// header (if any) plus this block's tune, resolves every span against
/// the full document via [`OffsetResolver`], and hands the owned document
/// to [`abc_parser::bar_duration_warnings`]. Slice-relative warning spans
/// are returned verbatim so the caller can rebase them against
/// [`BlockRecord::base`].
fn analyze_tune_block(
    block: &ParsedBlock,
    base: isize,
    file_header: Option<&Vec<HeaderLine>>,
    source: &str,
) -> Option<Vec<ParseWarning<SimpleSpan<usize>>>> {
    let tune = block.items.iter().find_map(|item| match &item.value {
        DocumentItem::Tune(tune) => Some(tune.clone()),
        _ => None,
    })?;
    let owned_header = match file_header {
        Some(lines) => {
            let mut owned = Vec::with_capacity(lines.len());
            for line in lines {
                owned.push(line.clone().into_owned(source).ok()?);
            }
            owned
        }
        None => Vec::new(),
    };
    let resolver = OffsetResolver::new(base, source);
    let owned_tune = tune.into_owned(&resolver).ok()?;
    let owned = ParsedDocument {
        header: owned_header,
        items: vec![Spanned {
            value: DocumentItem::Tune(owned_tune),
            span: block.span,
        }],
    };
    let options = BarDurationOptions::new()
        .pickup_policy(BarDurationPickupPolicy::OpeningBar)
        .check_trailing_bar(false);
    Some(bar_duration_warnings(&owned, options))
}

const fn severity(level: DiagnosticLevel) -> Option<DiagnosticSeverity> {
    match level {
        DiagnosticLevel::Off => None,
        DiagnosticLevel::Hint => Some(DiagnosticSeverity::HINT),
        DiagnosticLevel::Information => Some(DiagnosticSeverity::INFORMATION),
        DiagnosticLevel::Warning => Some(DiagnosticSeverity::WARNING),
        DiagnosticLevel::Error => Some(DiagnosticSeverity::ERROR),
    }
}

fn build_diagnostic(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    range: Range<usize>,
    severity: DiagnosticSeverity,
    code: &'static str,
    message: String,
    tags: Option<Vec<DiagnosticTag>>,
) -> Option<Diagnostic> {
    Some(Diagnostic::new(
        index.lsp_range(range, encoding)?,
        Some(severity),
        Some(NumberOrString::String(code.to_owned())),
        Some("abc-parser".to_owned()),
        message,
        None,
        tags,
    ))
}

const fn rebase(span: SimpleSpan<usize>, base: isize) -> Range<usize> {
    let s = span.start.cast_signed() + base;
    let e = span.end.cast_signed() + base;
    let start = if s > 0 { s.cast_unsigned() } else { 0 };
    let end = if e > 0 { e.cast_unsigned() } else { 0 };
    start..end
}

fn shift_range(range: &mut Range<usize>, delta: isize) {
    let start = range.start.cast_signed() + delta;
    let end = range.end.cast_signed() + delta;
    range.start = start.max(0).cast_unsigned();
    range.end = end.max(0).cast_unsigned();
}

fn shift_typed_error(error: &mut TypedError, delta: isize) {
    shift_range(&mut error.span, delta);
}

fn shift_typed_warning(warning: &mut TypedWarning, delta: isize, edit_start: usize) {
    shift_range(&mut warning.span, delta);
    for related in &mut warning.related {
        if related.span.start >= edit_start {
            shift_range(&mut related.span, delta);
        }
    }
}

fn shift_block_diagnostics(diagnostics: &mut BlockDiagnostics, delta: isize, edit_start: usize) {
    if delta == 0 {
        return;
    }
    diagnostics
        .errors
        .iter_mut()
        .for_each(|error| shift_typed_error(error, delta));
    diagnostics
        .warnings
        .iter_mut()
        .for_each(|warning| shift_typed_warning(warning, delta, edit_start));
    diagnostics
        .legacy_decoration_ranges
        .iter_mut()
        .for_each(|range| shift_range(range, delta));
}

/// Populates every [`BlockRecord`]'s [`BlockDiagnostics`] bucket from the
/// parser's flat error and warning lists.
///
/// Each error or warning is assigned to the block whose slice-relative
/// span contains the diagnostic's start byte; the diagnostic's span and
/// related spans are then re-based from slice-relative to
/// document-absolute using that block's [`BlockRecord::base`]. Diagnostics
/// that fall outside every parsed block (in a separator run, or past the
/// end of the slice) are attached to the closest neighbouring block so
/// they remain visible to the editor.
fn populate_block_diagnostics(
    blocks: &mut [BlockRecord],
    errors: &[ParseError<SimpleSpan<usize>>],
    warnings: &[ParseWarning<SimpleSpan<usize>>],
    seeded_previous_field_led: Option<SimpleSpan<usize>>,
) {
    for record in blocks.iter_mut() {
        record.diagnostics = BlockDiagnostics::default();
    }
    for error in errors {
        if let Some(idx) = find_block_for_span(blocks, error.span.start) {
            blocks[idx].diagnostics.errors.push(TypedError {
                span: rebase(error.span, blocks[idx].base),
                code: error_kind_code(error.kind),
                message: error.message.clone(),
            });
        }
    }
    let first_field_led_idx = blocks
        .iter()
        .position(|b| b.parsed.first_field.is_some())
        .unwrap_or(blocks.len());
    for warning in warnings {
        if let Some(idx) = find_block_for_span(blocks, warning.span.start) {
            let related = warning
                .related
                .iter()
                .map(|related| {
                    let span = if idx < first_field_led_idx && seeded_previous_field_led.is_some() {
                        related.span.start..related.span.end
                    } else {
                        rebase(related.span, blocks[idx].base)
                    };
                    TypedRelatedSpan {
                        span,
                        message: related.message.clone(),
                    }
                })
                .collect();
            blocks[idx].diagnostics.warnings.push(TypedWarning {
                span: rebase(warning.span, blocks[idx].base),
                code: error_kind_code(warning.kind),
                message: warning.message.clone(),
                related,
            });
        }
    }
}

/// Returns the index of the block whose slice-relative content span
/// covers `slice_offset`. When the offset falls in a separator run
/// between two blocks, the closest neighbour is returned so the
/// diagnostic remains attached to a visible block.
fn find_block_for_span(blocks: &[BlockRecord], slice_offset: usize) -> Option<usize> {
    if blocks.is_empty() {
        return None;
    }
    for (idx, _) in blocks.iter().enumerate() {
        if let Some(next) = blocks.get(idx + 1) {
            if slice_offset < next.parsed.span.start {
                return Some(idx);
            }
        } else {
            return Some(idx);
        }
    }
    Some(blocks.len() - 1)
}

/// Scans one block's source slice for legacy `+name+` decoration matches
/// and returns them in document-absolute byte ranges.
fn populate_legacy_decorations(blocks: &mut [BlockRecord], source: &str) {
    for record in blocks.iter_mut() {
        let block_text = &source[record.lines.clone()];
        record.diagnostics.legacy_decoration_ranges = legacy_decorations(block_text)
            .map(|range| record.lines.start + range.start..record.lines.start + range.end)
            .collect();
    }
}

/// Returns true when line 1's version-marker status changed, or when
/// a version-marker line was edited, inserted, or deleted.
fn line_one_edited(
    previous: &DocumentModel,
    _changes: &[TextDocumentContentChangeEvent],
    text: &str,
) -> bool {
    let new_first_line = text.lines().next().unwrap_or("");
    let old_first_line = previous.text.lines().next().unwrap_or("");
    let new_strict = version_marker(new_first_line) == Some(true);
    let old_strict = version_marker(old_first_line) == Some(true);
    new_strict != old_strict || ((new_strict || old_strict) && new_first_line != old_first_line)
}

fn ranges_intersect(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() {
        (b.start..=b.end).contains(&a.start)
    } else if b.is_empty() {
        (a.start..=a.end).contains(&b.start)
    } else {
        a.start < b.end && a.end > b.start
    }
}

/// Returns the indices of every block whose document-absolute line span
/// intersects any edit range. An edit landing inside a separator run
/// touches the blocks on either side.
fn touched_block_indices(blocks: &[BlockRecord], edits: &[Range<usize>]) -> Vec<usize> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut touched: Vec<usize> = Vec::new();
    for edit in edits {
        if edit.start <= blocks[0].lines.start {
            touched.push(0);
        }
        for (idx, block) in blocks.iter().enumerate() {
            if ranges_intersect(edit, &block.lines) {
                touched.push(idx);
            }
            if let Some(next) = blocks.get(idx + 1) {
                let sep = block.lines.end..next.lines.start;
                if ranges_intersect(edit, &sep) {
                    touched.push(idx);
                    touched.push(idx + 1);
                }
            } else if edit.end >= block.lines.end {
                touched.push(idx);
            }
        }
    }
    touched.sort_unstable();
    touched.dedup();
    touched
}

/// Returns true iff the file header's `M:` meter or `L:` unit length
/// changed in the new model, which forces every tune's bar-duration
/// analysis to re-run (D3).
fn header_defaults_changed(previous: &HeaderSnapshot, blocks: &[BlockRecord]) -> bool {
    let current = blocks
        .first()
        .filter(|record| !record.parsed.header.is_empty())
        .map(|record| header_snapshot(&record.parsed.header))
        .unwrap_or_default();
    current != *previous
}

/// Re-buckets the parser's flat error and warning lists into the
/// per-block [`BlockDiagnostics`] buckets of one parse slice.
fn populate_block_diagnostics_for_region(
    blocks: &mut [BlockRecord],
    errors: &[ParseError<SimpleSpan<usize>>],
    warnings: &[ParseWarning<SimpleSpan<usize>>],
    seeded_previous_field_led: Option<SimpleSpan<usize>>,
) {
    populate_block_diagnostics(blocks, errors, warnings, seeded_previous_field_led);
}

/// Constructs [`BlockRecord`]s for a freshly parsed region.
fn build_region_blocks(
    parsed_region: &[ParsedBlock],
    region_base: usize,
    text: &str,
    needs_diagnostics: bool,
) -> Vec<BlockRecord> {
    parsed_region
        .iter()
        .map(|parsed_block| {
            let absolute_start = region_base + parsed_block.span.start;
            let absolute_end = region_base + parsed_block.span.end;
            let mut record = BlockRecord {
                lines: absolute_start..absolute_end,
                base: region_base.cast_signed(),
                parsed: parsed_block.clone(),
                diagnostics: BlockDiagnostics::default(),
                tune_analysis: None,
            };
            if needs_diagnostics {
                let block_text = &text[record.lines.clone()];
                record.diagnostics.legacy_decoration_ranges = legacy_decorations(block_text)
                    .map(|range| record.lines.start + range.start..record.lines.start + range.end)
                    .collect();
            }
            record
        })
        .collect()
}

/// Assembles the publishable diagnostics list from the per-block typed
/// diagnostics, preserving the ordering `Analysis::new` produced today:
/// parser errors, parser warnings, bar-duration warnings, legacy
/// decorations, each in block order (block order is span order, so the
/// per-block buckets are already sorted internally).
fn assemble_diagnostics(
    blocks: &[BlockRecord],
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    config: Config,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    // Parser errors in source order.
    for record in blocks {
        for error in &record.diagnostics.errors {
            if let Some(diagnostic) = build_diagnostic(
                index,
                encoding,
                error.span.clone(),
                DiagnosticSeverity::ERROR,
                error.code,
                error.message.clone(),
                None,
            ) {
                diagnostics.push(diagnostic);
            }
        }
    }
    // Parser warnings in source order.
    for record in blocks {
        for warning in &record.diagnostics.warnings {
            let level = if warning.code == "missing-reference" {
                config.validation.ambiguous_music
            } else {
                DiagnosticLevel::Warning
            };
            let Some(severity) = severity(level) else {
                continue;
            };
            if let Some(diagnostic) = build_diagnostic(
                index,
                encoding,
                warning.span.clone(),
                severity,
                warning.code,
                warning.message.clone(),
                None,
            ) {
                diagnostics.push(diagnostic);
            }
        }
    }
    // Bar-duration warnings in tune (block) order.
    if let Some(level) = severity(config.validation.bar_duration) {
        for record in blocks {
            if let Some(analysis) = &record.tune_analysis {
                for warning in &analysis.warnings {
                    let span = rebase(warning.span, record.base);
                    if let Some(diagnostic) = build_diagnostic(
                        index,
                        encoding,
                        span,
                        level,
                        "bar-duration",
                        bar_duration_message(warning.message.clone()),
                        None,
                    ) {
                        diagnostics.push(diagnostic);
                    }
                }
            }
        }
    }
    // Legacy decoration warnings in block order.
    if let Some(level) = severity(config.validation.legacy_decoration) {
        for record in blocks {
            for range in &record.diagnostics.legacy_decoration_ranges {
                if let Some(diagnostic) = build_diagnostic(
                    index,
                    encoding,
                    range.clone(),
                    level,
                    "legacy-decoration",
                    "legacy +name+ decoration; prefer !name!".to_owned(),
                    Some(vec![DiagnosticTag::DEPRECATED]),
                ) {
                    diagnostics.push(diagnostic);
                }
            }
        }
    }
    diagnostics
}

fn bar_duration_message(message: String) -> String {
    if let Some(prefix) = message.strip_suffix(" beats under the effective meter") {
        return prefix.to_owned();
    }
    if let Some(prefix) = message.strip_suffix(" beat under the effective meter") {
        return prefix.to_owned();
    }
    message
}

#[cfg(test)]
mod tests {
    use tower_lsp_server::ls_types::Range as LspRange;

    use super::*;
    use crate::analysis::Analysis;

    fn diagnostics_via_analysis(text: &str) -> (Vec<Diagnostic>, bool) {
        let index = LineIndex::new(text.to_owned());
        let analysis = Analysis::new(&index, &PositionEncodingKind::UTF16, Config::default());
        (analysis.diagnostics, analysis.has_errors)
    }

    fn diagnostics_via_model(text: &str) -> (Vec<Diagnostic>, bool) {
        let model = DocumentModel::full(
            text.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        (model.diagnostics, model.has_errors)
    }

    fn severity_label(severity: Option<DiagnosticSeverity>) -> &'static str {
        match severity {
            Some(DiagnosticSeverity::ERROR) => "ERROR",
            Some(DiagnosticSeverity::WARNING) => "WARNING",
            Some(DiagnosticSeverity::INFORMATION) => "INFORMATION",
            Some(DiagnosticSeverity::HINT) => "HINT",
            _ => "NONE",
        }
    }

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

    #[test]
    fn golden_diagnostics_match_existing_analysis() {
        let corpus: &[(&str, &str)] = &[
            ("simple valid", "X:1\nT:Title\nK:C\nCDEF |\n"),
            ("bar mismatch", "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C |\n"),
            ("orphan music", "X:1\nK:C\nCDEF |\n\nCDEF |\n"),
            ("legacy decoration", "X:1\nK:C\n+trill+C\n"),
            ("invalid field", "X:1\nM:nope\nK:C\nC |\n"),
            ("missing key", "X:1\nCDEF |\n"),
            ("free text", "X:1\nK:C\nC |\n\njust text\n"),
            ("crlf line endings", "X:1\r\nK:C\r\nC |\r\n"),
            ("cr only line endings", "X:1\rK:C\rC |\r"),
            ("two tunes", "X:1\nT:A\nK:C\nC |\n\nX:2\nT:B\nK:G\nG |\n"),
        ];
        for (name, source) in corpus {
            let (model_diags, model_has_errors) = diagnostics_via_model(source);
            let (analysis_diags, analysis_has_errors) = diagnostics_via_analysis(source);
            assert_eq!(
                model_has_errors, analysis_has_errors,
                "{name}: has_errors differs"
            );
            assert_eq!(
                model_diags.len(),
                analysis_diags.len(),
                "{name}: diagnostic count differs\nmodel:    {model_diags:#?}\nanalysis: {analysis_diags:#?}"
            );
            for (index, (model, analysis)) in
                model_diags.iter().zip(analysis_diags.iter()).enumerate()
            {
                assert_eq!(
                    severity_label(model.severity),
                    severity_label(analysis.severity),
                    "{name}#{index}: severity"
                );
                assert_eq!(
                    code_label(model),
                    code_label(analysis),
                    "{name}#{index}: code"
                );
                assert_eq!(model.message, analysis.message, "{name}#{index}: message");
                assert_eq!(model.range, analysis.range, "{name}#{index}: range");
            }
        }
    }

    #[test]
    fn block_boundaries_match_parsed_blocks_for_full_document() {
        // Block boundaries are computed from the original source by
        // scanning for blank-line separators, while parsed blocks live
        // in the post-prologue slice. The boundaries cover the full
        // document content; the `base` is the shared prologue offset.
        let source = "X:1\nT:A\nK:C\nCDEF |\n\nX:2\nT:B\nK:G\nGABc |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert_eq!(model.blocks.len(), 2);
        assert_eq!(model.blocks[0].lines, 0..18);
        assert_eq!(model.blocks[1].lines, 20..38);
        assert_eq!(model.blocks[0].base, 0);
        assert_eq!(model.blocks[1].base, 0);
    }

    #[test]
    fn file_header_timing_snapshot_captures_first_block_default() {
        let source = "M:4/4\nL:1/8\n\nX:1\nK:C\nC |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert!(model.header.meter.is_some());
        assert!(model.header.unit_length.is_some());
        assert!(model.blocks.iter().any(|record| {
            record
                .parsed
                .items
                .iter()
                .any(|item| matches!(item.value, abc_parser::DocumentItem::Tune(_)))
        }));
    }

    #[test]
    fn document_without_tunes_emits_no_bar_duration_diagnostics() {
        let source = "X:1\nT:Title\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert!(
            model
                .diagnostics
                .iter()
                .all(|diagnostic| { code_label(diagnostic) != "bar-duration" })
        );
    }

    #[test]
    fn strict_marker_forces_strict_mode() {
        let source = "%abc-2.1\nX:1\nK:C\nC |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert!(model.strict);
    }

    #[test]
    fn crlf_line_endings_produce_identical_diagnostics() {
        let lf = "X:1\nM:4/4\nL:1/4\nK:C\nCDEF | C | CDEF | C |\n";
        let crlf = "X:1\r\nM:4/4\r\nL:1/4\r\nK:C\r\nCDEF | C | CDEF | C |\r\n";
        let (model_lf, _) = diagnostics_via_model(lf);
        let (model_crlf, _) = diagnostics_via_model(crlf);
        assert_eq!(model_lf.len(), model_crlf.len());
        for (a, b) in model_lf.iter().zip(model_crlf.iter()) {
            assert_eq!(a.message, b.message);
            assert_eq!(a.severity, b.severity);
            assert_eq!(code_label(a), code_label(b));
        }
    }

    use tower_lsp_server::ls_types::Position;

    fn encode_change(range: Option<LspRange>, text: &str) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range,
            range_length: None,
            text: text.to_owned(),
        }
    }

    fn change_at_line(
        start_line: u32,
        end_line: u32,
        text: &str,
    ) -> TextDocumentContentChangeEvent {
        encode_change(
            Some(LspRange::new(
                Position::new(start_line, 0),
                Position::new(end_line, 0),
            )),
            text,
        )
    }

    #[test]
    fn apply_changes_single_block_edit_keeps_parse_count_bounded() {
        use std::fmt::Write as _;
        // Build a 500-tune tunebook with blank-line separators.
        let mut source = String::new();
        for i in 1..=500 {
            let _ = write!(source, "X:{i}\nT:Tune {i}\nK:C\nC |\n\n");
        }
        let mut model = DocumentModel::full(
            source.clone(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert_eq!(model.blocks.len(), 500);

        // Edit a single tune's body line in place. The expected parse
        // budget is one regional parse that covers the touched block
        // and, at most, the cascade neighbour.
        let target_block = 10;
        let block_text = &source[model.blocks[target_block].lines.clone()];
        let body_rel = block_text.rfind("C |").expect("body line");
        let body_offset = model.blocks[target_block].lines.start + body_rel;
        let new_text = format!(
            "{}{}{}",
            &source[..body_offset],
            "CDEF |",
            &source[body_offset + "C |".len()..]
        );
        let range = model
            .index
            .lsp_range(
                body_offset..body_offset + "C |".len(),
                &PositionEncodingKind::UTF16,
            )
            .unwrap();
        let edit = encode_change(Some(range), "CDEF |");

        let next = DocumentModel::apply_changes(&model, std::slice::from_ref(&edit), new_text)
            .expect("single-block edit must succeed");
        model = next;

        assert!(
            model.parse_count <= 2,
            "single-block edit re-parsed {} blocks, expected <= 2 (touched + cascade)",
            model.parse_count
        );
        let _ = target_block;
    }

    #[test]
    fn apply_changes_range_none_requests_full_rebuild() {
        let source = "X:1\nK:C\nCDEF |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        let result = DocumentModel::apply_changes(
            &model,
            &[encode_change(None, "X:2\nK:D\nG |\n")],
            "X:2\nK:D\nG |\n".to_owned(),
        );
        assert_eq!(result.err(), Some(EditError::FullRebuildRequired));
    }

    #[test]
    fn apply_changes_line_one_edit_requests_full_rebuild() {
        let source = "X:1\nK:C\nCDEF |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        // Touch the version marker on line 1 (insert "%abc-2.1" at byte 0).
        let result = DocumentModel::apply_changes(
            &model,
            &[change_at_line(0, 0, "%abc-2.1\n")],
            "%abc-2.1\nX:1\nK:C\nCDEF |\n".to_owned(),
        );
        assert_eq!(result.err(), Some(EditError::FullRebuildRequired));
    }

    #[test]
    fn apply_changes_bar_duration_invalidates_when_header_changes() {
        let source = "M:4/4\nL:1/8\n\nX:1\nK:C\nCDEF |\n\nX:2\nK:C\nCDEF |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            0,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert!(model.has_tunes());
        let header_blocks = model.blocks[0].lines.end;
        // Replace M:3/4 (header changes; D3) and re-emit the document.
        let new_source = format!(
            "{}M:3/4\n{}",
            &source[..header_blocks],
            &source[header_blocks..]
        );
        let edit = change_at_line(0, 1, "M:3/4\n");

        let next = DocumentModel::apply_changes(&model, std::slice::from_ref(&edit), new_source)
            .expect("header edit must succeed");
        assert_eq!(
            next.bar_duration_count, 2,
            "header M: change must re-run bar duration on every tune"
        );
    }

    #[test]
    fn leading_separator_edit_touches_block_zero_without_full_rebuild() {
        let source = "\n\n\n\n\nX:1\nK:C\nCDEF |\n";
        let model = DocumentModel::full(
            source.to_owned(),
            1,
            PositionEncodingKind::UTF16,
            Config::default(),
        );
        assert_eq!(model.blocks[0].lines.start, 5);

        let edit = TextDocumentContentChangeEvent {
            range: Some(tower_lsp_server::ls_types::Range::new(
                tower_lsp_server::ls_types::Position::new(0, 0),
                tower_lsp_server::ls_types::Position::new(0, 0),
            )),
            range_length: None,
            text: "\n".to_owned(),
        };
        let new_text = format!("\n{source}");
        let result = DocumentModel::apply_changes(&model, std::slice::from_ref(&edit), new_text);
        assert!(result.is_ok(), "leading edit must not require full rebuild");
        let updated = result.unwrap();
        assert!(updated.parse_count <= 1, "parse_count should be <= 1");
    }
}
