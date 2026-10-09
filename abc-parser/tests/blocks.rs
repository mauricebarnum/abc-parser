//! Acceptance tests for the block-scoped parse API added in
//! docs/lsp/incremental-language-server.md. Verifies that [`parse_blocks`]
//! produces diagnostics byte-identical to the corresponding slice of
//! [`parse`] when the grammar, options, and seeding context match, and that
//! [`version_marker`] matches the parser's marker predicate.

use abc_parser::BlocksContext;
use abc_parser::ErrorKind;
use abc_parser::ParserOptions;
use abc_parser::parse;
use abc_parser::parse_blocks;
use abc_parser::version_marker;
use chumsky::span::SimpleSpan;

#[test]
fn parse_blocks_full_document_matches_parse() {
    // `parse_blocks` with `at_document_start = true` and `previous_field_led
    // = None` must reproduce `parse` on the same document, including
    // header resolution and strict-mode diagnostics.
    let source = "X:1\nT:Title\nK:C\nCDEF |\n\nX:2\nT:Second\nK:G\nGABc |\n";
    let full = parse(source);
    let report = parse_blocks(
        source,
        ParserOptions::default(),
        BlocksContext {
            at_document_start: true,
            previous_field_led: None,
        },
    );
    assert!(report.is_valid(), "{:#?}", report.errors);
    let blocks = report.output.expect("blocks parsed");
    assert_eq!(blocks.len(), 2, "two tunes, no header");
    let first_tune = &blocks[0];
    assert_eq!(first_tune.header.len(), 0);
    assert_eq!(first_tune.items.len(), 1);
    assert!(matches!(
        first_tune.items[0].value,
        abc_parser::DocumentItem::Tune(_)
    ));
    assert_eq!(first_tune.first_field.map(|s| s.start), Some(0));
    let second_tune = &blocks[1];
    assert_eq!(second_tune.items.len(), 1);
    assert!(matches!(
        second_tune.items[0].value,
        abc_parser::DocumentItem::Tune(_)
    ));
    // X:2 starts at the first byte of the second tune (after the blank line).
    assert_eq!(second_tune.first_field.map(|s| s.start), Some(24));
    // The block content starts at the first field letter.
    assert_eq!(second_tune.span.start, 24);

    // Diagnostics must be the same set.
    let full_kinds: Vec<_> = full.errors.iter().map(|e| e.kind).collect();
    let blocks_kinds: Vec<_> = report.errors.iter().map(|e| e.kind).collect();
    assert_eq!(full_kinds, blocks_kinds);
    let full_warn_kinds: Vec<_> = full
        .warnings
        .iter()
        .map(|w| (w.kind, w.message.clone()))
        .collect();
    let blocks_warn_kinds: Vec<_> = report
        .warnings
        .iter()
        .map(|w| (w.kind, w.message.clone()))
        .collect();
    assert_eq!(full_warn_kinds, blocks_warn_kinds);
}

#[test]
fn parse_blocks_version_marker_force_strict() {
    // `%abc-2.1` (followed by `\n`) forces strict interpretation for
    // the rest of the document, matching the document parser's behaviour.
    let source = "%abc-2.1\nX:1\nT:Title\nK:C\nCDEF |\n";
    let report = parse_blocks(
        source,
        ParserOptions::default(),
        BlocksContext {
            at_document_start: true,
            previous_field_led: None,
        },
    );
    let _ = report.output.expect("blocks parsed");
    assert!(
        report.errors.is_empty(),
        "strict mode should accept a well-ordered header; got {:?}",
        report.errors
    );
    // X out of order should emit a warning under strict mode.
    let source2 = "%abc-2.1\nT:Title\nX:1\nK:C\nCDEF |\n";
    let report2 = parse_blocks(
        source2,
        ParserOptions::default(),
        BlocksContext {
            at_document_start: true,
            previous_field_led: None,
        },
    );
    assert!(
        report2
            .warnings
            .iter()
            .any(|w| matches!(w.kind, ErrorKind::InvalidFieldOrder)),
        "strict mode should warn when X is not first; got {:?}",
        report2.warnings
    );
}

#[test]
fn parse_blocks_middle_region_uses_previous_field_led() {
    // Parsing a middle region with `previous_field_led = Some(X: span)` and
    // a fieldless block whose deciding line is valid music should produce
    // the extra-blank-line hint whose related span equals the supplied
    // previous-field-led span.
    // Parent source was: "X:1\nK:C\nCDEF |\n\nCDEF |\n";
    // The region is the orphan music block, starting at the second "CDEF".
    // In the *parent* document the previous field-led block was X:1 at
    // byte offset 0..2. In the *slice* (the orphan block) the relative
    // offset of "X:1" is 0..2 because the slice has its own coordinate
    // space.
    let region = "CDEF |\n";
    let report = parse_blocks(
        region,
        ParserOptions::default(),
        BlocksContext {
            at_document_start: false,
            previous_field_led: Some(SimpleSpan::from(0..2)),
        },
    );
    let block = &report.output.expect("blocks parsed")[0];
    let warning = report
        .warnings
        .iter()
        .find(|w| matches!(w.kind, ErrorKind::MissingReference))
        .expect("missing-reference warning");
    assert_eq!(
        warning.message,
        "block parses as music but has no leading information field; a \
         preceding information field block may have been separated from this \
         music by an extra blank line; treating it as free text"
    );
    let related = warning.related.first().expect("related span");
    assert_eq!(related.span.start, 0);
    assert_eq!(related.span.end, 2);
    assert!(block.first_field.is_none());
    assert!(block.header.is_empty());
    assert_eq!(block.span.start, 0);
}

#[test]
fn parse_blocks_equivalent_to_parse_for_corpus() {
    // For each fixture, `parse_blocks(source, ..., at_document_start=true)`
    // must yield the same item count and diagnostics as `parse(source)`.
    let fixtures: &[(&str, &str)] = &[
        (
            "two tunes",
            "X:1\nT:A\nK:C\nCDEF |\n\nX:2\nT:B\nK:G\nGABc |\n",
        ),
        ("header + tune", "M:4/4\nL:1/8\n\nX:1\nK:C\nCDEF |\n"),
        (
            "tune with music warning",
            "X:1\nT:A\nK:C\nCDEF |\n\nCDEF |\n",
        ),
        (
            "free text between tunes",
            "X:1\nT:A\nK:C\nCDEF |\n\nFree text\n\nX:2\nT:B\nK:G\nGABc |\n",
        ),
        (
            "leading comment + tune",
            "% a leading comment\nX:1\nT:A\nK:C\nCDEF |\n",
        ),
    ];
    for (name, source) in fixtures.iter().copied() {
        let full = parse(source);
        let full_items = &full.output.as_ref().unwrap().items;
        let report = parse_blocks(
            source,
            ParserOptions::default(),
            BlocksContext {
                at_document_start: true,
                previous_field_led: None,
            },
        );
        let blocks = report.output.as_ref().expect("blocks parsed");
        let block_item_count: usize = blocks.iter().map(|b| b.items.len() + b.header.len()).sum();
        let full_item_count = full_items.len() + full.output.as_ref().unwrap().header.len();
        assert_eq!(
            block_item_count, full_item_count,
            "{name}: items count mismatch"
        );
        let full_err_kinds: Vec<_> = full.errors.iter().map(|e| e.kind).collect();
        let blocks_err_kinds: Vec<_> = report.errors.iter().map(|e| e.kind).collect();
        assert_eq!(
            full_err_kinds, blocks_err_kinds,
            "{name}: error kinds mismatch"
        );
        let full_warn_kinds: Vec<_> = full.warnings.iter().map(|w| w.kind).collect();
        let blocks_warn_kinds: Vec<_> = report.warnings.iter().map(|w| w.kind).collect();
        assert_eq!(
            full_warn_kinds, blocks_warn_kinds,
            "{name}: warning kinds mismatch"
        );
    }
}

#[test]
fn version_marker_vectors_match_parser() {
    // Each vector mirrors a documented behaviour of `strict_version_marker`.
    let cases: &[(&str, Option<bool>)] = &[
        ("%abc-2.1\n", Some(true)),
        ("%abc-2.1x\n", Some(true)),
        ("%abc-3.0\n", Some(true)),
        ("%abc-3\n", Some(true)),
        ("%abc-2.0\n", None),
        ("%abc-2\n", None),
        ("%abc-2x\n", None),
        ("%abc\n", None),
        ("%abc-\n", None),
        ("\n", None),
    ];
    for (input, expected) in cases {
        assert_eq!(
            version_marker(input),
            *expected,
            "version_marker({input:?})"
        );
    }
}

#[test]
fn parse_blocks_bom_spans_are_indexed_from_document_start() {
    let source = "\u{feff}X:1\nK:C\nC |\n";
    let report = parse_blocks(
        source,
        ParserOptions::default(),
        BlocksContext {
            at_document_start: true,
            previous_field_led: None,
        },
    );
    assert!(report.is_valid(), "{:#?}", report.errors);
    let blocks = report.output.expect("blocks parsed");
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    assert_eq!(block.span.start, 3);
    assert_eq!(block.first_field.map(|s| (s.start, s.end)), Some((3, 6)));
}
