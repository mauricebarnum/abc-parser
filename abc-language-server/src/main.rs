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

//! Language Server Protocol support for ABC music notation.

mod analysis;
mod backend;
mod config;
mod position;

use clap::Parser;
use tower_lsp_server::LspService;
use tower_lsp_server::Server;

use crate::backend::Backend;

/// Starts an ABC language server over standard input and output.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Arguments {}

#[tokio::main]
async fn main() {
    Arguments::parse();
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
