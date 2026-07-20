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

//! Source-backed text and standalone AST conversion support.

use std::borrow::Cow;
use std::convert::Infallible;
use std::fmt;
use std::ops::Range;

use chumsky::span::SimpleSpan;

use super::Annotation;
use super::BarLine;
use super::Directive;
use super::Document;
use super::DocumentItem;
use super::Field;
use super::FieldParameter;
use super::FieldValue;
use super::FreeText;
use super::KeySignature;
use super::Line;
use super::MacroDefinition;
use super::MusicElement;
use super::PartSequence;
use super::PartToken;
use super::Spanned;
use super::SymbolDefinition;
use super::Tempo;
use super::Tune;
use super::TypesetText;
use super::VoiceDefinition;

/// Text retained from source or synthesized while parsing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceText<S> {
    /// A reference to text in the parser input.
    Span(S),
    /// Text retained, normalized, or synthesized directly by the parser.
    Synthesized(String),
}

/// Resolves a parser span into human-readable source text.
pub trait SourceResolver<S> {
    /// Error returned when the span cannot be resolved.
    type Error;

    /// Resolves `span`, borrowing text when the source representation permits.
    ///
    /// # Errors
    ///
    /// Returns the resolver-specific error when the span is invalid.
    fn resolve<'src>(&'src self, span: &S) -> Result<Cow<'src, str>, Self::Error>;

    /// Returns the complete original source when it remains available.
    ///
    /// Diagnostic renderers use this to calculate line and column positions and
    /// display surrounding source. Resolvers without the complete source may
    /// retain the default implementation.
    fn full_source(&self) -> Option<Cow<'_, str>> {
        None
    }

    /// Converts a native span to byte offsets in [`Self::full_source`].
    ///
    /// Diagnostic rendering uses this to support sources whose native offsets
    /// are not UTF-8 byte offsets. Resolvers without a complete source may
    /// retain the default implementation.
    fn diagnostic_range(&self, _span: &S) -> Option<Range<usize>> {
        None
    }
}

/// Converts a span-backed AST node into a standalone owned node.
pub trait IntoOwnedAst<S>: Sized {
    /// Standalone output type.
    type Owned;

    /// Resolves every source-backed text value and preserves semantic values.
    ///
    /// ```
    /// use abc_parser::IntoOwnedAst;
    /// use abc_parser::Line;
    /// use abc_parser::SourceText;
    /// use chumsky::span::SimpleSpan;
    ///
    /// let line = Line::<SimpleSpan<usize>>::Comment(SourceText::Span((1..8).into()));
    /// let owned = line.into_owned("%comment").unwrap();
    /// assert_eq!(owned, Line::Comment("comment".to_owned()));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the first error produced by `resolver`.
    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized;
}

/// Failure to resolve a span against an available source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResolveError {
    /// The span lies outside the source.
    OutOfBounds,
    /// A byte span does not fall on UTF-8 character boundaries.
    InvalidCharBoundary,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds => formatter.write_str("source span is out of bounds"),
            Self::InvalidCharBoundary => {
                formatter.write_str("source span is not on UTF-8 character boundaries")
            }
        }
    }
}

impl std::error::Error for ResolveError {}

impl SourceResolver<SimpleSpan<usize>> for str {
    type Error = ResolveError;

    fn resolve<'src>(&'src self, span: &SimpleSpan<usize>) -> Result<Cow<'src, str>, Self::Error> {
        if span.start > span.end || span.end > self.len() {
            return Err(ResolveError::OutOfBounds);
        }
        if !self.is_char_boundary(span.start) || !self.is_char_boundary(span.end) {
            return Err(ResolveError::InvalidCharBoundary);
        }
        Ok(Cow::Borrowed(&self[span.start..span.end]))
    }

    fn full_source(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self))
    }

    fn diagnostic_range(&self, span: &SimpleSpan<usize>) -> Option<Range<usize>> {
        self.get(span.start..span.end).map(|_| span.start..span.end)
    }
}

impl SourceResolver<SimpleSpan<usize>> for [char] {
    type Error = ResolveError;

    fn resolve<'src>(&'src self, span: &SimpleSpan<usize>) -> Result<Cow<'src, str>, Self::Error> {
        let characters = self
            .get(span.start..span.end)
            .ok_or(ResolveError::OutOfBounds)?;
        Ok(Cow::Owned(characters.iter().collect()))
    }

    fn full_source(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.iter().collect()))
    }

    fn diagnostic_range(&self, span: &SimpleSpan<usize>) -> Option<Range<usize>> {
        if span.start > span.end || span.end > self.len() {
            return None;
        }
        let start = self[..span.start]
            .iter()
            .map(|character| character.len_utf8())
            .sum();
        let length = self[span.start..span.end]
            .iter()
            .map(|character| character.len_utf8())
            .sum::<usize>();
        Some(start..start + length)
    }
}

impl SourceResolver<Range<usize>> for str {
    type Error = ResolveError;

    fn resolve<'src>(&'src self, span: &Range<usize>) -> Result<Cow<'src, str>, Self::Error> {
        self.resolve(&SimpleSpan::from(span.clone()))
    }

    fn full_source(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self))
    }

    fn diagnostic_range(&self, span: &Range<usize>) -> Option<Range<usize>> {
        self.get(span.clone()).map(|_| span.clone())
    }
}

impl SourceResolver<Range<usize>> for [char] {
    type Error = ResolveError;

    fn resolve<'src>(&'src self, span: &Range<usize>) -> Result<Cow<'src, str>, Self::Error> {
        self.resolve(&SimpleSpan::from(span.clone()))
    }

    fn full_source(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Owned(self.iter().collect()))
    }

    fn diagnostic_range(&self, span: &Range<usize>) -> Option<Range<usize>> {
        SourceResolver::<SimpleSpan<usize>>::diagnostic_range(self, &SimpleSpan::from(span.clone()))
    }
}

/// Prefix identifying a generated missing-source placeholder.
pub const SOURCE_REFERENCE_PREFIX: &str = "[[ABC_SOURCE_REF:";

/// Suffix terminating a generated missing-source placeholder.
pub const SOURCE_REFERENCE_SUFFIX: &str = "]]";

/// Detects the documented placeholder shape heuristically.
///
/// Legitimate source text can have the same shape and is intentionally
/// indistinguishable after ownership conversion.
pub fn is_source_reference_placeholder(text: &str) -> bool {
    text.starts_with(SOURCE_REFERENCE_PREFIX) && text.ends_with(SOURCE_REFERENCE_SUFFIX)
}

/// Resolves every span to a conspicuous placeholder without needing source.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlaceholderResolver;

impl<S> SourceResolver<S> for PlaceholderResolver
where
    S: fmt::Debug,
{
    type Error = Infallible;

    fn resolve<'src>(&'src self, span: &S) -> Result<Cow<'src, str>, Self::Error> {
        Ok(Cow::Owned(format!(
            "{SOURCE_REFERENCE_PREFIX}{span:?}{SOURCE_REFERENCE_SUFFIX}"
        )))
    }
}

impl<S> IntoOwnedAst<S> for SourceText<S> {
    type Owned = String;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        match self {
            Self::Span(span) => resolver.resolve(&span).map(Cow::into_owned),
            Self::Synthesized(text) => Ok(text),
        }
    }
}

impl<S, T> IntoOwnedAst<S> for Spanned<T, S>
where
    T: IntoOwnedAst<S>,
{
    type Owned = Spanned<T::Owned, S>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Spanned {
            value: self.value.into_owned(resolver)?,
            span: self.span,
        })
    }
}

impl<S> IntoOwnedAst<S> for Directive<SourceText<S>> {
    type Owned = Directive<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Directive {
            name: self.name.into_owned(resolver)?,
            arguments: self.arguments.into_owned(resolver)?,
            kind: self.kind,
            body: self.body.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for FieldParameter<SourceText<S>> {
    type Owned = FieldParameter<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(FieldParameter {
            name: self
                .name
                .map(|name| name.into_owned(resolver))
                .transpose()?,
            value: self.value.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for Tempo<SourceText<S>> {
    type Owned = Tempo<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::MetronomeMark {
                prelude,
                beats,
                bpm,
                postlude,
            } => Tempo::MetronomeMark {
                prelude: prelude.map(|text| text.into_owned(resolver)).transpose()?,
                beats,
                bpm,
                postlude: postlude.map(|text| text.into_owned(resolver)).transpose()?,
            },
            Self::TextOnly(text) => Tempo::TextOnly(text.into_owned(resolver)?),
            Self::Deprecated(text) => Tempo::Deprecated(text.into_owned(resolver)?),
        })
    }
}

impl<S> IntoOwnedAst<S> for KeySignature<SourceText<S>> {
    type Owned = KeySignature<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(KeySignature {
            tonic: self.tonic,
            mode: self.mode.into_owned(resolver)?,
            parameters: self
                .parameters
                .into_iter()
                .map(|parameter| parameter.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl<S> IntoOwnedAst<S> for VoiceDefinition<SourceText<S>> {
    type Owned = VoiceDefinition<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(VoiceDefinition {
            id: self.id.into_owned(resolver)?,
            properties: self
                .properties
                .into_iter()
                .map(|property| property.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl<S> IntoOwnedAst<S> for PartToken<SourceText<S>> {
    type Owned = PartToken<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Part(name) => PartToken::Part(name.into_owned(resolver)?),
            Self::Repeat(count) => PartToken::Repeat(count),
            Self::Open => PartToken::Open,
            Self::Close => PartToken::Close,
            Self::Separator => PartToken::Separator,
        })
    }
}

impl<S> IntoOwnedAst<S> for PartSequence<SourceText<S>> {
    type Owned = PartSequence<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(PartSequence {
            tokens: self
                .tokens
                .into_iter()
                .map(|token| token.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl<S> IntoOwnedAst<S> for SymbolDefinition<SourceText<S>> {
    type Owned = SymbolDefinition<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(SymbolDefinition {
            symbol: self.symbol,
            replacement: self.replacement.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for MacroDefinition<SourceText<S>> {
    type Owned = MacroDefinition<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(MacroDefinition {
            pattern: self.pattern.into_owned(resolver)?,
            replacement: self.replacement.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for FieldValue<SourceText<S>> {
    type Owned = FieldValue<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Empty => FieldValue::Empty,
            Self::Text(text) => FieldValue::Text(text.into_owned(resolver)?),
            Self::UnitLength(value) => FieldValue::UnitLength(value),
            Self::Meter(value) => FieldValue::Meter(value),
            Self::Tempo(value) => FieldValue::Tempo(value.into_owned(resolver)?),
            Self::Key(value) => FieldValue::Key(value.into_owned(resolver)?),
            Self::Reference(value) => FieldValue::Reference(value),
            Self::Voice(value) => FieldValue::Voice(value.into_owned(resolver)?),
            Self::Parts(value) => FieldValue::Parts(value.into_owned(resolver)?),
            Self::UserSymbol(value) => FieldValue::UserSymbol(value.into_owned(resolver)?),
            Self::Macro(value) => FieldValue::Macro(value.into_owned(resolver)?),
            Self::Unparsed(text) => FieldValue::Unparsed(match text {
                SourceText::Span(span) => resolver.resolve(&span)?.trim().to_owned(),
                SourceText::Synthesized(text) => text.trim().to_owned(),
            }),
        })
    }
}

impl<S> IntoOwnedAst<S> for Field<SourceText<S>> {
    type Owned = Field<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Field {
            key: self.key,
            kind: self.kind,
            value: self.value.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for BarLine<SourceText<S>> {
    type Owned = BarLine<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(BarLine {
            kind: self.kind,
            source: self.source.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for super::Decoration<SourceText<S>> {
    type Owned = super::Decoration<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(super::Decoration {
            name: self.name.into_owned(resolver)?,
            legacy_delimiter: self.legacy_delimiter,
        })
    }
}

impl<S> IntoOwnedAst<S> for Annotation<SourceText<S>> {
    type Owned = Annotation<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Annotation {
            placement: self.placement,
            text: self.text.into_owned(resolver)?,
        })
    }
}

impl<S> IntoOwnedAst<S> for MusicElement<SourceText<S>> {
    type Owned = MusicElement<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Note(value) => MusicElement::Note(value),
            Self::Rest(value) => MusicElement::Rest(value),
            Self::MultiMeasureRest(value) => MusicElement::MultiMeasureRest(value),
            Self::Chord(value) => MusicElement::Chord(value),
            Self::Bar(value) => MusicElement::Bar(value.into_owned(resolver)?),
            Self::Ending(value) => MusicElement::Ending(value),
            Self::InlineField(value) => MusicElement::InlineField(value.into_owned(resolver)?),
            Self::Grace(value) => MusicElement::Grace(value),
            Self::Decoration(value) => MusicElement::Decoration(value.into_owned(resolver)?),
            Self::Annotation(value) => MusicElement::Annotation(value.into_owned(resolver)?),
            Self::Tuplet(value) => MusicElement::Tuplet(value),
            Self::Slur(value) => MusicElement::Slur(value),
            Self::Tie(value) => MusicElement::Tie(value),
            Self::BrokenRhythm(value) => MusicElement::BrokenRhythm(value),
            Self::Overlay(value) => MusicElement::Overlay(value),
            Self::BeamBreak(value) => MusicElement::BeamBreak(value.into_owned(resolver)?),
            Self::BeamContinuation(value) => MusicElement::BeamContinuation(value),
            Self::LineBreak(value) => MusicElement::LineBreak(value),
            Self::Extension(value) => MusicElement::Extension(value.into_owned(resolver)?),
        })
    }
}

impl<S> IntoOwnedAst<S> for Line<S, SourceText<S>> {
    type Owned = Line<S, String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Blank => Line::Blank,
            Self::Comment(value) => Line::Comment(value.into_owned(resolver)?),
            Self::Directive(value) => Line::Directive(value.into_owned(resolver)?),
            Self::Field(value) => Line::Field(value.into_owned(resolver)?),
            Self::FieldContinuation(value) => Line::FieldContinuation(value.into_owned(resolver)?),
            Self::DeprecatedHistoryContinuation(value) => {
                Line::DeprecatedHistoryContinuation(value.into_owned(resolver)?)
            }
            Self::Music(values) => Line::Music(
                values
                    .into_iter()
                    .map(|value| value.into_owned(resolver))
                    .collect::<Result<_, _>>()?,
            ),
            Self::TypesetText(value) => Line::TypesetText(value.into_owned(resolver)?),
            Self::DirectiveText(value) => Line::DirectiveText(value.into_owned(resolver)?),
        })
    }
}

impl<S> IntoOwnedAst<S> for FreeText<SourceText<S>> {
    type Owned = FreeText<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(FreeText {
            lines: self
                .lines
                .into_iter()
                .map(|line| line.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl<S> IntoOwnedAst<S> for TypesetText<SourceText<S>> {
    type Owned = TypesetText<String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Text(text) => TypesetText::Text(text.into_owned(resolver)?),
            Self::Centered(text) => TypesetText::Centered(text.into_owned(resolver)?),
            Self::Block(lines) => TypesetText::Block(
                lines
                    .into_iter()
                    .map(|line| line.into_owned(resolver))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }
}

impl<S> IntoOwnedAst<S> for DocumentItem<S, SourceText<S>> {
    type Owned = DocumentItem<S, String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(match self {
            Self::Tune(tune) => DocumentItem::Tune(tune.into_owned(resolver)?),
            Self::FreeText(text) => DocumentItem::FreeText(text.into_owned(resolver)?),
            Self::TypesetText(text) => DocumentItem::TypesetText(text.into_owned(resolver)?),
            Self::Comment(text) => DocumentItem::Comment(text.into_owned(resolver)?),
            Self::Directive(value) => DocumentItem::Directive(value.into_owned(resolver)?),
        })
    }
}

impl<S> IntoOwnedAst<S> for Tune<S, SourceText<S>> {
    type Owned = Tune<S, String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Tune {
            lines: self
                .lines
                .into_iter()
                .map(|line| line.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl<S> IntoOwnedAst<S> for Document<S, SourceText<S>> {
    type Owned = Document<S, String>;

    fn into_owned<R>(self, resolver: &R) -> Result<Self::Owned, R::Error>
    where
        R: SourceResolver<S> + ?Sized,
    {
        Ok(Document {
            header: self
                .header
                .into_iter()
                .map(|line| line.into_owned(resolver))
                .collect::<Result<_, _>>()?,
            items: self
                .items
                .into_iter()
                .map(|item| item.into_owned(resolver))
                .collect::<Result<_, _>>()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_and_character_sources_resolve_their_native_span_units() {
        let utf8 = SourceText::Span(SimpleSpan::from(1..3));
        assert_eq!(utf8.into_owned("aéb").unwrap(), "é");

        let characters = ['a', 'é', 'b'];
        let character = SourceText::Span(SimpleSpan::from(1..2));
        assert_eq!(character.into_owned(characters.as_slice()).unwrap(), "é");
    }

    #[test]
    fn string_resolver_rejects_invalid_byte_spans() {
        assert_eq!(
            "é".resolve(&SimpleSpan::from(1..2)),
            Err(ResolveError::InvalidCharBoundary)
        );
        assert_eq!(
            "abc".resolve(&SimpleSpan::from(2..4)),
            Err(ResolveError::OutOfBounds)
        );
    }

    #[test]
    fn placeholder_is_obvious_and_heuristically_detectable() {
        let text = SourceText::Span(SimpleSpan::from(4..9))
            .into_owned(&PlaceholderResolver)
            .unwrap();
        assert_eq!(text, "[[ABC_SOURCE_REF:4..9]]");
        assert!(is_source_reference_placeholder(&text));
        assert!(!is_source_reference_placeholder("ordinary source text"));
    }
}
