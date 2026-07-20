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

//! Validates ABC 2.1 input and optionally emits deterministic fixes.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use abc_parser::Decoration;
use abc_parser::DiagnosticRenderer;
use abc_parser::Document;
use abc_parser::ErrorKind;
use abc_parser::Field;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::Fraction;
use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::Meter;
use abc_parser::MusicElement;
use abc_parser::ParseWarning;
use abc_parser::ParserOptions;
use abc_parser::Tempo;
use abc_parser::ToAbc;
use abc_parser::Tune;
use abc_parser::parse_with_options;
use chumsky::span::SimpleSpan;
use clap::Parser;

const EIGHTH_NOTE: Fraction = Fraction {
    numerator: 1,
    denominator: 8,
};
const SIXTEENTH_NOTE: Fraction = Fraction {
    numerator: 1,
    denominator: 16,
};

/// Command-line arguments for validating and fixing an ABC document.
#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "abc-lint",
    about = "Validate an ABC 2.1 document",
    long_about = "Validate an ABC 2.1 document and optionally emit deterministic fixes."
)]
struct Arguments {
    /// ABC input file, or - to read standard input.
    input: PathBuf,
    /// Emit a canonical document with unambiguous fixes applied.
    #[arg(long)]
    fix: bool,
    /// Write fixed output to this file instead of standard output.
    #[arg(long, value_name = "FILE", requires = "fix")]
    out: Option<PathBuf>,
}

/// Active unit-note-length state while traversing one tune.
#[derive(Clone, Copy)]
struct UnitLengthState {
    value: Fraction,
    explicit: bool,
    body_started: bool,
}

/// One position in a tune header while its field blocks are reordered.
enum HeaderToken<S> {
    FieldSlot,
    Other(abc_parser::Spanned<Line<S, String>, S>),
}

/// One field and any physical continuation lines that belong to it.
struct HeaderFieldBlock<S> {
    rank: u8,
    lines: Vec<abc_parser::Spanned<Line<S, String>, S>>,
}

impl Default for UnitLengthState {
    fn default() -> Self {
        Self {
            value: EIGHTH_NOTE,
            explicit: false,
            body_started: false,
        }
    }
}

/// Parses arguments, validates the input, and optionally writes fixed ABC.
fn main() -> ExitCode {
    match run(&Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("abc-lint: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the lint command with fully parsed arguments.
fn run(arguments: &Arguments) -> Result<(), String> {
    let source = read_source(&arguments.input)?;
    let input_name = input_name(&arguments.input);
    let parsed = parse_with_options(source.as_str(), ParserOptions::new().strict(true));
    let mut owned_document = parsed
        .output
        .clone()
        .map(|document| document.into_owned(source.as_str()))
        .transpose()
        .map_err(|error| format!("could not resolve parsed source text: {error}"))?;
    let mut diagnostic_renderer = DiagnosticRenderer::new(source.as_str());
    let order_warnings = owned_document
        .as_ref()
        .map(recommended_order_warnings)
        .unwrap_or_default();
    let fixable_warnings = owned_document
        .as_ref()
        .map(|document| fixable_syntax_warnings(document, source.as_str()))
        .unwrap_or_default();
    for warning in parsed
        .warnings
        .iter()
        .chain(&order_warnings)
        .chain(&fixable_warnings)
    {
        eprintln!(
            "abc-lint: warning: {input_name}:{}",
            diagnostic_renderer.render_warning(warning)
        );
    }
    let has_unfixable_errors = parsed
        .errors
        .iter()
        .any(|error| !arguments.fix || error.kind != ErrorKind::MissingReference);
    if has_unfixable_errors {
        let diagnostics = parsed
            .errors
            .iter()
            .map(|error| format!("{input_name}:{}", diagnostic_renderer.render_error(error)))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("input is not valid ABC 2.1:\n{diagnostics}"));
    }
    if !arguments.fix {
        return Ok(());
    }

    let mut document = owned_document
        .take()
        .ok_or_else(|| "parser did not produce a document".to_owned())?;
    fix_document(&mut document)?;
    let fixed = document.to_abc();
    validate_fixed_output(&fixed)?;
    write_output(fixed, arguments.out.as_deref())
}

/// Reports every deterministic source change performed by the fixer.
fn fixable_syntax_warnings(
    document: &Document<SimpleSpan<usize>, String>,
    source: &str,
) -> Vec<ParseWarning<SimpleSpan<usize>>> {
    let mut warnings = Vec::new();
    collect_line_fix_warnings(&document.header, document.header.len(), &mut warnings);
    for tune in document.tunes() {
        if !tune.lines.iter().any(|line| {
            matches!(
                line.value,
                Line::Field(Field {
                    kind: FieldKind::Reference,
                    ..
                })
            )
        }) && let Some(line) = tune.lines.first()
        {
            warnings.push(ParseWarning {
                kind: ErrorKind::FixableSyntax,
                message: "missing X: field will be assigned a unique reference by --fix".to_owned(),
                span: line.span,
            });
        }
        let header_end = tune
            .lines
            .iter()
            .position(|line| matches!(line.value, Line::Music(_)))
            .unwrap_or(tune.lines.len());
        collect_line_fix_warnings(&tune.lines, header_end, &mut warnings);
        warnings.extend(interspersed_continuation_warnings(
            &tune.lines[..header_end],
        ));
        for line in &tune.lines {
            if let Line::Music(elements) = &line.value {
                for element in elements {
                    match &element.value {
                        MusicElement::InlineField(Field { key: 'H', .. }) => {
                            warnings.push(ParseWarning {
                                kind: ErrorKind::FixableSyntax,
                                message: "inline H: field will be removed by --fix".to_owned(),
                                span: element.span,
                            });
                        }
                        MusicElement::Decoration(Decoration {
                            legacy_delimiter: true,
                            ..
                        }) => {
                            warnings.push(ParseWarning {
                                kind: ErrorKind::DeprecatedSyntax,
                                message: "deprecated +name+ decoration will be rewritten as !name!"
                                    .to_owned(),
                                span: element.span,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let mut canonical = document.to_abc();
    if !canonical.ends_with('\n') {
        canonical.push('\n');
    }
    if let Some(warning) = canonical_form_warning(source, &canonical) {
        warnings.push(warning);
    }
    warnings
}

/// Locates and describes the first character changed by canonical emission.
fn canonical_form_warning(
    source: &str,
    canonical: &str,
) -> Option<ParseWarning<SimpleSpan<usize>>> {
    let mut source_characters = source.char_indices();
    let mut canonical_characters = canonical.char_indices();
    loop {
        match (source_characters.next(), canonical_characters.next()) {
            (Some((_, found)), Some((_, expected))) if found == expected => {}
            (Some((start, found)), Some((_, expected))) => {
                return Some(ParseWarning {
                    kind: ErrorKind::FixableSyntax,
                    message: format!(
                        "document is not in canonical form; first difference: --fix expects {expected:?}, found {found:?}"
                    ),
                    span: (start..start + found.len_utf8()).into(),
                });
            }
            (Some((start, found)), None) => {
                return Some(ParseWarning {
                    kind: ErrorKind::FixableSyntax,
                    message: format!(
                        "document is not in canonical form; first difference: --fix expects end of input, found {found:?}"
                    ),
                    span: (start..start + found.len_utf8()).into(),
                });
            }
            (None, Some((_, expected))) => {
                return Some(ParseWarning {
                    kind: ErrorKind::FixableSyntax,
                    message: format!(
                        "document is not in canonical form; first difference: --fix expects {expected:?}, found end of input"
                    ),
                    span: (source.len()..source.len()).into(),
                });
            }
            (None, None) => return None,
        }
    }
}

/// Reports continuations that reordering will reunite with their field.
fn interspersed_continuation_warnings<S>(
    lines: &[abc_parser::Spanned<Line<S, String>, S>],
) -> Vec<ParseWarning<S>>
where
    S: Clone,
{
    let mut warnings = Vec::new();
    let mut has_field = false;
    let mut has_intervening_line = false;
    for line in lines {
        match &line.value {
            Line::Field(_) => {
                has_field = true;
                has_intervening_line = false;
            }
            Line::FieldContinuation(_) if has_field => {
                if has_intervening_line {
                    warnings.push(ParseWarning {
                        kind: ErrorKind::FixableSyntax,
                        message: "field continuation will move next to its field during --fix"
                            .to_owned(),
                        span: line.span.clone(),
                    });
                }
                has_intervening_line = false;
            }
            Line::Comment(_) | Line::Directive(_) if has_field => {
                has_intervening_line = true;
            }
            _ => {
                has_field = false;
                has_intervening_line = false;
            }
        }
    }
    warnings
}

/// Collects fixer diagnostics for one physical-line region.
fn collect_line_fix_warnings<S>(
    lines: &[abc_parser::Spanned<Line<S, String>, S>],
    field_end: usize,
    warnings: &mut Vec<ParseWarning<S>>,
) where
    S: Clone,
{
    for line in lines {
        if let Line::Field(field) = &line.value {
            match field {
                Field { key: 'H', .. } => warnings.push(ParseWarning {
                    kind: ErrorKind::FixableSyntax,
                    message: "H: field will be removed by --fix".to_owned(),
                    span: line.span.clone(),
                }),
                Field {
                    kind: FieldKind::Reference,
                    value: FieldValue::Empty,
                    ..
                } => warnings.push(ParseWarning {
                    kind: ErrorKind::FixableSyntax,
                    message: "empty X: field will be assigned a unique reference by --fix"
                        .to_owned(),
                    span: line.span.clone(),
                }),
                _ => {}
            }
        }
    }
    warnings.extend(redundant_field_warnings(&lines[..field_end]));
}

/// Reports earlier instruction fields made useless by a later definition.
fn redundant_field_warnings<S>(
    lines: &[abc_parser::Spanned<Line<S, String>, S>],
) -> Vec<ParseWarning<S>>
where
    S: Clone,
{
    let mut seen = BTreeSet::new();
    let mut warnings = Vec::new();
    for line in lines.iter().rev() {
        let Line::Field(field) = &line.value else {
            continue;
        };
        let Some(identity) = overriding_field_identity(field) else {
            continue;
        };
        if !seen.insert(identity) {
            warnings.push(ParseWarning {
                kind: ErrorKind::FixableSyntax,
                message: format!(
                    "earlier {}: field is overridden by a later definition and will be removed by --fix",
                    field.key
                ),
                span: line.span.clone(),
            });
        }
    }
    warnings.reverse();
    warnings
}

/// Identifies the effective setting assigned by an instruction field.
fn overriding_field_identity(field: &Field<String>) -> Option<String> {
    match (&field.value, field.key) {
        (_, 'K' | 'L' | 'M' | 'P' | 'Q') => Some(field.key.to_string()),
        (FieldValue::Text(text), 'I') => text
            .split_whitespace()
            .next()
            .map(|name| format!("I:{name}")),
        (FieldValue::Voice(voice), 'V') => Some(format!("V:{}", voice.id)),
        (FieldValue::UserSymbol(symbol), 'U') => Some(format!("U:{}", symbol.symbol)),
        (FieldValue::Macro(definition), 'm') => Some(format!("m:{}", definition.pattern.trim())),
        _ => None,
    }
}

/// Reports tune-header fields that depart from the recommended stable order.
fn recommended_order_warnings<S, T>(document: &Document<S, T>) -> Vec<ParseWarning<S>>
where
    S: Clone,
{
    let mut warnings = Vec::new();
    for tune in document.tunes() {
        let mut preceding_fields = Vec::new();
        for line in &tune.lines {
            let Line::Field(field) = &line.value else {
                if matches!(line.value, Line::Music(_)) {
                    break;
                }
                continue;
            };
            let rank = header_field_rank(field.key);
            if field.key != 'X'
                && let Some((_, preceding_key)) = preceding_fields
                    .iter()
                    .find(|(preceding_rank, _)| *preceding_rank > rank)
            {
                warnings.push(ParseWarning {
                    kind: ErrorKind::InvalidFieldOrder,
                    message: format!(
                        "{}: field should appear before {preceding_key}: field in the recommended tune-header order",
                        field.key,
                    ),
                    span: line.span.clone(),
                });
            }
            preceding_fields.push((rank, field.key));
        }
    }
    warnings
}

/// Returns a display name for diagnostics concerning one input path.
fn input_name(path: &Path) -> String {
    if path.as_os_str() == "-" {
        "<stdin>".to_owned()
    } else {
        path.display().to_string()
    }
}

/// Reads the named file, or standard input when the path is `-`.
fn read_source(path: &Path) -> Result<String, String> {
    if path.as_os_str() == "-" {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| format!("could not read standard input: {error}"))?;
        Ok(source)
    } else {
        fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
    }
}

/// Writes fixed ABC to the selected file or standard output.
fn write_output(mut source: String, path: Option<&Path>) -> Result<(), String> {
    if !source.ends_with('\n') {
        source.push('\n');
    }
    if let Some(path) = path {
        fs::write(path, &source)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    } else {
        io::stdout()
            .write_all(source.as_bytes())
            .map_err(|error| format!("could not write standard output: {error}"))
    }
}

/// Applies deterministic fixes throughout an owned document.
fn fix_document<S>(document: &mut Document<S, String>) -> Result<(), String>
where
    S: Clone,
{
    let mut global_state = UnitLengthState::default();
    fix_lines(&mut document.header, &mut global_state)?;
    let header_len = document.header.len();
    remove_overridden_fields(&mut document.header, header_len);
    let mut used_references = document
        .tunes()
        .flat_map(|tune| &tune.lines)
        .filter_map(|line| match &line.value {
            Line::Field(Field {
                value: FieldValue::Reference(value),
                ..
            }) => Some(*value),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut next_reference = 1_u64;
    for tune in document.tunes_mut() {
        let mut tune_state = global_state;
        tune_state.body_started = false;
        let state_at_music = fix_lines(&mut tune.lines, &mut tune_state)?;
        preserve_unit_length_across_early_key(tune, state_at_music);
        let header_end = tune
            .lines
            .iter()
            .position(|line| matches!(line.value, Line::Music(_)))
            .unwrap_or(tune.lines.len());
        remove_overridden_fields(&mut tune.lines, header_end);
        ensure_reference(tune, &mut used_references, &mut next_reference)?;
        reorder_tune_header(tune);
    }
    Ok(())
}

/// Removes header instruction fields superseded by later definitions.
fn remove_overridden_fields<S>(
    lines: &mut Vec<abc_parser::Spanned<Line<S, String>, S>>,
    field_end: usize,
) {
    let mut seen = BTreeSet::new();
    let mut redundant = BTreeSet::new();
    for (index, line) in lines[..field_end].iter().enumerate().rev() {
        let Line::Field(field) = &line.value else {
            continue;
        };
        let Some(identity) = overriding_field_identity(field) else {
            continue;
        };
        if !seen.insert(identity) {
            redundant.insert(index);
        }
    }

    let mut retained = Vec::with_capacity(lines.len() - redundant.len());
    let mut discard_continuations = false;
    for (index, line) in std::mem::take(lines).into_iter().enumerate() {
        match &line.value {
            Line::Field(_) => discard_continuations = redundant.contains(&index),
            Line::FieldContinuation(_) if discard_continuations => continue,
            Line::FieldContinuation(_) | Line::Comment(_) | Line::Directive(_) => {}
            _ => discard_continuations = false,
        }
        if !redundant.contains(&index) {
            retained.push(line);
        }
    }
    *lines = retained;
}

/// Fixes and filters physical lines while maintaining unit-length state.
fn fix_lines<S>(
    lines: &mut Vec<abc_parser::Spanned<Line<S, String>, S>>,
    state: &mut UnitLengthState,
) -> Result<UnitLengthState, String> {
    let mut fixed = Vec::with_capacity(lines.len());
    let mut discard_field_continuations = false;
    let mut state_at_music = None;
    for mut line in std::mem::take(lines) {
        match &line.value {
            Line::Field(field) => {
                discard_field_continuations = matches!(field.key, 'E' | 'H');
            }
            Line::FieldContinuation(_) if discard_field_continuations => continue,
            Line::FieldContinuation(_)
            | Line::Comment(_)
            | Line::Directive(_)
            | Line::DeprecatedHistoryContinuation(_) => {}
            _ => discard_field_continuations = false,
        }
        if state_at_music.is_none() && matches!(line.value, Line::Music(_)) {
            state_at_music = Some(*state);
        }
        if fix_line(&mut line.value, state)? {
            fixed.push(line);
        }
    }
    *lines = fixed;
    Ok(state_at_music.unwrap_or(*state))
}

/// Supplies an empty or absent tune reference with a unique increasing value.
fn ensure_reference<S>(
    tune: &mut Tune<S, String>,
    used: &mut BTreeSet<u32>,
    next: &mut u64,
) -> Result<(), String>
where
    S: Clone,
{
    let reference_index = tune.lines.iter().position(|line| {
        matches!(
            line.value,
            Line::Field(Field {
                kind: FieldKind::Reference,
                ..
            })
        )
    });
    if let Some(index) = reference_index {
        if let Line::Field(field) = &mut tune.lines[index].value {
            if let FieldValue::Reference(value) = field.value {
                *next = (*next).max(u64::from(value) + 1);
            } else {
                field.value = FieldValue::Reference(allocate_reference(used, next)?);
            }
        }
        let music_index = tune
            .lines
            .iter()
            .position(|line| matches!(line.value, Line::Music(_)));
        if music_index.is_some_and(|music_index| index > music_index) {
            let reference = tune.lines.remove(index);
            tune.lines.insert(0, reference);
        }
        return Ok(());
    }

    let value = allocate_reference(used, next)?;
    let span = tune
        .lines
        .first()
        .ok_or_else(|| "could not assign X: to an empty tune".to_owned())?
        .span
        .clone();
    tune.lines.insert(
        0,
        abc_parser::Spanned {
            value: Line::Field(Field {
                key: 'X',
                kind: FieldKind::Reference,
                value: FieldValue::Reference(value),
            }),
            span,
        },
    );
    Ok(())
}

/// Allocates the next unused reference number without exceeding `u32`.
fn allocate_reference(used: &mut BTreeSet<u32>, next: &mut u64) -> Result<u32, String> {
    loop {
        let candidate = u32::try_from(*next)
            .map_err(|_| "could not assign another increasing X: reference number".to_owned())?;
        *next += 1;
        if used.insert(candidate) {
            return Ok(candidate);
        }
    }
}

/// Reorders tune-header fields while leaving the music body in source order.
fn reorder_tune_header<S>(tune: &mut Tune<S, String>)
where
    S: Clone,
{
    let header_end = tune
        .lines
        .iter()
        .position(|line| matches!(line.value, Line::Music(_)))
        .unwrap_or(tune.lines.len());
    let header = tune.lines.drain(..header_end).collect::<Vec<_>>();
    let mut tokens = Vec::with_capacity(header.len());
    let mut fields = Vec::<HeaderFieldBlock<S>>::new();
    let mut last_field = None;

    for line in header {
        match &line.value {
            Line::Field(field) => {
                last_field = Some(fields.len());
                fields.push(HeaderFieldBlock {
                    rank: header_field_rank(field.key),
                    lines: vec![line],
                });
                tokens.push(HeaderToken::FieldSlot);
            }
            Line::FieldContinuation(_) => {
                if let Some(index) = last_field {
                    fields[index].lines.push(line);
                } else {
                    tokens.push(HeaderToken::Other(line));
                }
            }
            Line::Comment(_) | Line::Directive(_) => tokens.push(HeaderToken::Other(line)),
            _ => {
                last_field = None;
                tokens.push(HeaderToken::Other(line));
            }
        }
    }

    fields.sort_by_key(|field| field.rank);
    let mut fields = fields.into_iter();
    let mut reordered = Vec::with_capacity(header_end);
    for token in tokens {
        match token {
            HeaderToken::FieldSlot => {
                reordered.extend(
                    fields
                        .next()
                        .expect("every header field has exactly one field slot")
                        .lines,
                );
            }
            HeaderToken::Other(line) => reordered.push(line),
        }
    }
    reordered.append(&mut tune.lines);
    tune.lines = reordered;
}

/// Preserves the unit length when an early key made a later meter body-scoped.
fn preserve_unit_length_across_early_key<S>(
    tune: &mut Tune<S, String>,
    state_at_music: UnitLengthState,
) where
    S: Clone,
{
    let header_end = tune
        .lines
        .iter()
        .position(|line| matches!(line.value, Line::Music(_)))
        .unwrap_or(tune.lines.len());
    let key_index = tune.lines[..header_end].iter().position(|line| {
        matches!(
            line.value,
            Line::Field(Field {
                kind: FieldKind::Key,
                ..
            })
        )
    });
    let Some(key_index) = key_index else {
        return;
    };
    let meter_crosses_key = tune.lines[key_index + 1..header_end].iter().any(|line| {
        matches!(
            line.value,
            Line::Field(Field {
                kind: FieldKind::Meter,
                ..
            })
        )
    });
    if !meter_crosses_key || state_at_music.explicit {
        return;
    }

    let span = tune.lines[key_index].span.clone();
    tune.lines.insert(
        header_end,
        abc_parser::Spanned {
            value: Line::Field(Field {
                key: 'L',
                kind: FieldKind::UnitLength,
                value: FieldValue::UnitLength(state_at_music.value),
            }),
            span,
        },
    );
}

/// Returns the stable, recommended sort rank for one tune-header field.
const fn header_field_rank(key: char) -> u8 {
    match key {
        'X' => 0,
        'T' => 1,
        'C' => 2,
        'A' | 'O' => 3,
        'R' => 4,
        'B' => 5,
        'D' => 6,
        'F' => 7,
        'G' => 8,
        'H' => 9,
        'N' => 10,
        'S' => 11,
        'Z' => 12,
        'I' => 14,
        'm' => 15,
        'U' => 16,
        'P' => 17,
        'V' => 18,
        'M' => 19,
        'L' => 20,
        'Q' => 21,
        'W' => 22,
        'K' => u8::MAX,
        _ => 13,
    }
}

/// Applies fixes to one physical line and returns whether to retain it.
fn fix_line<S>(line: &mut Line<S, String>, state: &mut UnitLengthState) -> Result<bool, String> {
    match line {
        Line::Field(field) => fix_field(field, state),
        Line::DeprecatedHistoryContinuation(_) => Ok(false),
        Line::Music(elements) => {
            state.body_started = true;
            let mut fixed = Vec::with_capacity(elements.len());
            for mut element in std::mem::take(elements) {
                if fix_music_element(&mut element.value, state)? {
                    fixed.push(element);
                }
            }
            *elements = fixed;
            Ok(!elements.is_empty())
        }
        _ => Ok(true),
    }
}

/// Applies fixes to one music element and returns whether to retain it.
fn fix_music_element(
    element: &mut MusicElement<String>,
    state: &mut UnitLengthState,
) -> Result<bool, String> {
    match element {
        MusicElement::InlineField(field) => fix_field(field, state),
        MusicElement::Decoration(Decoration {
            legacy_delimiter, ..
        }) => {
            *legacy_delimiter = false;
            Ok(true)
        }
        _ => Ok(true),
    }
}

/// Applies fixes to one field and returns whether to retain it.
fn fix_field(field: &mut Field<String>, state: &mut UnitLengthState) -> Result<bool, String> {
    match field.key {
        'A' => {
            field.key = 'O';
            field.kind = FieldKind::Origin;
        }
        'E' | 'H' => return Ok(false),
        _ => {}
    }

    match &mut field.value {
        FieldValue::UnitLength(value) => {
            state.value = *value;
            state.explicit = true;
        }
        FieldValue::Meter(meter) if !state.body_started && !state.explicit => {
            state.value = default_unit_length(meter);
        }
        FieldValue::Tempo(tempo) => fix_tempo(tempo, state.value)?,
        _ => {}
    }
    if field.key == 'K' {
        state.body_started = true;
    }
    Ok(true)
}

/// Rewrites a deprecated tempo using the active unit note length.
fn fix_tempo(tempo: &mut Tempo<String>, unit_length: Fraction) -> Result<(), String> {
    let replacement = match tempo {
        Tempo::Deprecated(raw) => Some(Tempo::MetronomeMark {
            prelude: None,
            beats: vec![unit_length],
            bpm: deprecated_tempo_bpm(raw)?,
            postlude: None,
        }),
        _ => None,
    };
    if let Some(replacement) = replacement {
        *tempo = replacement;
    }
    Ok(())
}

/// Extracts the uninterpreted BPM digits from a recognized deprecated tempo.
fn deprecated_tempo_bpm(raw: &str) -> Result<u32, String> {
    let raw = raw.trim();
    let digits = if let Some(after_c) = raw.strip_prefix('C') {
        after_c
            .trim_start()
            .strip_prefix('=')
            .map(str::trim)
            .ok_or_else(|| format!("could not resolve deprecated tempo {raw:?}"))?
    } else {
        raw
    };
    digits
        .parse()
        .map_err(|error| format!("could not resolve deprecated tempo {raw:?}: {error}"))
}

/// Computes the default unit note length selected by a meter.
fn default_unit_length(meter: &Meter) -> Fraction {
    let below_three_quarters = match meter {
        Meter::Simple(fraction) => fraction_is_below_three_quarters(*fraction),
        Meter::Compound {
            groups,
            denominator,
        } => {
            let numerator = groups.iter().map(|value| u64::from(*value)).sum::<u64>();
            numerator * 4 < u64::from(*denominator) * 3
        }
        Meter::Common | Meter::Cut | Meter::None => false,
    };
    if below_three_quarters {
        SIXTEENTH_NOTE
    } else {
        EIGHTH_NOTE
    }
}

/// Returns whether a rational meter is less than three quarters.
const fn fraction_is_below_three_quarters(fraction: Fraction) -> bool {
    (fraction.numerator as u64) * 4 < (fraction.denominator as u64) * 3
}

/// Ensures the fixed document remains valid ABC 2.1.
fn validate_fixed_output(source: &str) -> Result<(), String> {
    let report = parse_with_options(source, ParserOptions::new().strict(true));
    if report.is_valid() {
        return Ok(());
    }
    let mut renderer = DiagnosticRenderer::new(source);
    let diagnostics = report
        .errors
        .iter()
        .map(|error| renderer.render_error(error))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "fixed output unexpectedly failed validation:\n{diagnostics}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_selects_the_standard_default_unit_length() {
        assert_eq!(default_unit_length(&Meter::None), EIGHTH_NOTE);
        assert_eq!(default_unit_length(&Meter::Common), EIGHTH_NOTE);
        assert_eq!(
            default_unit_length(&Meter::Simple(Fraction {
                numerator: 2,
                denominator: 4,
            })),
            SIXTEENTH_NOTE
        );
        assert_eq!(
            default_unit_length(&Meter::Simple(Fraction {
                numerator: 3,
                denominator: 4,
            })),
            EIGHTH_NOTE
        );
        assert_eq!(
            default_unit_length(&Meter::Compound {
                groups: vec![2, 3, 2],
                denominator: 8,
            }),
            EIGHTH_NOTE
        );
    }

    #[test]
    fn deprecated_tempo_digits_are_checked_only_while_fixing() {
        assert_eq!(deprecated_tempo_bpm("120").unwrap(), 120);
        assert_eq!(deprecated_tempo_bpm("C = 120").unwrap(), 120);
        assert!(deprecated_tempo_bpm("999999999999999999999").is_err());
    }
}
