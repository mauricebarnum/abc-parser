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

//! Converts ABC 2.1 music notation to `MusicXML` 4.0.

use std::fs;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use abc_musicxml::AnnotationLevel;
use abc_musicxml::MusicXmlError;
use abc_musicxml::MusicXmlOptions;
use abc_musicxml::SpannedLine;
use abc_musicxml::build_info;
use abc_musicxml::document_to_musicxml;
use abc_musicxml::extract_tune_identity;
use abc_musicxml::generate_opus_xml;
use abc_musicxml::tune_to_musicxml;
use abc_parser::DiagnosticRenderer;
use abc_parser::Document;
use abc_parser::Field;
use abc_parser::FieldKind;
use abc_parser::FieldValue;
use abc_parser::IntoOwnedAst;
use abc_parser::Line;
use abc_parser::OwnedDocument;
use abc_parser::SimpleSpan;
use abc_parser::Tune;
use abc_parser::build_info::input_name;
use abc_parser::build_info::read_source;
use abc_parser::parse;
use clap::Parser;
use clap::ValueEnum;

/// CLI value enum mirroring [`AnnotationLevel`] so library builds remain `clap`-free.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum CliAnnotationLevel {
    /// Emit no ABC source comments.
    #[default]
    None,
    /// Emit Level 1 line comments.
    Line,
    /// Emit Level 1 line comments plus Level 2 span comments.
    Verbose,
}

impl From<CliAnnotationLevel> for AnnotationLevel {
    fn from(value: CliAnnotationLevel) -> Self {
        match value {
            CliAnnotationLevel::None => Self::None,
            CliAnnotationLevel::Line => Self::Line,
            CliAnnotationLevel::Verbose => Self::Verbose,
        }
    }
}

/// Command-line arguments for converting ABC to `MusicXML` 4.0.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, Parser, PartialEq)]
#[command(
    name = "abc-musicxml",
    about = "Convert ABC 2.1 notation to MusicXML 4.0",
    long_about = "Convert ABC 2.1 tunes to MusicXML 4.0 partwise scores with optional ABC source provenance comments."
)]
struct Arguments {
    /// ABC input file, or - to read standard input.
    input: PathBuf,

    /// Select a specific tune by its X: reference number.
    #[arg(
        short = 'x',
        long = "tune",
        value_name = "REF",
        conflicts_with = "out_dir"
    )]
    tune: Option<u32>,

    /// Include ABC source annotations as XML comments (`none`, `line`, or `verbose`).
    #[arg(
        short = 'a',
        long = "annotate",
        value_enum,
        default_value_t = CliAnnotationLevel::None,
        num_args = 0..=1,
        default_missing_value = "line",
        value_name = "LEVEL"
    )]
    annotate: CliAnnotationLevel,

    /// Write output to this file instead of standard output.
    #[arg(short = 'o', long = "out", value_name = "FILE")]
    out: Option<PathBuf>,

    /// Write each tune in a multi-tune ABC file to `<DIR>/<stem>-<X>.musicxml`.
    #[arg(
        long = "out-dir",
        value_name = "DIR",
        conflicts_with = "out",
        conflicts_with = "tune"
    )]
    out_dir: Option<PathBuf>,

    /// Write output as compressed `.mxl` archive.
    #[arg(long = "mxl")]
    mxl: bool,

    /// Omit the <!DOCTYPE score-partwise ...> declaration.
    #[arg(long = "no-doctype")]
    no_doctype: bool,

    /// Attempt best-effort conversion even if the ABC input has syntax errors.
    #[arg(long = "recover")]
    recover: bool,

    /// Print version information (combine with --verbose for detailed build info).
    #[arg(short = 'V', long)]
    version: bool,

    /// Emit verbose output (shows detailed build info when combined with --version).
    #[arg(short = 'v', long)]
    verbose: bool,
}

fn main() -> ExitCode {
    if let Some(exit_code) = build_info::check_version_and_exit("abc-musicxml") {
        return exit_code;
    }

    match run(&Arguments::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("abc-musicxml: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Writes all output bytes to the selected file or standard output.
fn write_output(mut content: String, path: Option<&PathBuf>, mxl: bool) -> Result<(), String> {
    if mxl {
        #[cfg(feature = "mxl")]
        {
            if let Some(path) = path {
                let file = fs::File::create(path)
                    .map_err(|error| format!("could not create {}: {error}", path.display()))?;
                abc_musicxml::create_mxl_seek(file, "score.musicxml", content.as_bytes())
                    .map_err(|error| format!("could not create mxl archive: {error}"))?;
            } else {
                let stdout = io::stdout();
                let mut lock = stdout.lock();
                abc_musicxml::create_mxl(&mut lock, "score.musicxml", content.as_bytes())
                    .map_err(|error| format!("could not create mxl archive: {error}"))?;
            }
            return Ok(());
        }
        #[cfg(not(feature = "mxl"))]
        {
            let _ = path;
            let _ = content;
            return Err("compressed .mxl output requires the 'mxl' cargo feature".to_owned());
        }
    }

    if !content.ends_with('\n') {
        content.push('\n');
    }
    if let Some(path) = path {
        fs::write(path, &content)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    } else {
        io::stdout()
            .write_all(content.as_bytes())
            .map_err(|error| format!("could not write standard output: {error}"))
    }
}

/// Extracts any document-level title from the file header.
fn extract_header_title(doc: &Document<SimpleSpan<usize>, String>) -> Option<&str> {
    doc.header.iter().find_map(|line| {
        if let Line::Field(Field {
            kind: FieldKind::Title,
            value,
            ..
        }) = &line.value
        {
            match value {
                FieldValue::Text(t) | FieldValue::Unparsed(t) => Some(t.trim()),
                _ => None,
            }
        } else {
            None
        }
    })
}

/// Runs the command with fully parsed arguments.
fn run(arguments: &Arguments) -> Result<(), String> {
    let source = read_source(&arguments.input)?;
    let parsed = parse(source.as_str());

    let input_name = input_name(&arguments.input);

    let mut diagnostic_renderer = DiagnosticRenderer::new(source.as_str());
    for warning in &parsed.warnings {
        eprintln!(
            "abc-musicxml: warning: {input_name}:{}",
            diagnostic_renderer.render_warning(warning)
        );
    }

    if !parsed.is_valid() {
        if arguments.recover && parsed.output.is_some() {
            for error in &parsed.errors {
                eprintln!(
                    "abc-musicxml: error: {input_name}:{}",
                    diagnostic_renderer.render_error(error)
                );
            }
        } else {
            let diagnostics = parsed
                .errors
                .iter()
                .map(|error| format!("{input_name}:{}", diagnostic_renderer.render_error(error)))
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!("input is not valid ABC:\n{diagnostics}"));
        }
    }

    let parsed_document = parsed
        .output
        .ok_or_else(|| "parser did not produce a document".to_owned())?;
    let document: OwnedDocument<SimpleSpan<usize>> = parsed_document
        .into_owned(source.as_str())
        .map_err(|error| format!("could not resolve parsed source text: {error}"))?;

    if document.tunes().next().is_none() {
        return Err(MusicXmlError::EmptyDocument.to_string());
    }

    let options = MusicXmlOptions::new()
        .with_source_name(&input_name)
        .with_annotation_level(arguments.annotate.into())
        .with_doctype(!arguments.no_doctype)
        .with_recover(arguments.recover);

    let is_mxl = arguments.mxl
        || arguments.out.as_ref().is_some_and(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mxl"))
        });

    dispatch_conversion(
        &document,
        &source,
        arguments,
        &input_name,
        &options,
        is_mxl,
        &mut diagnostic_renderer,
    )
}

fn dispatch_conversion(
    document: &OwnedDocument<SimpleSpan<usize>>,
    source: &str,
    arguments: &Arguments,
    input_name: &str,
    options: &MusicXmlOptions,
    is_mxl: bool,
    diagnostic_renderer: &mut DiagnosticRenderer<'_, str, SimpleSpan<usize>>,
) -> Result<(), String> {
    let tunes: Vec<&Tune<SimpleSpan<usize>, String>> = document.tunes().collect();
    if let Some(ref_num) = arguments.tune {
        let matching = tunes
            .iter()
            .find(|t| extract_tune_identity(t).0 == Some(ref_num));
        let Some(tune) = matching else {
            let available: Vec<u32> = tunes
                .iter()
                .filter_map(|t| extract_tune_identity(t).0)
                .collect();
            return Err(MusicXmlError::TuneNotFound {
                requested: ref_num,
                available,
            }
            .to_string());
        };

        return convert_single_tune(
            tune,
            &document.header,
            source,
            input_name,
            arguments.out.as_ref(),
            is_mxl,
            options,
            diagnostic_renderer,
        );
    }

    if arguments.out_dir.is_some() {
        return convert_out_dir(
            document,
            arguments,
            source,
            input_name,
            options,
            diagnostic_renderer,
        );
    }

    if tunes.len() == 1 {
        return convert_single_tune(
            tunes[0],
            &document.header,
            source,
            input_name,
            arguments.out.as_ref(),
            is_mxl,
            options,
            diagnostic_renderer,
        );
    }

    let available: Vec<u32> = tunes
        .iter()
        .filter_map(|t| extract_tune_identity(t).0)
        .collect();
    Err(MusicXmlError::MultipleTunes { available }.to_string())
}

/// Converts a single tune and writes the result to a file or standard output.
#[allow(clippy::too_many_arguments)]
fn convert_single_tune(
    tune: &Tune<SimpleSpan<usize>, String>,
    header: &[SpannedLine],
    source: &str,
    input_name: &str,
    out: Option<&PathBuf>,
    mxl: bool,
    options: &MusicXmlOptions,
    diagnostic_renderer: &mut DiagnosticRenderer<'_, str, SimpleSpan<usize>>,
) -> Result<(), String> {
    let output =
        tune_to_musicxml(tune, header, Some(source), options).map_err(|e| e.to_string())?;
    for warning in &output.warnings {
        eprintln!(
            "abc-musicxml: warning: {input_name}:{}",
            diagnostic_renderer.render_warning(warning)
        );
    }
    write_output(output.xml, out, mxl)
}

/// Converts all tunes in a document and writes them into an output directory.
fn convert_out_dir(
    document: &OwnedDocument<SimpleSpan<usize>>,
    arguments: &Arguments,
    source: &str,
    input_name: &str,
    options: &MusicXmlOptions,
    diagnostic_renderer: &mut DiagnosticRenderer<'_, str, SimpleSpan<usize>>,
) -> Result<(), String> {
    let out_dir = arguments
        .out_dir
        .as_deref()
        .expect("out_dir checked before call");
    fs::create_dir_all(out_dir)
        .map_err(|error| format!("could not create directory {}: {error}", out_dir.display()))?;

    let stem = if arguments.input.as_os_str() == "-" {
        "tune"
    } else {
        arguments
            .input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("tune")
    };

    let outputs =
        document_to_musicxml(document, Some(source), options).map_err(|e| e.to_string())?;

    let ext = if arguments.mxl { "mxl" } else { "musicxml" };
    let mut hrefs = Vec::with_capacity(outputs.len());
    let mut write_tasks = Vec::with_capacity(outputs.len());
    let mut seen_names = std::collections::HashSet::with_capacity(outputs.len());
    for (idx, tune_output) in outputs.into_iter().enumerate() {
        for warning in &tune_output.warnings {
            eprintln!(
                "abc-musicxml: warning: {input_name}:{}",
                diagnostic_renderer.render_warning(warning)
            );
        }
        let ref_str = tune_output
            .reference
            .map_or_else(|| (idx + 1).to_string(), |r| r.to_string());
        let candidate = format!("{stem}-{ref_str}.{ext}");
        let filename = if seen_names.insert(candidate.clone()) {
            candidate
        } else {
            let mut disambiguated = format!("{stem}-{ref_str}-{}.{ext}", idx + 1);
            let mut suffix = 2_usize;
            while !seen_names.insert(disambiguated.clone()) {
                disambiguated = format!("{stem}-{ref_str}-{}-{suffix}.{ext}", idx + 1);
                suffix += 1;
            }
            disambiguated
        };
        let file_path = out_dir.join(&filename);
        hrefs.push(filename);
        write_tasks.push((file_path, tune_output.xml));
    }
    write_tune_files(write_tasks, arguments.mxl)?;

    if hrefs.len() > 1 {
        let title = extract_header_title(document);
        let href_refs: Vec<&str> = hrefs.iter().map(String::as_str).collect();
        let opus_content = generate_opus_xml(title, &href_refs, 2)
            .map_err(|e| format!("failed to generate opus.xml: {e}"))?;
        let opus_path = out_dir.join("opus.xml");
        write_output(opus_content, Some(&opus_path), false)?;
    }

    Ok(())
}

/// Writes generated tune documents with distinct target paths to disk.
fn write_tune_files(tasks: Vec<(PathBuf, String)>, mxl: bool) -> Result<(), String> {
    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::IntoParallelIterator;
        use rayon::prelude::ParallelIterator;

        if tasks.len() > 1 {
            return tasks
                .into_par_iter()
                .try_for_each(|(path, xml)| write_output(xml, Some(&path), mxl));
        }
    }
    tasks
        .into_iter()
        .try_for_each(|(path, xml)| write_output(xml, Some(&path), mxl))
}
