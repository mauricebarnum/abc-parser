// Copyright 2026 Maurice S. Barnum
// SPDX-License-Identifier: Apache-2.0

//! LSP lifecycle, synchronized documents, and request dispatch.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tower_lsp_server::Client;
use tower_lsp_server::LanguageServer;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::CodeAction;
use tower_lsp_server::ls_types::CodeActionKind;
use tower_lsp_server::ls_types::CodeActionOptions;
use tower_lsp_server::ls_types::CodeActionOrCommand;
use tower_lsp_server::ls_types::CodeActionParams;
use tower_lsp_server::ls_types::CodeActionResponse;
use tower_lsp_server::ls_types::CompletionOptions;
use tower_lsp_server::ls_types::CompletionParams;
use tower_lsp_server::ls_types::CompletionResponse;
use tower_lsp_server::ls_types::ConfigurationItem;
use tower_lsp_server::ls_types::DidChangeConfigurationParams;
use tower_lsp_server::ls_types::DidChangeTextDocumentParams;
use tower_lsp_server::ls_types::DidCloseTextDocumentParams;
use tower_lsp_server::ls_types::DidOpenTextDocumentParams;
use tower_lsp_server::ls_types::DocumentFormattingParams;
use tower_lsp_server::ls_types::DocumentRangeFormattingParams;
use tower_lsp_server::ls_types::DocumentSymbolParams;
use tower_lsp_server::ls_types::DocumentSymbolResponse;
use tower_lsp_server::ls_types::FoldingRange;
use tower_lsp_server::ls_types::FoldingRangeParams;
use tower_lsp_server::ls_types::FormattingOptions;
use tower_lsp_server::ls_types::Hover;
use tower_lsp_server::ls_types::HoverContents;
use tower_lsp_server::ls_types::HoverParams;
use tower_lsp_server::ls_types::HoverProviderCapability;
use tower_lsp_server::ls_types::InitializeParams;
use tower_lsp_server::ls_types::InitializeResult;
use tower_lsp_server::ls_types::InitializedParams;
use tower_lsp_server::ls_types::Location;
use tower_lsp_server::ls_types::MarkupContent;
use tower_lsp_server::ls_types::MarkupKind;
use tower_lsp_server::ls_types::MessageType;
use tower_lsp_server::ls_types::OneOf;
use tower_lsp_server::ls_types::PositionEncodingKind;
use tower_lsp_server::ls_types::Range as LspRange;
use tower_lsp_server::ls_types::SelectionRange;
use tower_lsp_server::ls_types::SelectionRangeParams;
use tower_lsp_server::ls_types::SelectionRangeProviderCapability;
use tower_lsp_server::ls_types::SemanticTokenModifier;
use tower_lsp_server::ls_types::SemanticTokenType;
use tower_lsp_server::ls_types::SemanticTokensFullOptions;
use tower_lsp_server::ls_types::SemanticTokensLegend;
use tower_lsp_server::ls_types::SemanticTokensOptions;
use tower_lsp_server::ls_types::SemanticTokensParams;
use tower_lsp_server::ls_types::SemanticTokensRangeParams;
use tower_lsp_server::ls_types::SemanticTokensRangeResult;
use tower_lsp_server::ls_types::SemanticTokensResult;
use tower_lsp_server::ls_types::ServerCapabilities;
use tower_lsp_server::ls_types::ServerInfo;
use tower_lsp_server::ls_types::SymbolInformation;
use tower_lsp_server::ls_types::TextDocumentSyncKind;
use tower_lsp_server::ls_types::TextEdit;
use tower_lsp_server::ls_types::Uri;
use tower_lsp_server::ls_types::WorkspaceEdit;

use crate::analysis;
use crate::analysis::Analysis;
use crate::config::Config;
use crate::config::NoteLengthStyle;
use crate::position::LineIndex;

/// One immutable synchronized document version.
#[derive(Clone, Debug)]
struct DocumentState {
    version: i32,
    index: LineIndex,
    analysis: Analysis,
    config: Config,
}

impl DocumentState {
    fn new(text: String, version: i32, encoding: &PositionEncodingKind, config: Config) -> Self {
        let index = LineIndex::new(text);
        let analysis = Analysis::new(&index, encoding, config);
        Self {
            version,
            index,
            analysis,
            config,
        }
    }
}

async fn analyze_document(
    text: String,
    version: i32,
    encoding: PositionEncodingKind,
    config: Config,
) -> Arc<DocumentState> {
    tokio::task::spawn_blocking(move || {
        Arc::new(DocumentState::new(text, version, &encoding, config))
    })
    .await
    .expect("ABC analysis task should not panic")
}

/// Mutable server-wide state protected independently of request execution.
#[derive(Debug)]
struct State {
    documents: HashMap<Uri, Arc<DocumentState>>,
    encoding: PositionEncodingKind,
    config: Config,
    supports_configuration: bool,
    supports_hierarchical_symbols: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            documents: HashMap::new(),
            encoding: PositionEncodingKind::UTF16,
            config: Config::default(),
            supports_configuration: false,
            supports_hierarchical_symbols: false,
        }
    }
}

/// ABC language server implementation.
#[derive(Debug)]
pub struct Backend {
    client: Client,
    state: RwLock<State>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            state: RwLock::new(State::default()),
        }
    }

    async fn document(&self, uri: &Uri) -> Option<(Arc<DocumentState>, PositionEncodingKind)> {
        let state = self.state.read().await;
        Some((
            Arc::clone(state.documents.get(uri)?),
            state.encoding.clone(),
        ))
    }

    async fn publish(&self, uri: Uri, document: &DocumentState) {
        self.client
            .publish_diagnostics(
                uri,
                document.analysis.diagnostics.clone(),
                Some(document.version),
            )
            .await;
    }

    async fn config_for_uri(&self, uri: &Uri) -> Config {
        let (fallback, supported) = {
            let state = self.state.read().await;
            (state.config, state.supports_configuration)
        };
        if !supported {
            return fallback;
        }
        let items = vec![ConfigurationItem {
            scope_uri: Some(uri.clone()),
            section: Some("abc".to_owned()),
        }];
        self.client
            .configuration(items)
            .await
            .ok()
            .and_then(|values| values.first().and_then(config_from_value))
            .unwrap_or(fallback)
    }

    async fn replace_config(&self, config: Config) {
        let republished = {
            let mut state = self.state.write().await;
            state.config = config;
            let encoding = state.encoding.clone();
            let documents = state
                .documents
                .iter()
                .map(|(uri, document)| {
                    let replacement = Arc::new(DocumentState::new(
                        document.index.source().to_owned(),
                        document.version,
                        &encoding,
                        config,
                    ));
                    (uri.clone(), replacement)
                })
                .collect::<Vec<_>>();
            for (uri, document) in &documents {
                state.documents.insert(uri.clone(), Arc::clone(document));
            }
            documents
        };
        for (uri, document) in republished {
            self.publish(uri, &document).await;
        }
    }

    async fn refresh_configuration(&self) {
        let (fallback, encoding, snapshots) = {
            let state = self.state.read().await;
            (
                state.config,
                state.encoding.clone(),
                state
                    .documents
                    .iter()
                    .map(|(uri, document)| {
                        (
                            uri.clone(),
                            document.version,
                            document.index.source().to_owned(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let mut items = vec![ConfigurationItem {
            scope_uri: None,
            section: Some("abc".to_owned()),
        }];
        items.extend(snapshots.iter().map(|(uri, _, _)| ConfigurationItem {
            scope_uri: Some(uri.clone()),
            section: Some("abc".to_owned()),
        }));
        let Ok(values) = self.client.configuration(items).await else {
            return;
        };
        let config = values
            .first()
            .and_then(config_from_value)
            .unwrap_or(fallback);
        let replacements = snapshots
            .into_iter()
            .enumerate()
            .map(|(index, (uri, version, source))| {
                let document_config = values
                    .get(index + 1)
                    .and_then(config_from_value)
                    .unwrap_or(config);
                (
                    uri,
                    Arc::new(DocumentState::new(
                        source,
                        version,
                        &encoding,
                        document_config,
                    )),
                )
            })
            .collect::<Vec<_>>();
        let republished = {
            let mut state = self.state.write().await;
            state.config = config;
            let mut republished = Vec::new();
            for (uri, replacement) in replacements {
                if state
                    .documents
                    .get(&uri)
                    .is_some_and(|current| current.version == replacement.version)
                {
                    state
                        .documents
                        .insert(uri.clone(), Arc::clone(&replacement));
                    republished.push((uri, replacement));
                }
            }
            drop(state);
            republished
        };
        for (uri, document) in republished {
            self.publish(uri, &document).await;
        }
    }

    async fn formatting_edits(
        &self,
        uri: &Uri,
        range: Option<LspRange>,
        options: &FormattingOptions,
    ) -> Option<Vec<TextEdit>> {
        let (document, encoding) = self.document(uri).await?;
        if document.analysis.has_errors {
            return Some(Vec::new());
        }
        let style = self.state.read().await.config.format.note_length;
        let scope = range
            .and_then(|range| document.index.byte_range(range, &encoding))
            .unwrap_or_else(|| 0..document.index.source().len());
        let mut edits = analysis::duration_edits(&document.index, &encoding, style, scope);
        if range.is_none() {
            edits.extend(whitespace_edits(&document.index, &encoding, options));
        }
        Some(edits)
    }
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let offered = params
            .capabilities
            .general
            .as_ref()
            .and_then(|general| general.position_encodings.as_ref());
        let encoding = offered
            .and_then(|encodings| {
                encodings
                    .iter()
                    .find(|encoding| {
                        **encoding == PositionEncodingKind::UTF8
                            || **encoding == PositionEncodingKind::UTF16
                    })
                    .cloned()
            })
            .unwrap_or(PositionEncodingKind::UTF16);
        let config = params
            .initialization_options
            .as_ref()
            .and_then(config_from_value)
            .unwrap_or_default();
        let supports_configuration = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.configuration)
            .unwrap_or(false);
        let supports_hierarchical_symbols = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text_document| text_document.document_symbol.as_ref())
            .and_then(|symbols| symbols.hierarchical_document_symbol_support)
            .unwrap_or(false);
        {
            let mut state = self.state.write().await;
            state.encoding = encoding.clone();
            state.config = config;
            state.supports_configuration = supports_configuration;
            state.supports_hierarchical_symbols = supports_hierarchical_symbols;
        }
        let semantic_options = SemanticTokensOptions {
            legend: SemanticTokensLegend {
                token_types: vec![
                    SemanticTokenType::COMMENT,
                    SemanticTokenType::KEYWORD,
                    SemanticTokenType::NUMBER,
                    SemanticTokenType::STRING,
                    SemanticTokenType::ENUM_MEMBER,
                    SemanticTokenType::OPERATOR,
                    SemanticTokenType::DECORATOR,
                    SemanticTokenType::VARIABLE,
                ],
                token_modifiers: vec![
                    SemanticTokenModifier::DECLARATION,
                    SemanticTokenModifier::DEPRECATED,
                ],
            },
            range: Some(true),
            full: Some(SemanticTokensFullOptions::Bool(true)),
            ..SemanticTokensOptions::default()
        };
        let capabilities = ServerCapabilities {
            position_encoding: Some(encoding),
            text_document_sync: Some(TextDocumentSyncKind::FULL.into()),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            completion_provider: Some(CompletionOptions {
                trigger_characters: Some(vec!["%".to_owned(), ":".to_owned(), "[".to_owned()]),
                ..CompletionOptions::default()
            }),
            document_symbol_provider: Some(OneOf::Left(true)),
            folding_range_provider: Some(true.into()),
            selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
            semantic_tokens_provider: Some(semantic_options.into()),
            document_formatting_provider: Some(OneOf::Left(true)),
            document_range_formatting_provider: Some(OneOf::Left(true)),
            code_action_provider: Some(
                CodeActionOptions {
                    code_action_kinds: Some(vec![CodeActionKind::REFACTOR_REWRITE]),
                    resolve_provider: Some(false),
                    ..CodeActionOptions::default()
                }
                .into(),
            ),
            ..ServerCapabilities::default()
        };
        Ok(InitializeResult {
            capabilities,
            server_info: Some(ServerInfo {
                name: "abc-language-server".to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            offset_encoding: None,
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        if !self.state.read().await.supports_configuration {
            return;
        }
        self.refresh_configuration().await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let item = params.text_document;
        let config = self.config_for_uri(&item.uri).await;
        let encoding = self.state.read().await.encoding.clone();
        let document = analyze_document(item.text, item.version, encoding, config).await;
        {
            let mut state = self.state.write().await;
            state
                .documents
                .insert(item.uri.clone(), Arc::clone(&document));
        }
        self.publish(item.uri, &document).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        let (encoding, config) = {
            let state = self.state.read().await;
            if state
                .documents
                .get(&uri)
                .is_some_and(|current| current.version >= version)
            {
                return;
            }
            let config = state
                .documents
                .get(&uri)
                .map_or(state.config, |current| current.config);
            (state.encoding.clone(), config)
        };
        let document = analyze_document(change.text, version, encoding, config).await;
        {
            let mut state = self.state.write().await;
            if state
                .documents
                .get(&uri)
                .is_some_and(|current| current.version >= version)
            {
                return;
            }
            state.documents.insert(uri.clone(), Arc::clone(&document));
        }
        self.publish(uri, &document).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.state.write().await.documents.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        if self.state.read().await.supports_configuration {
            self.refresh_configuration().await;
        } else if let Some(config) = config_from_value(&params.settings) {
            self.replace_config(config).await;
        } else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    "invalid ABC language-server configuration",
                )
                .await;
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let position = params.text_document_position_params.position;
        let uri = params.text_document_position_params.text_document.uri;
        let Some((document, encoding)) = self.document(&uri).await else {
            return Ok(None);
        };
        let Some(offset) = document
            .index
            .byte_range(LspRange::new(position, position), &encoding)
            .map(|range| range.start)
        else {
            return Ok(None);
        };
        Ok(
            analysis::hover(&document.index, offset).and_then(|(range, value)| {
                Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range: Some(document.index.lsp_range(range, &encoding)?),
                })
            }),
        )
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position.position;
        let uri = params.text_document_position.text_document.uri;
        let Some((document, encoding)) = self.document(&uri).await else {
            return Ok(None);
        };
        let Some(offset) = document
            .index
            .byte_range(LspRange::new(position, position), &encoding)
            .map(|range| range.start)
        else {
            return Ok(None);
        };
        Ok(Some(CompletionResponse::Array(analysis::completions(
            &document.index,
            &encoding,
            offset,
        ))))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let Some((document, encoding)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        let symbols = analysis::document_symbols(&document.index, &encoding);
        if self.state.read().await.supports_hierarchical_symbols {
            return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
        }
        let flat =
            symbols
                .into_iter()
                .flat_map(|symbol| {
                    let tune_name = symbol.name.clone();
                    let mut flattened = vec![symbol_information(
                        symbol.name,
                        symbol.kind,
                        symbol.selection_range,
                        params.text_document.uri.clone(),
                        None,
                    )];
                    flattened.extend(symbol.children.unwrap_or_default().into_iter().map(
                        |child| {
                            symbol_information(
                                child.name,
                                child.kind,
                                child.selection_range,
                                params.text_document.uri.clone(),
                                Some(tune_name.clone()),
                            )
                        },
                    ));
                    flattened
                })
                .collect();
        Ok(Some(DocumentSymbolResponse::Flat(flat)))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let Some((document, _)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        Ok(Some(analysis::folding_ranges(&document.index)))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let Some((document, encoding)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        Ok(Some(
            params
                .positions
                .into_iter()
                .filter_map(|position| {
                    let offset = document
                        .index
                        .byte_range(LspRange::new(position, position), &encoding)?
                        .start;
                    analysis::selection_range(&document.index, &encoding, offset)
                })
                .collect(),
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let Some((document, encoding)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        Ok(Some(
            analysis::semantic_tokens(&document.index, &encoding, None).into(),
        ))
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let Some((document, encoding)) = self.document(&params.text_document.uri).await else {
            return Ok(None);
        };
        Ok(Some(
            analysis::semantic_tokens(&document.index, &encoding, Some(params.range)).into(),
        ))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        Ok(self
            .formatting_edits(&params.text_document.uri, None, &params.options)
            .await)
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        Ok(self
            .formatting_edits(
                &params.text_document.uri,
                Some(params.range),
                &params.options,
            )
            .await)
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let Some((document, encoding)) = self.document(&uri).await else {
            return Ok(None);
        };
        if document.analysis.has_errors {
            return Ok(Some(Vec::new()));
        }
        let Some(scope) = document.index.byte_range(params.range, &encoding) else {
            return Ok(Some(Vec::new()));
        };
        let scope = if scope.is_empty() {
            document.index.line_bounds(scope.start).unwrap_or(scope)
        } else {
            scope
        };
        let actions = [
            (
                "Use shorthand note lengths",
                "refactor.rewrite.abc.noteLength.shorthand",
                NoteLengthStyle::Shorthand,
            ),
            (
                "Use explicit note-length denominators",
                "refactor.rewrite.abc.noteLength.explicit",
                NoteLengthStyle::Explicit,
            ),
        ]
        .into_iter()
        .filter_map(|(title, kind, style)| {
            let edits = analysis::duration_edits(&document.index, &encoding, style, scope.clone());
            (!edits.is_empty()).then(|| {
                let mut changes = HashMap::new();
                changes.insert(uri.clone(), edits);
                CodeActionOrCommand::CodeAction(CodeAction {
                    title: title.to_owned(),
                    kind: Some(CodeActionKind::new(kind)),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..WorkspaceEdit::default()
                    }),
                    ..CodeAction::default()
                })
            })
        })
        .collect();
        Ok(Some(actions))
    }
}

fn config_from_value(value: &serde_json::Value) -> Option<Config> {
    let value = value.get("abc").unwrap_or(value);
    serde_json::from_value(value.clone()).ok()
}

#[allow(deprecated)]
const fn symbol_information(
    name: String,
    kind: tower_lsp_server::ls_types::SymbolKind,
    range: LspRange,
    uri: Uri,
    container_name: Option<String>,
) -> SymbolInformation {
    SymbolInformation {
        name,
        kind,
        tags: None,
        deprecated: None,
        location: Location::new(uri, range),
        container_name,
    }
}

fn whitespace_edits(
    index: &LineIndex,
    encoding: &PositionEncodingKind,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    let source = index.source();
    let mut edits = Vec::new();
    if options.trim_trailing_whitespace == Some(true) {
        let mut base = 0;
        for line in source.split_inclusive('\n') {
            let content = line.trim_end_matches(['\r', '\n']);
            let trimmed = content.trim_end_matches([' ', '\t']);
            if trimmed.len() < content.len() {
                edits.push(TextEdit::new(
                    index
                        .lsp_range(base + trimmed.len()..base + content.len(), encoding)
                        .expect("ASCII whitespace is on character boundaries"),
                    String::new(),
                ));
            }
            base += line.len();
        }
    }
    if options.insert_final_newline == Some(true) && !source.ends_with('\n') {
        edits.push(TextEdit::new(
            index
                .lsp_range(source.len()..source.len(), encoding)
                .expect("end of source is a character boundary"),
            "\n".to_owned(),
        ));
    }
    if options.trim_final_newlines == Some(true) {
        let mut start = source.len();
        let mut count = 0;
        while start > 0 {
            if source.as_bytes()[start - 1] == b'\n' {
                start -= 1;
                if start > 0 && source.as_bytes()[start - 1] == b'\r' {
                    start -= 1;
                }
                count += 1;
            } else if source.as_bytes()[start - 1] == b'\r' {
                start -= 1;
                count += 1;
            } else {
                break;
            }
        }
        if count > 1 {
            let keep = if source[start..].starts_with("\r\n") {
                start + 2
            } else {
                start + 1
            };
            edits.push(TextEdit::new(
                index
                    .lsp_range(keep..source.len(), encoding)
                    .expect("line endings are on character boundaries"),
                String::new(),
            ));
        }
    }
    edits
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use serde_json::json;
    use tower::Service;
    use tower::ServiceExt;
    use tower_lsp_server::LspService;
    use tower_lsp_server::jsonrpc::Request;
    use tower_lsp_server::ls_types::Position;

    use super::*;

    #[tokio::test]
    async fn negotiates_encoding_and_publishes_versioned_diagnostics() {
        let (mut service, mut client) = LspService::new(Backend::new);
        let initialize = Request::build("initialize")
            .params(json!({
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] }
                }
            }))
            .id(1)
            .finish();
        let response = service
            .ready()
            .await
            .expect("service should be ready")
            .call(initialize)
            .await
            .expect("initialize should be handled")
            .expect("initialize has a response");
        assert_eq!(
            response.result().and_then(|result| {
                result
                    .pointer("/capabilities/positionEncoding")
                    .and_then(serde_json::Value::as_str)
            }),
            Some("utf-8")
        );

        let open = Request::build("textDocument/didOpen")
            .params(json!({
                "textDocument": {
                    "uri": "file:///tmp/protocol-test.abc",
                    "languageId": "abc",
                    "version": 7,
                    "text": "X:1\nK:C\n[\n"
                }
            }))
            .finish();
        let response = service
            .ready()
            .await
            .expect("service should remain ready")
            .call(open)
            .await
            .expect("open should be handled");
        assert!(response.is_none());

        let diagnostics = client
            .next()
            .await
            .expect("diagnostics should be published");
        assert_eq!(diagnostics.method(), "textDocument/publishDiagnostics");
        let params = diagnostics.params().expect("diagnostics have parameters");
        assert_eq!(
            params.get("version").and_then(serde_json::Value::as_i64),
            Some(7)
        );
        assert!(
            params
                .get("diagnostics")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|diagnostics| !diagnostics.is_empty())
        );
    }

    #[test]
    fn accepts_wrapped_and_unwrapped_configuration() {
        let expected = Config::default();
        assert_eq!(config_from_value(&json!({})), Some(expected));
        assert_eq!(config_from_value(&json!({ "abc": {} })), Some(expected));
        assert_eq!(
            config_from_value(&json!({ "format": { "noteLength": "unknown" } })),
            None
        );
    }

    #[test]
    fn trimming_final_newlines_preserves_one_complete_line_ending() {
        let index = LineIndex::new("X:1\r\n\r\n".to_owned());
        let options = FormattingOptions {
            trim_final_newlines: Some(true),
            ..FormattingOptions::default()
        };
        let edits = whitespace_edits(&index, &PositionEncodingKind::UTF16, &options);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(1, 0));
        assert_eq!(edits[0].range.end, Position::new(2, 0));
        assert!(edits[0].new_text.is_empty());
    }
}
