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

//! Canonical ABC source emission from semantic AST nodes.

use super::Accidental;
use super::Annotation;
use super::AnnotationPlacement;
use super::BarLine;
use super::BrokenRhythm;
use super::Chord;
use super::ChordMember;
use super::Decoration;
use super::Directive;
use super::Document;
use super::DocumentItem;
use super::EndingSelector;
use super::Field;
use super::FieldParameter;
use super::FieldValue;
use super::Fraction;
use super::FreeText;
use super::GraceGroup;
use super::KeyAccidental;
use super::KeySignature;
use super::Line;
use super::LineBreak;
use super::MacroDefinition;
use super::Meter;
use super::MultiMeasureRest;
use super::MusicElement;
use super::Note;
use super::NoteLength;
use super::Overlay;
use super::PartSequence;
use super::PartToken;
use super::Pitch;
use super::PitchClass;
use super::Rest;
use super::RestKind;
use super::Slur;
use super::Spanned;
use super::SymbolDefinition;
use super::Tempo;
use super::Tie;
use super::Tune;
use super::Tuplet;
use super::TypesetText;
use super::VariantEnding;
use super::VoiceDefinition;
use std::fmt;
use std::fmt::Write;

/// Selects the canonical spelling used for shortened note lengths.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NoteLengthStyle {
    /// Use repeated slash shorthand such as `A/`, `A//`, and `A///`.
    #[default]
    Shorthand,
    /// Write the denominator explicitly, such as `A/2`, `A/4`, and `A/8`.
    ExplicitDenominator,
}

/// Controls canonical ABC emission choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmitOptions {
    note_length_style: NoteLengthStyle,
}

impl EmitOptions {
    /// Creates options using the default canonical spellings.
    pub const fn new() -> Self {
        Self {
            note_length_style: NoteLengthStyle::Shorthand,
        }
    }

    /// Selects how shortened note lengths are written.
    #[must_use]
    pub const fn with_note_length_style(mut self, style: NoteLengthStyle) -> Self {
        self.note_length_style = style;
        self
    }

    /// Returns the selected shortened note-length spelling.
    pub const fn note_length_style(self) -> NoteLengthStyle {
        self.note_length_style
    }
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Writes AST values using a shared destination and emission preferences.
pub struct AbcEmitter<'writer> {
    output: &'writer mut dyn Write,
    options: EmitOptions,
}

impl<'writer> AbcEmitter<'writer> {
    /// Creates an emitter using the default canonical spellings.
    pub fn new(output: &'writer mut dyn Write) -> Self {
        Self::with_options(output, EmitOptions::new())
    }

    /// Creates an emitter using the supplied canonical spelling preferences.
    pub const fn with_options(output: &'writer mut dyn Write, options: EmitOptions) -> Self {
        Self { output, options }
    }

    /// Returns this emitter's canonical spelling preferences.
    pub const fn options(&self) -> &EmitOptions {
        &self.options
    }

    /// Writes one AST value through this emitter.
    ///
    /// # Errors
    ///
    /// Returns [`fmt::Error`] when the destination writer fails.
    pub fn emit<T>(&mut self, value: &T) -> fmt::Result
    where
        T: ToAbc + ?Sized,
    {
        value.write_abc_with(self)
    }
}

impl Write for AbcEmitter<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.output.write_str(value)
    }

    fn write_char(&mut self, value: char) -> fmt::Result {
        self.output.write_char(value)
    }
}

/// Converts an AST node to canonical ABC notation.
///
/// Source positions are intentionally ignored. Exact spellings retained in
/// textual values and bar lines are reused; syntax normalized by the parser is
/// emitted using a deterministic valid spelling. Programmatically constructed
/// quoted values must not contain an unescaped `"` delimiter.
///
/// ```
/// use abc_parser::EmitOptions;
/// use abc_parser::IntoOwnedAst;
/// use abc_parser::NoteLengthStyle;
/// use abc_parser::ToAbc;
/// use abc_parser::parse;
///
/// let source = "X:1\nM:2+3/8\nK:C\nCDEF |\n";
/// let document = parse(source)
///     .output
///     .unwrap()
///     .into_owned(source)
///     .unwrap();
/// let source = document.to_abc();
/// assert!(source.contains("M:2+3/8"));
/// assert!(source.contains("CDEF |"));
///
/// let options = EmitOptions::new()
///     .with_note_length_style(NoteLengthStyle::ExplicitDenominator);
/// let source = "X:1\nK:C\nA/\n";
/// let document = parse(source)
///     .output
///     .unwrap()
///     .into_owned(source)
///     .unwrap();
/// assert_eq!(document.to_abc_with_options(options), "X:1\nK:C\nA/2");
/// ```
pub trait ToAbc {
    /// Writes this node as ABC notation.
    ///
    /// # Errors
    ///
    /// Returns [`fmt::Error`] when the destination writer fails.
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result;

    /// Writes this node through a configured emitter.
    ///
    /// Implementations predating [`AbcEmitter`] may rely on this default,
    /// which ignores preferences and delegates to [`Self::write_abc`].
    ///
    /// # Errors
    ///
    /// Returns [`fmt::Error`] when the emitter's destination writer fails.
    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        self.write_abc(emitter)
    }

    /// Returns this node as an owned ABC source string.
    fn to_abc(&self) -> String {
        let mut output = String::new();
        if self.write_abc(&mut output).is_err() {
            unreachable!("writing to String cannot fail");
        }
        output
    }

    /// Returns this node using the supplied canonical spelling preferences.
    fn to_abc_with_options(&self, options: EmitOptions) -> String {
        let mut output = String::new();
        let mut emitter = AbcEmitter::with_options(&mut output, options);
        if emitter.emit(self).is_err() {
            unreachable!("writing to String cannot fail");
        }
        output
    }
}

impl<T, S> ToAbc for Spanned<T, S>
where
    T: ToAbc,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        self.value.write_abc(output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.emit(&self.value)
    }
}

impl<S, T> ToAbc for Document<S, T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        let mut needs_separator = false;
        for line in &self.header {
            write_line_with_separator(&line.value, output, &mut needs_separator)?;
        }
        for item in &self.items {
            if needs_separator {
                output.write_str("\n\n")?;
            }
            item.value.write_abc(output)?;
            needs_separator = true;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        let mut needs_separator = false;
        for line in &self.header {
            write_line_with_emitter(&line.value, emitter, &mut needs_separator)?;
        }
        for item in &self.items {
            if needs_separator {
                emitter.write_str("\n\n")?;
            }
            emitter.emit(&item.value)?;
            needs_separator = true;
        }
        Ok(())
    }
}

impl<S, T> ToAbc for DocumentItem<S, T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Tune(tune) => tune.write_abc(output),
            Self::FreeText(text) => text.write_abc(output),
            Self::TypesetText(text) => text.write_abc(output),
            Self::Comment(text) => write!(output, "%{}", text.as_ref()),
            Self::Directive(value) => value.write_abc(output),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Tune(tune) => emitter.emit(tune),
            Self::FreeText(text) => emitter.emit(text),
            Self::TypesetText(text) => emitter.emit(text),
            Self::Comment(text) => write!(emitter, "%{}", text.as_ref()),
            Self::Directive(value) => emitter.emit(value),
        }
    }
}

impl<T> ToAbc for FreeText<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                output.write_char('\n')?;
            }
            output.write_str(line.as_ref())?;
        }
        Ok(())
    }
}

impl<T> ToAbc for TypesetText<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Text(text) => write!(output, "%%text {}", text.as_ref()),
            Self::Centered(text) => write!(output, "%%center {}", text.as_ref()),
            Self::Block(lines) => {
                output.write_str("%%begintext")?;
                for line in lines {
                    output.write_char('\n')?;
                    write!(output, "%%{}", line.as_ref())?;
                }
                output.write_str("\n%%endtext")
            }
        }
    }
}

impl<S, T> ToAbc for Tune<S, T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        let mut needs_separator = false;
        for line in &self.lines {
            write_line_with_separator(&line.value, output, &mut needs_separator)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        let mut needs_separator = false;
        for line in &self.lines {
            write_line_with_emitter(&line.value, emitter, &mut needs_separator)?;
        }
        Ok(())
    }
}

/// Writes one line, prefixing a newline after the first emitted line.
fn write_line_with_separator<S, T>(
    line: &Line<S, T>,
    output: &mut dyn Write,
    needs_separator: &mut bool,
) -> fmt::Result
where
    T: AsRef<str>,
{
    if *needs_separator {
        output.write_char('\n')?;
    }
    *needs_separator = true;
    line.write_abc(output)
}

/// Writes one line through an emitter, prefixing a newline after the first line.
fn write_line_with_emitter<S, T>(
    line: &Line<S, T>,
    emitter: &mut AbcEmitter<'_>,
    needs_separator: &mut bool,
) -> fmt::Result
where
    T: AsRef<str>,
{
    if *needs_separator {
        emitter.write_char('\n')?;
    }
    *needs_separator = true;
    emitter.emit(line)
}

impl<S, T> ToAbc for Line<S, T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Blank => Ok(()),
            Self::Comment(text) => write!(output, "%{}", text.as_ref()),
            Self::Directive(directive) => directive.write_abc(output),
            Self::Field(field) => field.write_abc(output),
            Self::Music(elements) => {
                for element in elements {
                    element.value.write_abc(output)?;
                }
                Ok(())
            }
            Self::TypesetText(text) => text.write_abc(output),
            Self::DirectiveText(text) => write!(output, "%%{}", text.as_ref()),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Blank => Ok(()),
            Self::Comment(text) => write!(emitter, "%{}", text.as_ref()),
            Self::Directive(directive) => emitter.emit(directive),
            Self::Field(field) => emitter.emit(field),
            Self::Music(elements) => {
                for element in elements {
                    emitter.emit(&element.value)?;
                }
                Ok(())
            }
            Self::TypesetText(text) => emitter.emit(text),
            Self::DirectiveText(text) => write!(emitter, "%%{}", text.as_ref()),
        }
    }
}

impl<T> ToAbc for Directive<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(output, "%%{}", self.name.as_ref())?;
        if !self.arguments.as_ref().is_empty() {
            write!(output, " {}", self.arguments.as_ref())?;
        }
        Ok(())
    }
}

impl<T> ToAbc for Field<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(output, "{}:", self.key)?;
        self.value.write_abc(output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        write!(emitter, "{}:", self.key)?;
        emitter.emit(&self.value)
    }
}

impl<T> ToAbc for FieldValue<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Text(text) | Self::Unparsed(text) => output.write_str(text.as_ref()),
            Self::UnitLength(value) => value.write_abc(output),
            Self::Meter(value) => value.write_abc(output),
            Self::Tempo(value) => value.write_abc(output),
            Self::Key(value) => value.write_abc(output),
            Self::Reference(value) => write!(output, "{value}"),
            Self::Voice(value) => value.write_abc(output),
            Self::Parts(value) => value.write_abc(output),
            Self::UserSymbol(value) => value.write_abc(output),
            Self::Macro(value) => value.write_abc(output),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) | Self::Unparsed(text) => emitter.write_str(text.as_ref()),
            Self::UnitLength(value) => emitter.emit(value),
            Self::Meter(value) => emitter.emit(value),
            Self::Tempo(value) => emitter.emit(value),
            Self::Key(value) => emitter.emit(value),
            Self::Reference(value) => write!(emitter, "{value}"),
            Self::Voice(value) => emitter.emit(value),
            Self::Parts(value) => emitter.emit(value),
            Self::UserSymbol(value) => emitter.emit(value),
            Self::Macro(value) => emitter.emit(value),
        }
    }
}

impl ToAbc for Fraction {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(output, "{}/{}", self.numerator, self.denominator)
    }
}

impl ToAbc for Meter {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Common => output.write_char('C'),
            Self::Cut => output.write_str("C|"),
            Self::None => output.write_str("none"),
            Self::Simple(value) => value.write_abc(output),
            Self::Compound {
                groups,
                denominator,
            } => {
                for (index, group) in groups.iter().enumerate() {
                    if index > 0 {
                        output.write_char('+')?;
                    }
                    write!(output, "{group}")?;
                }
                write!(output, "/{denominator}")
            }
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Common => emitter.write_char('C'),
            Self::Cut => emitter.write_str("C|"),
            Self::None => emitter.write_str("none"),
            Self::Simple(value) => emitter.emit(value),
            Self::Compound {
                groups,
                denominator,
            } => {
                for (index, group) in groups.iter().enumerate() {
                    if index > 0 {
                        emitter.write_char('+')?;
                    }
                    write!(emitter, "{group}")?;
                }
                write!(emitter, "/{denominator}")
            }
        }
    }
}

impl<T> ToAbc for Tempo<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        if let Some(prelude) = &self.prelude {
            write_quoted(prelude.as_ref(), output)?;
            output.write_char(' ')?;
        }
        for (index, beat) in self.beats.iter().enumerate() {
            if index > 0 {
                output.write_char(' ')?;
            }
            beat.write_abc(output)?;
        }
        write!(output, "={}", self.bpm)?;
        if let Some(postlude) = &self.postlude {
            output.write_char(' ')?;
            write_quoted(postlude.as_ref(), output)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        if let Some(prelude) = &self.prelude {
            write_quoted(prelude.as_ref(), emitter)?;
            emitter.write_char(' ')?;
        }
        for (index, beat) in self.beats.iter().enumerate() {
            if index > 0 {
                emitter.write_char(' ')?;
            }
            emitter.emit(beat)?;
        }
        write!(emitter, "={}", self.bpm)?;
        if let Some(postlude) = &self.postlude {
            emitter.write_char(' ')?;
            write_quoted(postlude.as_ref(), emitter)?;
        }
        Ok(())
    }
}

impl<T> ToAbc for KeySignature<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        if let Some(tonic) = self.tonic {
            write_pitch_class(tonic.class, false, output)?;
            match tonic.accidental {
                Some(KeyAccidental::Sharp) => output.write_char('#')?,
                Some(KeyAccidental::Flat) => output.write_char('b')?,
                None => {}
            }
        } else {
            output.write_str("none")?;
        }
        output.write_str(self.mode.as_ref())?;
        for parameter in &self.parameters {
            output.write_char(' ')?;
            parameter.write_abc(output)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        if let Some(tonic) = self.tonic {
            write_pitch_class(tonic.class, false, emitter)?;
            match tonic.accidental {
                Some(KeyAccidental::Sharp) => emitter.write_char('#')?,
                Some(KeyAccidental::Flat) => emitter.write_char('b')?,
                None => {}
            }
        } else {
            emitter.write_str("none")?;
        }
        emitter.write_str(self.mode.as_ref())?;
        for parameter in &self.parameters {
            emitter.write_char(' ')?;
            emitter.emit(parameter)?;
        }
        Ok(())
    }
}

impl<T> ToAbc for VoiceDefinition<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_str(self.id.as_ref())?;
        for property in &self.properties {
            output.write_char(' ')?;
            property.write_abc(output)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.write_str(self.id.as_ref())?;
        for property in &self.properties {
            emitter.write_char(' ')?;
            emitter.emit(property)?;
        }
        Ok(())
    }
}

impl<T> ToAbc for FieldParameter<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        if let Some(name) = &self.name {
            write!(output, "{}=", name.as_ref())?;
        }
        let value = self.value.as_ref();
        if parameter_needs_quotes(value) {
            write_quoted(value, output)
        } else {
            output.write_str(value)
        }
    }
}

/// Determines whether a field parameter needs delimiters when emitted.
fn parameter_needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value
            .chars()
            .any(|character| character.is_whitespace() || matches!(character, '=' | ']'))
}

/// Writes text inside the quote delimiters used by ABC fields and annotations.
fn write_quoted(text: &str, output: &mut dyn Write) -> fmt::Result {
    output.write_char('"')?;
    output.write_str(text)?;
    output.write_char('"')
}

impl<T> ToAbc for PartSequence<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        for token in &self.tokens {
            token.write_abc(output)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        for token in &self.tokens {
            emitter.emit(token)?;
        }
        Ok(())
    }
}

impl<T> ToAbc for PartToken<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Part(name) => output.write_str(name.as_ref()),
            Self::Repeat(count) => write!(output, "{count}"),
            Self::Open => output.write_char('('),
            Self::Close => output.write_char(')'),
            Self::Separator => output.write_char('.'),
        }
    }
}

impl<T> ToAbc for SymbolDefinition<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(output, "{}={}", self.symbol, self.replacement.as_ref())
    }
}

impl<T> ToAbc for MacroDefinition<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(
            output,
            "{}={}",
            self.pattern.as_ref(),
            self.replacement.as_ref()
        )
    }
}

impl<T> ToAbc for MusicElement<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Note(value) => value.write_abc(output),
            Self::Rest(value) => value.write_abc(output),
            Self::MultiMeasureRest(value) => value.write_abc(output),
            Self::Chord(value) => value.write_abc(output),
            Self::Bar(value) => value.write_abc(output),
            Self::Ending(value) => value.write_abc(output),
            Self::InlineField(value) => {
                output.write_char('[')?;
                value.write_abc(output)?;
                output.write_char(']')
            }
            Self::Grace(value) => value.write_abc(output),
            Self::Decoration(value) => value.write_abc(output),
            Self::Annotation(value) => value.write_abc(output),
            Self::Tuplet(value) => value.write_abc(output),
            Self::Slur(value) => value.write_abc(output),
            Self::Tie(value) => value.write_abc(output),
            Self::BrokenRhythm(value) => value.write_abc(output),
            Self::Overlay(value) => value.write_abc(output),
            Self::BeamBreak(value) | Self::Extension(value) => output.write_str(value.as_ref()),
            Self::BeamContinuation(count) => write_repeated('`', *count, output),
            Self::LineBreak(value) => value.write_abc(output),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Note(value) => emitter.emit(value),
            Self::Rest(value) => emitter.emit(value),
            Self::MultiMeasureRest(value) => emitter.emit(value),
            Self::Chord(value) => emitter.emit(value),
            Self::Bar(value) => emitter.emit(value),
            Self::Ending(value) => emitter.emit(value),
            Self::InlineField(value) => {
                emitter.write_char('[')?;
                emitter.emit(value)?;
                emitter.write_char(']')
            }
            Self::Grace(value) => emitter.emit(value),
            Self::Decoration(value) => emitter.emit(value),
            Self::Annotation(value) => emitter.emit(value),
            Self::Tuplet(value) => emitter.emit(value),
            Self::Slur(value) => emitter.emit(value),
            Self::Tie(value) => emitter.emit(value),
            Self::BrokenRhythm(value) => emitter.emit(value),
            Self::Overlay(value) => emitter.emit(value),
            Self::BeamBreak(value) | Self::Extension(value) => emitter.write_str(value.as_ref()),
            Self::BeamContinuation(count) => write_repeated('`', *count, emitter),
            Self::LineBreak(value) => emitter.emit(value),
        }
    }
}

impl ToAbc for Note {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        self.pitch.write_abc(output)?;
        self.length.write_abc(output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.emit(&self.pitch)?;
        emitter.emit(&self.length)
    }
}

impl ToAbc for Pitch {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        if let Some(accidental) = self.accidental {
            accidental.write_abc(output)?;
        }
        let lowercase = self.octave >= 1;
        write_pitch_class(self.class, lowercase, output)?;
        if lowercase {
            let count = usize::from(self.octave.unsigned_abs().saturating_sub(1));
            write_repeated('\'', count, output)
        } else {
            write_repeated(',', usize::from(self.octave.unsigned_abs()), output)
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        if let Some(accidental) = self.accidental {
            emitter.emit(&accidental)?;
        }
        let lowercase = self.octave >= 1;
        write_pitch_class(self.class, lowercase, emitter)?;
        if lowercase {
            let count = usize::from(self.octave.unsigned_abs().saturating_sub(1));
            write_repeated('\'', count, emitter)
        } else {
            write_repeated(',', usize::from(self.octave.unsigned_abs()), emitter)
        }
    }
}

/// Writes a pitch letter using the octave-significant ABC case convention.
fn write_pitch_class(class: PitchClass, lowercase: bool, output: &mut dyn Write) -> fmt::Result {
    let letter = match class {
        PitchClass::C => 'C',
        PitchClass::D => 'D',
        PitchClass::E => 'E',
        PitchClass::F => 'F',
        PitchClass::G => 'G',
        PitchClass::A => 'A',
        PitchClass::B => 'B',
    };
    output.write_char(if lowercase {
        letter.to_ascii_lowercase()
    } else {
        letter
    })
}

impl ToAbc for Accidental {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Natural => output.write_char('='),
            Self::Sharp(amount) => write_accidental('^', *amount, output),
            Self::Flat(amount) => write_accidental('_', *amount, output),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Natural => emitter.write_char('='),
            Self::Sharp(amount) => write_accidental('^', *amount, emitter),
            Self::Flat(amount) => write_accidental('_', *amount, emitter),
        }
    }
}

/// Writes a sharp or flat amount in the parser's canonical fraction spelling.
fn write_accidental(marker: char, amount: Fraction, output: &mut dyn Write) -> fmt::Result {
    output.write_char(marker)?;
    if amount
        == (Fraction {
            numerator: 1,
            denominator: 1,
        })
    {
        return Ok(());
    }
    write!(output, "{}", amount.numerator)?;
    if amount.denominator != 1 {
        write!(output, "/{}", amount.denominator)?;
    }
    Ok(())
}

impl ToAbc for NoteLength {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write_note_length(*self, NoteLengthStyle::Shorthand, output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        let style = emitter.options().note_length_style();
        write_note_length(*self, style, emitter)
    }
}

/// Writes one note length using the selected equivalent spelling.
fn write_note_length(
    length: NoteLength,
    style: NoteLengthStyle,
    output: &mut dyn Write,
) -> fmt::Result {
    match (length.numerator, length.denominator) {
        (1, 1) => Ok(()),
        (numerator, 1) => write!(output, "{numerator}"),
        (1, denominator)
            if style == NoteLengthStyle::Shorthand && denominator.is_power_of_two() =>
        {
            write_repeated('/', denominator.ilog2() as usize, output)
        }
        (1, denominator) => write!(output, "/{denominator}"),
        (numerator, denominator) => write!(output, "{numerator}/{denominator}"),
    }
}

impl ToAbc for Rest {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char(match self.kind {
            RestKind::Visible => 'z',
            RestKind::Invisible => 'x',
        })?;
        self.length.write_abc(output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.write_char(match self.kind {
            RestKind::Visible => 'z',
            RestKind::Invisible => 'x',
        })?;
        emitter.emit(&self.length)
    }
}

impl ToAbc for MultiMeasureRest {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char(if self.invisible { 'X' } else { 'Z' })?;
        if self.measures != 1 {
            write!(output, "{}", self.measures)?;
        }
        Ok(())
    }
}

impl ToAbc for Chord {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char('[')?;
        for member in &self.members {
            member.write_abc(output)?;
        }
        output.write_char(']')?;
        self.length.write_abc(output)
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.write_char('[')?;
        for member in &self.members {
            emitter.emit(member)?;
        }
        emitter.write_char(']')?;
        emitter.emit(&self.length)
    }
}

impl ToAbc for ChordMember {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Note(value) => value.write_abc(output),
            Self::Rest(value) => value.write_abc(output),
        }
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        match self {
            Self::Note(value) => emitter.emit(value),
            Self::Rest(value) => emitter.emit(value),
        }
    }
}

impl<T> ToAbc for BarLine<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_str(self.source.as_ref())
    }
}

impl ToAbc for VariantEnding {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char('[')?;
        for (index, selector) in self.selectors.iter().enumerate() {
            if index > 0 {
                output.write_char(',')?;
            }
            selector.write_abc(output)?;
        }
        Ok(())
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.write_char('[')?;
        for (index, selector) in self.selectors.iter().enumerate() {
            if index > 0 {
                emitter.write_char(',')?;
            }
            emitter.emit(selector)?;
        }
        Ok(())
    }
}

impl ToAbc for EndingSelector {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match self {
            Self::Number(value) => write!(output, "{value}"),
            Self::Range { start, end } => write!(output, "{start}-{end}"),
        }
    }
}

impl ToAbc for GraceGroup {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char('{')?;
        if self.acciaccatura {
            output.write_char('/')?;
        }
        for note in &self.notes {
            note.write_abc(output)?;
        }
        output.write_char('}')
    }

    fn write_abc_with(&self, emitter: &mut AbcEmitter<'_>) -> fmt::Result {
        emitter.write_char('{')?;
        if self.acciaccatura {
            emitter.write_char('/')?;
        }
        for note in &self.notes {
            emitter.emit(note)?;
        }
        emitter.write_char('}')
    }
}

impl<T> ToAbc for Decoration<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        let delimiter = if self.legacy_delimiter { '+' } else { '!' };
        output.write_char(delimiter)?;
        output.write_str(self.name.as_ref())?;
        output.write_char(delimiter)
    }
}

impl<T> ToAbc for Annotation<T>
where
    T: AsRef<str>,
{
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_char('"')?;
        match self.placement {
            AnnotationPlacement::ChordSymbol => {}
            AnnotationPlacement::Above => output.write_char('^')?,
            AnnotationPlacement::Below => output.write_char('_')?,
            AnnotationPlacement::Left => output.write_char('<')?,
            AnnotationPlacement::Right => output.write_char('>')?,
            AnnotationPlacement::Free => output.write_char('@')?,
        }
        output.write_str(self.text.as_ref())?;
        output.write_char('"')
    }
}

impl ToAbc for Tuplet {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write!(output, "({}", self.p)?;
        if self.q.is_some() || self.r.is_some() {
            output.write_char(':')?;
            if let Some(q) = self.q {
                write!(output, "{q}")?;
            }
        }
        if let Some(r) = self.r {
            write!(output, ":{r}")?;
        }
        Ok(())
    }
}

impl ToAbc for Slur {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        match (self.opening, self.dotted) {
            (true, true) => output.write_str(".("),
            (false, true) => output.write_str(".)"),
            (true, false) => output.write_char('('),
            (false, false) => output.write_char(')'),
        }
    }
}

impl ToAbc for Tie {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_str(if self.dotted { ".-" } else { "-" })
    }
}

impl ToAbc for BrokenRhythm {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        write_repeated(
            if self.greater { '>' } else { '<' },
            usize::from(self.count),
            output,
        )
    }
}

impl ToAbc for Overlay {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_str(match self {
            Self::NextVoice => "&",
            Self::Start => "(&",
            Self::End => "&)",
        })
    }
}

impl ToAbc for LineBreak {
    fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
        output.write_str(match self {
            Self::Continue => "\\",
            Self::Break => "$",
            Self::Paragraph => "$$",
            Self::Space => "y",
        })
    }
}

/// Writes one character a fixed number of times without an intermediate string.
fn write_repeated(character: char, count: usize, output: &mut dyn Write) -> fmt::Result {
    for _ in 0..count {
        output.write_char(character)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IntoOwnedAst;
    use crate::parse;
    use crate::parse_field;
    use crate::parse_music_line;

    /// A pre-emitter-context implementation used to verify compatibility.
    struct LegacyValue;

    impl ToAbc for LegacyValue {
        fn write_abc(&self, output: &mut dyn Write) -> fmt::Result {
            output.write_str("legacy")
        }
    }

    #[test]
    fn structured_fields_use_canonical_spellings() {
        assert_eq!(parse_field("M:(2+3)/8").unwrap().to_abc(), "M:2+3/8");
        assert_eq!(
            parse_field("Q:\"Allegro\" 1/4=120 \"brightly\"")
                .unwrap()
                .to_abc(),
            "Q:\"Allegro\" 1/4=120 \"brightly\""
        );
        assert_eq!(
            parse_field("V:1 name=\"Soprano One\" clef=treble")
                .unwrap()
                .to_abc(),
            "V:1 name=\"Soprano One\" clef=treble"
        );
    }

    #[test]
    fn emitted_music_controls_are_accepted_by_the_parser() {
        let source = r"(&C&)\$$y";
        let parsed = parse_music_line(source);
        assert!(parsed.is_valid(), "{:#?}", parsed.errors);
        let emitted = parsed
            .output
            .as_ref()
            .unwrap()
            .iter()
            .map(ToAbc::to_abc)
            .collect::<String>();
        assert_eq!(emitted, source);
    }

    #[test]
    fn pitch_spelling_tracks_octaves_accidentals_and_lengths() {
        let note = Note {
            pitch: Pitch {
                class: PitchClass::B,
                octave: 3,
                accidental: Some(Accidental::Sharp(Fraction {
                    numerator: 1,
                    denominator: 2,
                })),
            },
            length: NoteLength {
                numerator: 3,
                denominator: 2,
            },
        };
        assert_eq!(note.to_abc(), "^1/2b''3/2");
    }

    #[test]
    fn note_length_styles_cover_equivalent_and_irregular_fractions() {
        let explicit =
            EmitOptions::new().with_note_length_style(NoteLengthStyle::ExplicitDenominator);
        let cases = [
            (
                NoteLength {
                    numerator: 1,
                    denominator: 1,
                },
                "",
                "",
            ),
            (
                NoteLength {
                    numerator: 1,
                    denominator: 2,
                },
                "/",
                "/2",
            ),
            (
                NoteLength {
                    numerator: 1,
                    denominator: 4,
                },
                "//",
                "/4",
            ),
            (
                NoteLength {
                    numerator: 1,
                    denominator: 8,
                },
                "///",
                "/8",
            ),
            (
                NoteLength {
                    numerator: 1,
                    denominator: 3,
                },
                "/3",
                "/3",
            ),
            (
                NoteLength {
                    numerator: 1,
                    denominator: 6,
                },
                "/6",
                "/6",
            ),
            (
                NoteLength {
                    numerator: 3,
                    denominator: 2,
                },
                "3/2",
                "3/2",
            ),
            (
                NoteLength {
                    numerator: 2,
                    denominator: 4,
                },
                "2/4",
                "2/4",
            ),
        ];
        for (length, shorthand, denominator) in cases {
            assert_eq!(length.to_abc(), shorthand);
            assert_eq!(length.to_abc_with_options(explicit), denominator);
        }
    }

    #[test]
    fn configured_emitter_reaches_nested_music_lengths() {
        let source = "X:1\nK:C\nA/ z// [B/z//]/// {c/}\n";
        let document = parse(source).output.unwrap().into_owned(source).unwrap();
        let explicit =
            EmitOptions::new().with_note_length_style(NoteLengthStyle::ExplicitDenominator);
        assert_eq!(document.to_abc(), "X:1\nK:C\nA/ z// [B/z//]/// {c/}");
        assert_eq!(
            document.to_abc_with_options(explicit),
            "X:1\nK:C\nA/2 z/4 [B/2z/4]/8 {c/2}"
        );
    }

    #[test]
    fn emitter_supports_legacy_trait_implementations() {
        let options =
            EmitOptions::new().with_note_length_style(NoteLengthStyle::ExplicitDenominator);
        let mut output = String::new();
        let mut emitter = AbcEmitter::with_options(&mut output, options);
        assert_eq!(
            emitter.options().note_length_style(),
            NoteLengthStyle::ExplicitDenominator
        );
        emitter.emit(&LegacyValue).unwrap();
        assert_eq!(output, "legacy");
    }
}
