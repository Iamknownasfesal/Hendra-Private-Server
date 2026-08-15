//! A language server for the behaviour language.
//!
//! The content is 20,000 lines of behaviour across sixty-one files, and the thing an author needs
//! most is not compilation but orientation: what does this behaviour do with its arguments, which
//! file holds the enemy this one spawns, where does this transition land. That is what this
//! answers, over the same lexer the compiler uses so the two never disagree about what a file says.
//!
//! It speaks the Language Server Protocol over stdin and stdout, which is what an editor expects,
//! and does its own framing rather than taking a dependency on an async runtime to move a few
//! kilobytes of JSON between two pipes.

mod cursor;
mod docs;
mod index;
mod server;
mod source;

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::server::Server;

fn main() {
    let mut arguments = std::env::args().skip(1);
    if let Some(flag) = arguments.next() {
        match flag.as_str() {
            "--check" => {
                let root = arguments.next().unwrap_or_else(|| ".".to_string());
                std::process::exit(check(&root));
            }
            "--version" => {
                println!("beh-lsp {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!(
                    "beh-lsp speaks the language server protocol on stdin and stdout.\n\
                     `--check <directory>` reports the same findings on the command line.\n\
                     `{other}` is neither."
                );
                std::process::exit(2);
            }
        }
    }

    serve();
}

/// Reports on every file under a directory, the way the editor would.
///
/// The same findings as the editor shows, in a form a build can fail on: an argument written under
/// a name nothing reads is invisible everywhere else.
fn check(root: &str) -> i32 {
    let mut server = Server::new();
    server.workspace.scan(std::path::Path::new(root));

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut notes = 0usize;

    let uris: Vec<String> = server.workspace.uris().map(|uri| uri.to_string()).collect();

    for uri in &uris {
        let name = server
            .workspace
            .get(uri)
            .map(|file| file.path.display().to_string())
            .unwrap_or_default();

        for finding in server.diagnostics(uri).as_array().into_iter().flatten() {
            let line = finding["range"]["start"]["line"].as_u64().unwrap_or(0) + 1;
            let column = finding["range"]["start"]["character"].as_u64().unwrap_or(0) + 1;
            let severity = finding["severity"].as_u64().unwrap_or(2);
            let message = finding["message"].as_str().unwrap_or_default();

            let label = match severity {
                1 => {
                    errors += 1;
                    "error"
                }
                2 => {
                    warnings += 1;
                    "warning"
                }
                // What the runtime has no feature for, which is worth knowing and not worth fixing.
                _ => {
                    notes += 1;
                    "note"
                }
            };
            println!("{name}:{line}:{column}: {label}: {message}");
        }
    }

    println!(
        "\n{} file{}, {errors} error{}, {warnings} warning{}, {notes} note{}",
        uris.len(),
        plural(uris.len()),
        plural(errors),
        plural(warnings),
        plural(notes),
    );

    i32::from(errors > 0)
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn serve() {
    let mut server = Server::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();

    while let Some(message) = read(&mut input) {
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

        // Notifications, which are answered with work rather than with a reply.
        match method.as_str() {
            "initialized" | "$/cancelRequest" | "workspace/didChangeConfiguration" => continue,
            "exit" => return,
            "textDocument/didOpen" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
                let text = params["textDocument"]["text"].as_str().unwrap_or_default();
                server.open(uri, text.to_string());
                publish(&mut output, uri, server.diagnostics(uri));
                continue;
            }
            "textDocument/didChange" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
                // The whole document arrives each time, which is what the server asks for.
                if let Some(text) = params["contentChanges"]
                    .as_array()
                    .and_then(|changes| changes.last())
                    .and_then(|change| change["text"].as_str())
                {
                    server.open(uri, text.to_string());
                }
                publish(&mut output, uri, server.diagnostics(uri));
                continue;
            }
            "textDocument/didSave" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
                if let Some(text) = params["text"].as_str() {
                    server.open(uri, text.to_string());
                }
                publish(&mut output, uri, server.diagnostics(uri));
                continue;
            }
            "textDocument/didClose" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
                server.close(uri);
                continue;
            }
            _ => {}
        }

        let Some(id) = id else {
            continue;
        };

        let result = match method.as_str() {
            "initialize" => Some(server.initialize(&params)),
            "shutdown" => Some(Value::Null),
            "textDocument/hover" => {
                let (uri, line, character) = place(&params);
                Some(server.hover(&uri, line, character))
            }
            "textDocument/definition" => {
                let (uri, line, character) = place(&params);
                Some(server.definition(&uri, line, character))
            }
            "textDocument/references" => {
                let (uri, line, character) = place(&params);
                Some(server.references(&uri, line, character))
            }
            "textDocument/documentSymbol" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or_default();
                Some(server.document_symbols(uri))
            }
            "workspace/symbol" => {
                let query = params["query"].as_str().unwrap_or_default();
                Some(server.workspace_symbols(query))
            }
            "textDocument/completion" => {
                let (uri, line, character) = place(&params);
                Some(server.completion(&uri, line, character))
            }
            _ => None,
        };

        let reply = match result {
            Some(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": format!("`{method}` is not something this server does") },
            }),
        };
        write(&mut output, &reply);
    }
}

/// The document and position a request is about.
fn place(params: &Value) -> (String, u32, u32) {
    (
        params["textDocument"]["uri"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        params["position"]["line"].as_u64().unwrap_or(0) as u32,
        params["position"]["character"].as_u64().unwrap_or(0) as u32,
    )
}

fn publish(output: &mut impl Write, uri: &str, diagnostics: Value) {
    write(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": { "uri": uri, "diagnostics": diagnostics },
        }),
    );
}

/// Reads one message, whose length comes in a header.
fn read(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;

    loop {
        let mut header = String::new();
        if input.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = value.trim().parse::<usize>().ok();
        }
    }

    let mut body = vec![0u8; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write(output: &mut impl Write, message: &Value) {
    let body = message.to_string();
    let _ = write!(output, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = output.flush();
}
