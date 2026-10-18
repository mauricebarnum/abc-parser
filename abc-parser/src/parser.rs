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

//! Chumsky parser constructors for ABC syntax.
//!
//! Document-level semantic parsers are intentionally boxed at selected
//! composition boundaries. Erasing those large concrete combinator types keeps
//! optimized-build monomorphization and code generation tractable. Token-level
//! music and field parsers remain unboxed so dynamic dispatch stays outside the
//! hottest character-by-character paths.

#![allow(clippy::redundant_pub_crate)]

mod blocks;
mod fields;
mod music;
mod primitives;

pub use blocks::Block;
pub use blocks::BlocksContext;
pub use blocks::line_parser;
pub use blocks::parse_blocks;
pub(crate) use blocks::parse_document;
pub use blocks::version_marker;
use chumsky::ParseResult;
use chumsky::error::Rich;
use chumsky::extra;
use chumsky::input::Input;
use chumsky::span::Span as ChumskySpan;
pub use fields::directive_parser;
pub use fields::field_parser;
pub use music::chord_parser;
pub use music::music_element_parser;
pub use music::music_line_parser;
pub use primitives::note_length_parser;
pub(crate) use primitives::trailing_comment;

use super::DocumentItem;
use super::Line;
use super::MusicElement;
use super::ParseError;
use super::ParseWarning;
use super::ParsedDocument;
use super::SourceText;
use super::Spanned;

pub(crate) type ParserDiagnostics<S> = (Vec<ParseError<S>>, Vec<ParseWarning<S>>);

/// Mutable diagnostics and file-wide interpretation selected while parsing.
#[derive(Default)]
pub struct ParserStateValue<S> {
    pub(crate) errors: Vec<ParseError<S>>,
    pub(crate) warnings: Vec<ParseWarning<S>>,
    pub(crate) last_field_led_span: Option<S>,
    pub(crate) body_linebreak_instruction_spans: Vec<S>,
}

impl<S> ParserStateValue<S> {
    /// Records the first-field span of the most recently resolved field-led block.
    pub(crate) fn note_field_led_start(&mut self, span: S) {
        self.last_field_led_span = Some(span);
    }

    /// Records the value span of an `I:linebreak` instruction field.
    pub(crate) fn note_linebreak_instruction(&mut self, span: S) {
        self.body_linebreak_instruction_spans.push(span);
    }
}

impl<S: ChumskySpan> ParserStateValue<S> {
    /// Returns whether `span` corresponds to a recorded `I:linebreak` instruction field.
    pub(crate) fn is_linebreak_instruction(&self, span: &S) -> bool
    where
        S::Offset: PartialEq,
    {
        self.body_linebreak_instruction_spans
            .iter()
            .any(|s| s.start() == span.start() && s.end() == span.end())
    }
}

pub(crate) type ParserState<S> = extra::SimpleState<ParserStateValue<S>>;

pub(crate) type Extra<'src, I> = extra::Full<
    Rich<'src, char, <I as Input<'src>>::Span>,
    ParserState<<I as Input<'src>>::Span>,
    (),
>;
pub(crate) type ParsedMusic<S> = Vec<Spanned<MusicElement<SourceText<S>>, S>>;
pub(crate) type ParsedLine<S> = Spanned<Line<S, SourceText<S>>, S>;
pub(crate) type ParsedDocumentItem<S> = Spanned<DocumentItem<S, SourceText<S>>, S>;
pub(crate) type DocumentParse<'src, I> = ParseResult<
    ParsedDocument<<I as Input<'src>>::Span>,
    Rich<'src, char, <I as Input<'src>>::Span>,
>;
pub(crate) type DocumentParseWithDiagnostics<'src, I> = (
    DocumentParse<'src, I>,
    ParserDiagnostics<<I as Input<'src>>::Span>,
);
