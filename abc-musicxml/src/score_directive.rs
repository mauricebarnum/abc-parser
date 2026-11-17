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

//! Parsing and layout flattening for `%%score` and `%%staves` directives.
//!
//! Implements voice and staff grouping per ABC 2.1 section 11.1:
//! - Parentheses `(V1 V2)` group voices on a single staff.
//! - Curly braces `{ ... }` connect staves with a brace symbol (keyboard).
//! - Brackets `[ ... ]` connect staves with a bracket symbol (orchestral).
//! - Continued bar lines `|` connect staves (reversed in `%%staves`).
//! - Floating voices `*V` attach to the preceding staff group.

use std::collections::BTreeSet;

/// The kind of score grouping directive (`%%score` or `%%staves`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScoreDirectiveKind {
    /// `%%score`: `|` indicates continued barlines between staves.
    Score,
    /// `%%staves`: lack of `|` indicates continued barlines; `|` suppresses them.
    Staves,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    OpenParen,
    CloseParen,
    Bar,
    Star,
    Ident(String),
}

fn tokenize_score_directive(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '[' => tokens.push(Token::OpenBracket),
            ']' => tokens.push(Token::CloseBracket),
            '{' => tokens.push(Token::OpenBrace),
            '}' => tokens.push(Token::CloseBrace),
            '(' => tokens.push(Token::OpenParen),
            ')' => tokens.push(Token::CloseParen),
            '|' => tokens.push(Token::Bar),
            '*' => tokens.push(Token::Star),
            '"' => {
                let mut s = String::new();
                let mut escaped = false;
                for (_, next_ch) in chars.by_ref() {
                    if escaped {
                        s.push(next_ch);
                        escaped = false;
                    } else if next_ch == '\\' {
                        escaped = true;
                    } else if next_ch == '"' {
                        break;
                    } else {
                        s.push(next_ch);
                    }
                }
                if !s.is_empty() {
                    tokens.push(Token::Ident(s));
                }
            }
            other => {
                let mut ident = String::new();
                ident.push(other);
                while let Some(&(_, next_ch)) = chars.peek() {
                    if next_ch.is_whitespace()
                        || matches!(next_ch, '[' | ']' | '{' | '}' | '(' | ')' | '|' | '*' | '"')
                    {
                        break;
                    }
                    chars.next();
                    ident.push(next_ch);
                }
                tokens.push(Token::Ident(ident));
            }
        }
    }
    tokens
}

/// A parsed element in a `%%score` or `%%staves` directive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScoreItem {
    /// A single voice on its own staff.
    Voice {
        /// Voice identifier.
        id: String,
        /// Whether marked as a floating voice (`*`).
        floating: bool,
    },
    /// Multiple voices sharing a single staff `(V1 V2 ...)`.
    StaffGroup(Vec<String>),
    /// A bracketed or braced group of staves/blocks.
    Group {
        /// Symbol: `"bracket"` or `"brace"`.
        symbol: &'static str,
        /// Inner items.
        items: Vec<Self>,
        /// Whether continued barlines connect staves in this group.
        has_barline: bool,
    },
}

fn parse_items(
    tokens: &[Token],
    cursor: &mut usize,
    kind: ScoreDirectiveKind,
    closer: Option<&Token>,
) -> (Vec<ScoreItem>, bool) {
    let mut items = Vec::new();
    let mut bar_seen = false;

    while let Some(tok) = tokens.get(*cursor) {
        if closer == Some(tok) {
            *cursor += 1;
            break;
        }

        match tok {
            Token::Bar => {
                bar_seen = true;
                *cursor += 1;
            }
            Token::OpenBracket => {
                *cursor += 1;
                let (sub_items, sub_bar) =
                    parse_items(tokens, cursor, kind, Some(&Token::CloseBracket));
                let has_barline = match kind {
                    ScoreDirectiveKind::Score => sub_bar,
                    ScoreDirectiveKind::Staves => !sub_bar,
                };
                items.push(ScoreItem::Group {
                    symbol: "bracket",
                    items: sub_items,
                    has_barline,
                });
            }
            Token::OpenBrace => {
                *cursor += 1;
                let (sub_items, sub_bar) =
                    parse_items(tokens, cursor, kind, Some(&Token::CloseBrace));
                let has_barline = match kind {
                    ScoreDirectiveKind::Score => sub_bar,
                    ScoreDirectiveKind::Staves => !sub_bar,
                };
                items.push(ScoreItem::Group {
                    symbol: "brace",
                    items: sub_items,
                    has_barline,
                });
            }
            Token::OpenParen => {
                *cursor += 1;
                let mut voices = Vec::new();
                while let Some(inner) = tokens.get(*cursor) {
                    if *inner == Token::CloseParen {
                        *cursor += 1;
                        break;
                    }
                    match inner {
                        Token::Ident(id) => {
                            voices.push(id.clone());
                            *cursor += 1;
                        }
                        Token::Star => {
                            *cursor += 1;
                            if let Some(Token::Ident(id)) = tokens.get(*cursor) {
                                voices.push(id.clone());
                                *cursor += 1;
                            }
                        }
                        _ => {
                            *cursor += 1;
                        }
                    }
                }
                if !voices.is_empty() {
                    items.push(ScoreItem::StaffGroup(voices));
                }
            }
            Token::Star => {
                *cursor += 1;
                if let Some(Token::Ident(id)) = tokens.get(*cursor) {
                    let id = id.clone();
                    *cursor += 1;
                    attach_floating_voice(&mut items, id);
                }
            }
            Token::Ident(id) => {
                let id = id.clone();
                *cursor += 1;
                items.push(ScoreItem::Voice {
                    id,
                    floating: false,
                });
            }
            Token::CloseBracket | Token::CloseBrace | Token::CloseParen => {
                *cursor += 1;
                break;
            }
        }
    }

    (items, bar_seen)
}

fn attach_floating_voice(items: &mut Vec<ScoreItem>, id: String) {
    if let Some(last) = items.last_mut() {
        match last {
            ScoreItem::Voice { id: prev_id, .. } => {
                let prev = prev_id.clone();
                *last = ScoreItem::StaffGroup(vec![prev, id]);
            }
            ScoreItem::StaffGroup(voices) => {
                voices.push(id);
            }
            ScoreItem::Group { items: sub, .. } => {
                attach_floating_voice(sub, id);
            }
        }
    } else {
        items.push(ScoreItem::Voice { id, floating: true });
    }
}

/// Specification for one staff layer inside a `<part>` in `MusicXML`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaffLayerSpec {
    /// Ordered ABC voices that render on this staff layer.
    pub voice_ids: Vec<String>,
}

/// Specification for one staff / `<part>` in `MusicXML`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaffSpec {
    /// Generated `<part id="...">` identifier (e.g. `"P1"`).
    pub part_id: String,
    /// Ordered ABC voices that render on this staff (or all voices if multi-staff).
    pub voice_ids: Vec<String>,
    /// Staff layers for keyboard / grand-staff parts (empty for single-staff parts).
    pub staves: Vec<StaffLayerSpec>,
}

/// An entry in `<part-list>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartListEntry {
    /// Opening tag of `<part-group>`.
    PartGroupStart {
        /// Distinguishing group number.
        number: u32,
        /// Visual bracket or brace symbol.
        symbol: Option<&'static str>,
        /// Whether continued barlines connect staves in this group.
        barline: bool,
    },
    /// Index into `ScoreLayout::staves`.
    ScorePart(usize),
    /// Closing tag of `<part-group>`.
    PartGroupStop {
        /// Distinguishing group number.
        number: u32,
    },
}

/// Flattened layout ready for `MusicXML` `<part-list>` and part generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreLayout {
    /// Ordered staves to generate as `<part>` elements.
    pub staves: Vec<StaffSpec>,
    /// Sequential entries for `<part-list>` serialization.
    pub entries: Vec<PartListEntry>,
}

impl ScoreLayout {
    /// Returns a set of all voice identifiers mentioned in this score layout.
    #[must_use]
    pub fn all_voice_ids(&self) -> BTreeSet<&str> {
        self.staves
            .iter()
            .flat_map(|s| s.voice_ids.iter().map(String::as_str))
            .collect()
    }
}

fn flatten_score_items(
    items: &[ScoreItem],
    staves: &mut Vec<StaffSpec>,
    entries: &mut Vec<PartListEntry>,
    next_group_number: &mut u32,
) {
    for item in items {
        match item {
            ScoreItem::Voice { id, .. } => {
                let idx = staves.len();
                staves.push(StaffSpec {
                    part_id: format!("P{}", idx + 1),
                    voice_ids: vec![id.clone()],
                    staves: Vec::new(),
                });
                entries.push(PartListEntry::ScorePart(idx));
            }
            ScoreItem::StaffGroup(voices) => {
                let idx = staves.len();
                staves.push(StaffSpec {
                    part_id: format!("P{}", idx + 1),
                    voice_ids: voices.clone(),
                    staves: Vec::new(),
                });
                entries.push(PartListEntry::ScorePart(idx));
            }
            ScoreItem::Group {
                symbol,
                items: sub_items,
                has_barline,
            } => {
                if *symbol == "brace" {
                    let mut staff_layers = Vec::new();
                    let mut all_voices = Vec::new();
                    let mut can_merge_to_multi_staff = !sub_items.is_empty();

                    for sub in sub_items {
                        match sub {
                            ScoreItem::Voice { id, .. } => {
                                staff_layers.push(StaffLayerSpec {
                                    voice_ids: vec![id.clone()],
                                });
                                all_voices.push(id.clone());
                            }
                            ScoreItem::StaffGroup(voices) => {
                                staff_layers.push(StaffLayerSpec {
                                    voice_ids: voices.clone(),
                                });
                                all_voices.extend(voices.clone());
                            }
                            ScoreItem::Group { .. } => {
                                can_merge_to_multi_staff = false;
                                break;
                            }
                        }
                    }

                    if can_merge_to_multi_staff && staff_layers.len() > 1 {
                        let idx = staves.len();
                        staves.push(StaffSpec {
                            part_id: format!("P{}", idx + 1),
                            voice_ids: all_voices,
                            staves: staff_layers,
                        });
                        entries.push(PartListEntry::ScorePart(idx));
                        continue;
                    }
                }

                let group_num = *next_group_number;
                *next_group_number += 1;
                entries.push(PartListEntry::PartGroupStart {
                    number: group_num,
                    symbol: Some(symbol),
                    barline: *has_barline,
                });
                flatten_score_items(sub_items, staves, entries, next_group_number);
                entries.push(PartListEntry::PartGroupStop { number: group_num });
            }
        }
    }
}

/// Parses a `%%score` or `%%staves` directive into a flattened [`ScoreLayout`].
#[must_use]
pub fn parse_score_directive(kind: ScoreDirectiveKind, arguments: &str) -> ScoreLayout {
    let tokens = tokenize_score_directive(arguments);
    let mut cursor = 0;
    let (items, _) = parse_items(&tokens, &mut cursor, kind, None);
    let mut staves = Vec::new();
    let mut entries = Vec::new();
    let mut next_group_number = 1;
    flatten_score_items(&items, &mut staves, &mut entries, &mut next_group_number);
    ScoreLayout { staves, entries }
}

#[cfg(test)]
mod tests {
    use super::PartListEntry;
    use super::ScoreDirectiveKind;
    use super::parse_score_directive;

    #[test]
    fn parses_bracketed_and_braced_groups_with_barlines() {
        let layout = parse_score_directive(
            ScoreDirectiveKind::Score,
            "Solo [(S A) (T B)] {RH | (LH1 LH2)}",
        );
        assert_eq!(layout.staves.len(), 4);
        assert_eq!(layout.staves[0].voice_ids, vec!["Solo"]);
        assert_eq!(layout.staves[1].voice_ids, vec!["S", "A"]);
        assert_eq!(layout.staves[2].voice_ids, vec!["T", "B"]);
        assert_eq!(layout.staves[3].voice_ids, vec!["RH", "LH1", "LH2"]);
        assert_eq!(layout.staves[3].staves.len(), 2);
        assert_eq!(layout.staves[3].staves[0].voice_ids, vec!["RH"]);
        assert_eq!(layout.staves[3].staves[1].voice_ids, vec!["LH1", "LH2"]);

        assert_eq!(
            layout.entries,
            vec![
                PartListEntry::ScorePart(0),
                PartListEntry::PartGroupStart {
                    number: 1,
                    symbol: Some("bracket"),
                    barline: false,
                },
                PartListEntry::ScorePart(1),
                PartListEntry::ScorePart(2),
                PartListEntry::PartGroupStop { number: 1 },
                PartListEntry::ScorePart(3),
            ]
        );
    }

    #[test]
    fn parses_nested_groups() {
        let layout =
            parse_score_directive(ScoreDirectiveKind::Score, "[{Vln1 | Vln2} | Vla | Vc | DB]");
        assert_eq!(layout.staves.len(), 4);
        assert_eq!(layout.staves[0].staves.len(), 2);
        assert_eq!(
            layout.entries,
            vec![
                PartListEntry::PartGroupStart {
                    number: 1,
                    symbol: Some("bracket"),
                    barline: true,
                },
                PartListEntry::ScorePart(0),
                PartListEntry::ScorePart(1),
                PartListEntry::ScorePart(2),
                PartListEntry::ScorePart(3),
                PartListEntry::PartGroupStop { number: 1 },
            ]
        );
    }

    #[test]
    fn parses_floating_voices_attached_to_preceding_staff() {
        let layout = parse_score_directive(ScoreDirectiveKind::Score, "{RH *M| LH}");
        assert_eq!(layout.staves.len(), 1);
        assert_eq!(layout.staves[0].voice_ids, vec!["RH", "M", "LH"]);
        assert_eq!(layout.staves[0].staves.len(), 2);
        assert_eq!(layout.staves[0].staves[0].voice_ids, vec!["RH", "M"]);
        assert_eq!(layout.staves[0].staves[1].voice_ids, vec!["LH"]);

        let layout2 = parse_score_directive(ScoreDirectiveKind::Score, "{(RH1 RH2) *M| (LH1 LH2)}");
        assert_eq!(layout2.staves.len(), 1);
        assert_eq!(
            layout2.staves[0].voice_ids,
            vec!["RH1", "RH2", "M", "LH1", "LH2"]
        );
        assert_eq!(layout2.staves[0].staves.len(), 2);
        assert_eq!(
            layout2.staves[0].staves[0].voice_ids,
            vec!["RH1", "RH2", "M"]
        );
        assert_eq!(layout2.staves[0].staves[1].voice_ids, vec!["LH1", "LH2"]);
    }

    #[test]
    fn staves_directive_inverts_barline_rules() {
        let score_no_bar = parse_score_directive(ScoreDirectiveKind::Score, "[1 2]");
        let staves_no_bar = parse_score_directive(ScoreDirectiveKind::Staves, "[1 2]");
        assert_eq!(
            score_no_bar.entries[0],
            PartListEntry::PartGroupStart {
                number: 1,
                symbol: Some("bracket"),
                barline: false,
            }
        );
        assert_eq!(
            staves_no_bar.entries[0],
            PartListEntry::PartGroupStart {
                number: 1,
                symbol: Some("bracket"),
                barline: true,
            }
        );

        let score_with_bar = parse_score_directive(ScoreDirectiveKind::Score, "[1 | 2]");
        let staves_with_bar = parse_score_directive(ScoreDirectiveKind::Staves, "[1 | 2]");
        assert_eq!(
            score_with_bar.entries[0],
            PartListEntry::PartGroupStart {
                number: 1,
                symbol: Some("bracket"),
                barline: true,
            }
        );
        assert_eq!(
            staves_with_bar.entries[0],
            PartListEntry::PartGroupStart {
                number: 1,
                symbol: Some("bracket"),
                barline: false,
            }
        );
    }

    #[test]
    fn parses_quoted_identifiers_with_escaped_quotes() {
        let layout = parse_score_directive(
            ScoreDirectiveKind::Score,
            r#"("Soprano \"1\"" "Soprano \"2\"")"#,
        );
        assert_eq!(
            layout.staves[0].voice_ids,
            vec![r#"Soprano "1""#, r#"Soprano "2""#]
        );
    }
}
