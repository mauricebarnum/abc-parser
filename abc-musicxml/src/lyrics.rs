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

//! Tokenization and note alignment for `w:` lyric and `s:` symbol lines.
//!
//! Implements the alignment rules of ABC 2.1 section 5.1: lyric tokens are
//! matched to notes in order, `*` skips a note, `|` advances to the next bar,
//! `-` splits syllables, `_` holds the previous syllable, `~` is a
//! non-breaking space, and `\-` is a literal hyphen.

use std::ops::Range;

/// Position of a syllable within a (possibly hyphenated) word.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Syllabic {
    /// The whole word is one syllable.
    Single,
    /// First syllable of a multi-syllable word.
    Begin,
    /// Interior syllable of a multi-syllable word.
    Middle,
    /// Last syllable of a multi-syllable word.
    End,
}

impl Syllabic {
    /// Returns the `MusicXML` `<syllabic>` text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Begin => "begin",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// Meaning of one `w:` token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LyricKind {
    /// Text aligned with one note.
    Syllable {
        /// Syllable text with `~` and `\-` already resolved.
        text: String,
        /// Position of the syllable within its word.
        syllabic: Syllabic,
    },
    /// `*`: one note without a lyric.
    Skip,
    /// A note inside a hyphenated word that carries no text (`syll-a--ble`).
    Hold,
    /// `_`: the previous syllable is held for one more note.
    Extend,
    /// `|`: advance to the first note of the next bar.
    Bar,
}

/// A lyric token with its absolute byte span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyricToken {
    /// Token meaning.
    pub kind: LyricKind,
    /// Byte span in the original source.
    pub span: Range<usize>,
}

/// Meaning of one `s:` token.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolKind {
    /// `*`: one note without a symbol.
    Skip,
    /// `|`: advance to the first note of the next bar.
    Bar,
    /// A decoration or quoted string aligned with one note.
    Element(String),
}

/// A symbol token with its absolute byte span.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolToken {
    /// Token meaning.
    pub kind: SymbolKind,
    /// Byte span in the original source.
    pub span: Range<usize>,
}

enum RawLyric {
    Text {
        text: String,
        hyphen_after: bool,
        span: Range<usize>,
    },
    HyphenHold(Range<usize>),
    Skip(Range<usize>),
    Extend(Range<usize>),
    Bar(Range<usize>),
}

fn flush_segment(
    segment: &mut String,
    start: &mut Option<usize>,
    end: usize,
    hyphen_after: bool,
    out: &mut Vec<RawLyric>,
) {
    if let Some(begin) = start.take()
        && !segment.is_empty()
    {
        out.push(RawLyric::Text {
            text: std::mem::take(segment),
            hyphen_after,
            span: begin..end,
        });
    }
    segment.clear();
}

fn tokenize_word(word: &str, base: usize, out: &mut Vec<RawLyric>) {
    let mut segment = String::new();
    let mut start: Option<usize> = None;
    let mut end = base;
    let mut chars = word.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        let abs = base + index;
        let width = ch.len_utf8();
        match ch {
            '\\' => {
                if let Some(&(next_index, next_ch)) = chars.peek()
                    && matches!(next_ch, '-' | '_' | '*' | '~' | '|' | '\\')
                {
                    chars.next();
                    start.get_or_insert(abs);
                    segment.push(next_ch);
                    end = base + next_index + next_ch.len_utf8();
                } else {
                    start.get_or_insert(abs);
                    segment.push('\\');
                    end = abs + width;
                }
            }
            '~' => {
                start.get_or_insert(abs);
                segment.push('\u{a0}');
                end = abs + width;
            }
            '-' => {
                if start.is_some() && !segment.is_empty() {
                    flush_segment(&mut segment, &mut start, end, true, out);
                } else {
                    out.push(RawLyric::HyphenHold(abs..abs + width));
                }
            }
            '_' => {
                flush_segment(&mut segment, &mut start, end, false, out);
                out.push(RawLyric::Extend(abs..abs + width));
            }
            '*' => {
                flush_segment(&mut segment, &mut start, end, false, out);
                out.push(RawLyric::Skip(abs..abs + width));
            }
            '|' => {
                flush_segment(&mut segment, &mut start, end, false, out);
                out.push(RawLyric::Bar(abs..abs + width));
            }
            other => {
                start.get_or_insert(abs);
                segment.push(other);
                end = abs + width;
            }
        }
    }
    flush_segment(&mut segment, &mut start, end, false, out);
}

/// Tokenizes one or more `(text, base_offset)` segments of a `w:` field.
///
/// Each segment is the text following the field marker on one physical line
/// (the `w:` line and any `+:` continuations); `base_offset` is the absolute
/// byte offset of that text, so token spans point into the original source.
#[must_use]
pub fn tokenize_lyrics<'a, I>(segments: I) -> Vec<LyricToken>
where
    I: IntoIterator<Item = (&'a str, usize)>,
{
    let mut raw: Vec<RawLyric> = Vec::new();
    for (text, base) in segments {
        let mut word_start: Option<usize> = None;
        for (index, ch) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            if ch.is_whitespace() {
                if let Some(begin) = word_start.take() {
                    tokenize_word(&text[begin..index], base + begin, &mut raw);
                }
            } else if word_start.is_none() {
                word_start = Some(index);
            }
        }
    }

    // A hold inside a word makes the preceding syllable continue the word.
    for index in 1..raw.len() {
        if matches!(raw[index], RawLyric::HyphenHold(_))
            && let RawLyric::Text { hyphen_after, .. } = &mut raw[index - 1]
        {
            *hyphen_after = true;
        }
    }

    let mut continuing = false;
    let mut tokens = Vec::with_capacity(raw.len());
    for item in raw {
        match item {
            RawLyric::Text {
                text,
                hyphen_after,
                span,
            } => {
                let syllabic = match (continuing, hyphen_after) {
                    (false, false) => Syllabic::Single,
                    (false, true) => Syllabic::Begin,
                    (true, true) => Syllabic::Middle,
                    (true, false) => Syllabic::End,
                };
                continuing = hyphen_after;
                tokens.push(LyricToken {
                    kind: LyricKind::Syllable { text, syllabic },
                    span,
                });
            }
            RawLyric::HyphenHold(span) => {
                continuing = true;
                tokens.push(LyricToken {
                    kind: LyricKind::Hold,
                    span,
                });
            }
            RawLyric::Skip(span) => {
                continuing = false;
                tokens.push(LyricToken {
                    kind: LyricKind::Skip,
                    span,
                });
            }
            RawLyric::Extend(span) => tokens.push(LyricToken {
                kind: LyricKind::Extend,
                span,
            }),
            RawLyric::Bar(span) => tokens.push(LyricToken {
                kind: LyricKind::Bar,
                span,
            }),
        }
    }
    tokens
}

/// Tokenizes one or more `(text, base_offset)` segments of an `s:` field.
#[must_use]
pub fn tokenize_symbols<'a, I>(segments: I) -> Vec<SymbolToken>
where
    I: IntoIterator<Item = (&'a str, usize)>,
{
    let mut tokens = Vec::new();
    for (text, base) in segments {
        let mut chars = text.char_indices().peekable();
        while let Some((index, ch)) = chars.next() {
            let abs = base + index;
            match ch {
                c if c.is_whitespace() => {}
                '*' => tokens.push(SymbolToken {
                    kind: SymbolKind::Skip,
                    span: abs..abs + 1,
                }),
                '|' => tokens.push(SymbolToken {
                    kind: SymbolKind::Bar,
                    span: abs..abs + 1,
                }),
                delimiter @ ('"' | '!') => {
                    let mut end = index + 1;
                    for (next_index, next) in chars.by_ref() {
                        end = next_index + next.len_utf8();
                        if next == delimiter {
                            break;
                        }
                    }
                    tokens.push(SymbolToken {
                        kind: SymbolKind::Element(text[index..end].to_owned()),
                        span: abs..base + end,
                    });
                }
                _ => {
                    let mut end = index + ch.len_utf8();
                    while let Some(&(next_index, next)) = chars.peek() {
                        if next.is_whitespace() || matches!(next, '*' | '|' | '"' | '!') {
                            break;
                        }
                        chars.next();
                        end = next_index + next.len_utf8();
                    }
                    tokens.push(SymbolToken {
                        kind: SymbolKind::Element(text[index..end].to_owned()),
                        span: abs..base + end,
                    });
                }
            }
        }
    }
    tokens
}

/// Whether an aligned token consumes a note or advances to the next bar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// The token occupies one note.
    Note,
    /// The token is a bar advance (`|`).
    Bar,
}

/// Aligns tokens to notes starting at slot `start`.
///
/// `slot_bars[i]` is the bar index of note slot `i`. Returns
/// `(slot, token_index)` pairs for every [`Step::Note`] token that found a
/// note; excess tokens are ignored, and a [`Step::Bar`] token skips the rest of
/// the current bar only if the previous note was in the same bar.
#[must_use]
pub fn align(steps: &[Step], slot_bars: &[usize], start: usize) -> Vec<(usize, usize)> {
    let mut cursor = start;
    let mut pairs = Vec::new();
    for (token_index, step) in steps.iter().enumerate() {
        match step {
            Step::Bar => {
                if cursor > 0
                    && cursor < slot_bars.len()
                    && slot_bars[cursor - 1] == slot_bars[cursor]
                {
                    let bar = slot_bars[cursor];
                    while cursor < slot_bars.len() && slot_bars[cursor] == bar {
                        cursor += 1;
                    }
                }
            }
            Step::Note => {
                if cursor >= slot_bars.len() {
                    break;
                }
                pairs.push((cursor, token_index));
                cursor += 1;
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::LyricKind;
    use super::Step;
    use super::Syllabic;
    use super::SymbolKind;
    use super::align;
    use super::tokenize_lyrics;
    use super::tokenize_symbols;

    fn kinds(text: &str) -> Vec<LyricKind> {
        tokenize_lyrics([(text, 0)])
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    fn syllable(text: &str, syllabic: Syllabic) -> LyricKind {
        LyricKind::Syllable {
            text: text.to_owned(),
            syllabic,
        }
    }

    #[test]
    fn splits_syllables_holds_and_escapes() {
        assert_eq!(
            kinds("syll-a--ble"),
            vec![
                syllable("syll", Syllabic::Begin),
                syllable("a", Syllabic::Middle),
                LyricKind::Hold,
                syllable("ble", Syllabic::End),
            ]
        );
        assert_eq!(kinds("syll-a -ble"), kinds("syll-a--ble"));
        assert_eq!(
            kinds("time__ of~the 1\\-x a\\_b c\\*d e\\|f g\\~h i\\\\j \\'e"),
            vec![
                syllable("time", Syllabic::Single),
                LyricKind::Extend,
                LyricKind::Extend,
                syllable("of\u{a0}the", Syllabic::Single),
                syllable("1-x", Syllabic::Single),
                syllable("a_b", Syllabic::Single),
                syllable("c*d", Syllabic::Single),
                syllable("e|f", Syllabic::Single),
                syllable("g~h", Syllabic::Single),
                syllable("i\\j", Syllabic::Single),
                syllable("\\'e", Syllabic::Single),
            ]
        );
        assert_eq!(
            kinds("mi- ran*|"),
            vec![
                syllable("mi", Syllabic::Begin),
                syllable("ran", Syllabic::End),
                LyricKind::Skip,
                LyricKind::Bar,
            ]
        );
    }

    #[test]
    fn tokens_carry_absolute_spans() {
        let tokens = tokenize_lyrics([("ab-c", 10)]);
        assert_eq!(tokens[0].span, 10..12);
        assert_eq!(tokens[1].span, 13..14);
    }

    #[test]
    fn tokenizes_symbols() {
        let tokens = tokenize_symbols([("\"C m\" * !f! | .", 0)]);
        let kinds: Vec<_> = tokens.iter().map(|token| token.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SymbolKind::Element("\"C m\"".to_owned()),
                SymbolKind::Skip,
                SymbolKind::Element("!f!".to_owned()),
                SymbolKind::Bar,
                SymbolKind::Element(".".to_owned()),
            ]
        );
        assert_eq!(tokens[0].span, 0..5);
    }

    #[test]
    fn bar_advance_skips_to_next_bar_only_when_mid_bar() {
        let bars = [0, 0, 1, 1];
        let steps = [Step::Note, Step::Bar, Step::Note, Step::Bar, Step::Note];
        assert_eq!(align(&steps, &bars, 0), vec![(0, 0), (2, 2)]);
        let steps = [Step::Note, Step::Note, Step::Bar, Step::Note];
        assert_eq!(align(&steps, &bars, 0), vec![(0, 0), (1, 1), (2, 3)]);
    }
}
