//! `codetags-lsp analyze`: a readable summary of a recording (the log
//! [`crate::record`] writes), aimed at the questions stage 0 must answer
//! (M3, D16–D18): what the client sends in `initialize`, which document-sync
//! notifications it sends and when, how it answers the server's requests,
//! and how long its own requests take.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use serde_json::Value;

/// Document-sync notifications the client may send.
const SYNC_METHODS: [&str; 5] = [
    "textDocument/didOpen",
    "textDocument/didChange",
    "textDocument/didSave",
    "textDocument/didClose",
    "workspace/didChangeWatchedFiles",
];

/// Server-to-client requests whose answers stage 0 checks (reported to be
/// answered with -32601, Method not found).
const CHECKED_SERVER_REQUESTS: [&str; 3] = [
    "client/registerCapability",
    "window/workDoneProgress/create",
    "workspace/configuration",
];

/// Capability paths the summary always reports, present or not.
const CAPABILITY_PATHS: [&str; 12] = [
    "workspace.didChangeWatchedFiles",
    "workspace.configuration",
    "workspace.workspaceFolders",
    "workspace.didChangeConfiguration",
    "window.workDoneProgress",
    "window.showDocument",
    "window.showMessage",
    "general.positionEncodings",
    "textDocument.synchronization",
    "textDocument.publishDiagnostics",
    "textDocument.diagnostic",
    "experimental",
];

/// One message record from the log.
#[derive(Debug)]
struct Message {
    t_ms: f64,
    from: String,
    kind: String,
    msg: Value,
}

impl Message {
    fn method(&self) -> &str {
        self.msg["method"].as_str().unwrap_or("")
    }

    fn id_key(&self) -> String {
        self.msg["id"].to_string()
    }
}

/// A request and, if one came, its response.
#[derive(Debug)]
struct Exchange<'a> {
    request: &'a Message,
    response: Option<&'a Message>,
}

impl Exchange<'_> {
    fn latency_ms(&self) -> Option<f64> {
        self.response
            .map(|response| response.t_ms - self.request.t_ms)
    }

    fn answer(&self) -> String {
        match self.response {
            None => "no answer".to_string(),
            Some(response) => describe_response(&response.msg),
        }
    }

    fn error_code(&self) -> Option<i64> {
        self.response
            .and_then(|response| response.msg["error"]["code"].as_i64())
    }
}

/// Summarizes the JSON-lines recording `log` as Markdown-ish text.
pub fn analyze(log: &str) -> Result<String, String> {
    let mut messages = Vec::new();
    let mut events = Vec::new();
    for (number, line) in log.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line)
            .map_err(|error| format!("line {}: not JSON: {error}", number + 1))?;
        match record["from"].as_str() {
            Some(from) => messages.push(Message {
                t_ms: record["t_ms"].as_f64().unwrap_or(0.0),
                from: from.to_string(),
                kind: record["kind"].as_str().unwrap_or("invalid").to_string(),
                msg: record["msg"].clone(),
            }),
            None => events.push(record),
        }
    }
    let mut out = String::new();
    session(&mut out, &events, &messages);
    let root = initialize(&mut out, &messages);
    let client_requests = exchanges(&messages, "client");
    let server_requests = exchanges(&messages, "server");
    document_sync(&mut out, &messages, root.as_deref());
    server_to_client(&mut out, &server_requests);
    client_to_server(&mut out, &client_requests);
    notification_counts(&mut out, &messages);
    gaps(&mut out, &messages, &server_requests);
    Ok(out)
}

fn session(out: &mut String, events: &[Value], messages: &[Message]) {
    let _ = writeln!(out, "# LSP recording summary\n");
    for event in events {
        match event["event"].as_str() {
            Some("start") => {
                let _ = writeln!(
                    out,
                    "- server: `{}` args {} (pid {}, cwd `{}`)",
                    event["server"].as_str().unwrap_or("?"),
                    event["args"],
                    event["pid"],
                    event["cwd"].as_str().unwrap_or("?")
                );
            }
            Some("eof") => {
                let _ = writeln!(
                    out,
                    "- {} closed its stream at {}{}",
                    event["side"].as_str().unwrap_or("?"),
                    seconds(event["t_ms"].as_f64().unwrap_or(0.0)),
                    event["error"]
                        .as_str()
                        .map(|error| format!(" ({error})"))
                        .unwrap_or_default()
                );
            }
            Some("exit") => {
                let _ = writeln!(
                    out,
                    "- server exited at {}: status {}, signal {}",
                    seconds(event["t_ms"].as_f64().unwrap_or(0.0)),
                    event["status"],
                    event["signal"]
                );
            }
            Some("malformed") => {
                let _ = writeln!(
                    out,
                    "- MALFORMED stream from the {} at {}: {}",
                    event["side"].as_str().unwrap_or("?"),
                    seconds(event["t_ms"].as_f64().unwrap_or(0.0)),
                    event["reason"].as_str().unwrap_or("?")
                );
            }
            _ => {}
        }
    }
    let from_client = messages.iter().filter(|m| m.from == "client").count();
    let _ = writeln!(
        out,
        "- messages: {from_client} from the client, {} from the server\n",
        messages.len() - from_client
    );
}

/// Writes the `initialize` section; returns the root URI, if any.
fn initialize(out: &mut String, messages: &[Message]) -> Option<String> {
    let _ = writeln!(out, "## initialize (client to server)\n");
    let Some(request) = messages
        .iter()
        .find(|m| m.from == "client" && m.method() == "initialize")
    else {
        let _ = writeln!(out, "No initialize request was recorded.\n");
        return None;
    };
    let params = &request.msg["params"];
    for key in [
        "rootUri",
        "rootPath",
        "workspaceFolders",
        "processId",
        "clientInfo",
        "locale",
        "trace",
        "initializationOptions",
    ] {
        let value = params
            .get(key)
            .map_or("(absent)".to_string(), Value::to_string);
        let _ = writeln!(out, "- `{key}`: {value}");
    }
    let capabilities = &params["capabilities"];
    let _ = writeln!(out, "\nSelected client capabilities:\n");
    for path in CAPABILITY_PATHS {
        let value = lookup(capabilities, path).map_or("(absent)".to_string(), Value::to_string);
        let _ = writeln!(out, "- `{path}`: {value}");
    }
    let _ = writeln!(
        out,
        "\nFull client capabilities:\n\n```json\n{}\n```\n",
        serde_json::to_string_pretty(capabilities).unwrap_or_default()
    );
    if let Some(response) = messages
        .iter()
        .find(|m| m.from == "server" && m.kind == "response" && m.id_key() == request.id_key())
    {
        let result = &response.msg["result"];
        let _ = writeln!(
            out,
            "Server answered after {:.0} ms: `textDocumentSync` {}, `positionEncoding` {}, `serverInfo` {}\n",
            response.t_ms - request.t_ms,
            result["capabilities"]["textDocumentSync"],
            result["capabilities"]["positionEncoding"],
            result["serverInfo"]
        );
    }
    params["rootUri"].as_str().map(str::to_string)
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |value, key| value.get(key))
}

fn document_sync(out: &mut String, messages: &[Message], root: Option<&str>) {
    let _ = writeln!(out, "## Document sync (client to server)\n");
    let short = |uri: &str| -> String {
        match root {
            Some(root) => uri
                .strip_prefix(root)
                .map(|rest| rest.trim_start_matches('/').to_string())
                .unwrap_or_else(|| uri.to_string()),
            None => uri.to_string(),
        }
    };
    let mut per_document: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut any = false;
    for message in messages
        .iter()
        .filter(|m| m.from == "client" && SYNC_METHODS.contains(&m.method()))
    {
        any = true;
        let params = &message.msg["params"];
        let document = &params["textDocument"];
        let uri = short(document["uri"].as_str().unwrap_or("?"));
        let detail = match message.method() {
            "textDocument/didOpen" => format!(
                "{uri} version {} ({} bytes)",
                document["version"],
                document["text"].as_str().map_or(0, str::len)
            ),
            "textDocument/didChange" => {
                let changes = params["contentChanges"]
                    .as_array()
                    .map_or(&[][..], Vec::as_slice);
                let full = changes.iter().filter(|c| c.get("range").is_none()).count();
                format!(
                    "{uri} version {} ({} changes, {full} full-text)",
                    document["version"],
                    changes.len()
                )
            }
            "workspace/didChangeWatchedFiles" => params["changes"]
                .as_array()
                .map(|changes| {
                    changes
                        .iter()
                        .map(|change| {
                            format!(
                                "{}:{}",
                                change_type(&change["type"]),
                                short(change["uri"].as_str().unwrap_or("?"))
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default(),
            _ => uri.clone(),
        };
        let slot = match message.method() {
            "textDocument/didOpen" => Some(0),
            "textDocument/didChange" => Some(1),
            "textDocument/didSave" => Some(2),
            "textDocument/didClose" => Some(3),
            _ => None,
        };
        if let Some(slot) = slot {
            per_document.entry(uri).or_default()[slot] += 1;
        }
        let _ = writeln!(
            out,
            "- {} `{}` {detail}",
            seconds(message.t_ms),
            message.method()
        );
    }
    if !any {
        let _ = writeln!(out, "None.");
    }
    if !per_document.is_empty() {
        let _ = writeln!(
            out,
            "\n| document | didOpen | didChange | didSave | didClose |\n|---|---|---|---|---|"
        );
        for (uri, [open, change, save, close]) in &per_document {
            let _ = writeln!(out, "| {uri} | {open} | {change} | {save} | {close} |");
        }
    }
    let _ = writeln!(out);
}

fn change_type(value: &Value) -> &'static str {
    match value.as_i64() {
        Some(1) => "created",
        Some(2) => "changed",
        Some(3) => "deleted",
        _ => "?",
    }
}

/// Requests `from` sent, in order, each with the other side's response.
fn exchanges<'a>(messages: &'a [Message], from: &str) -> Vec<Exchange<'a>> {
    let mut responses: HashMap<String, &Message> = HashMap::new();
    for message in messages
        .iter()
        .filter(|m| m.from != from && m.kind == "response")
    {
        responses.entry(message.id_key()).or_insert(message);
    }
    messages
        .iter()
        .filter(|m| m.from == from && m.kind == "request")
        .map(|request| Exchange {
            request,
            response: responses.get(&request.id_key()).copied(),
        })
        .collect()
}

fn describe_response(response: &Value) -> String {
    if let Some(error) = response.get("error") {
        return format!(
            "error {} {}",
            error["code"],
            error["message"].as_str().unwrap_or("")
        );
    }
    let result = response.get("result").cloned().unwrap_or(Value::Null);
    let text = result.to_string();
    if text.len() > 120 {
        format!("result ({} bytes)", text.len())
    } else {
        format!("result {text}")
    }
}

fn server_to_client(out: &mut String, requests: &[Exchange<'_>]) {
    let _ = writeln!(
        out,
        "## Server-to-client requests, and the client's answers\n"
    );
    if requests.is_empty() {
        let _ = writeln!(out, "None.\n");
        return;
    }
    let _ = writeln!(
        out,
        "| at | id | method | answer | after |\n|---|---|---|---|---|"
    );
    for exchange in requests {
        let _ = writeln!(
            out,
            "| {} | {} | `{}` | {} | {} |",
            seconds(exchange.request.t_ms),
            exchange.request.msg["id"],
            exchange.request.method(),
            exchange.answer(),
            exchange
                .latency_ms()
                .map_or("-".to_string(), |ms| format!("{ms:.0} ms"))
        );
    }
    let _ = writeln!(out);
}

fn client_to_server(out: &mut String, requests: &[Exchange<'_>]) {
    let _ = writeln!(out, "## Client-to-server requests\n");
    if requests.is_empty() {
        let _ = writeln!(out, "None.\n");
        return;
    }
    let _ = writeln!(
        out,
        "| at | id | method | answer | latency |\n|---|---|---|---|---|"
    );
    let mut latencies: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for exchange in requests {
        if let Some(latency) = exchange.latency_ms() {
            latencies
                .entry(exchange.request.method())
                .or_default()
                .push(latency);
        }
        let _ = writeln!(
            out,
            "| {} | {} | `{}` | {} | {} |",
            seconds(exchange.request.t_ms),
            exchange.request.msg["id"],
            exchange.request.method(),
            exchange.answer(),
            exchange
                .latency_ms()
                .map_or("-".to_string(), |ms| format!("{ms:.0} ms"))
        );
    }
    let _ = writeln!(
        out,
        "\n| method | answered | min | median | max |\n|---|---|---|---|---|"
    );
    for (method, mut values) in latencies {
        values.sort_by(f64::total_cmp);
        let median = values[values.len() / 2];
        let _ = writeln!(
            out,
            "| `{method}` | {} | {:.0} ms | {median:.0} ms | {:.0} ms |",
            values.len(),
            values[0],
            values[values.len() - 1]
        );
    }
    let _ = writeln!(out);
}

fn notification_counts(out: &mut String, messages: &[Message]) {
    let _ = writeln!(out, "## Notifications\n");
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for message in messages.iter().filter(|m| m.kind == "notification") {
        *counts.entry((&message.from, message.method())).or_default() += 1;
    }
    if counts.is_empty() {
        let _ = writeln!(out, "None.\n");
        return;
    }
    let _ = writeln!(out, "| from | method | count |\n|---|---|---|");
    for ((from, method), count) in counts {
        let _ = writeln!(out, "| {from} | `{method}` | {count} |");
    }
    let _ = writeln!(out);
}

/// Verdicts on the reported client gaps (stage 0's questions).
fn gaps(out: &mut String, messages: &[Message], server_requests: &[Exchange<'_>]) {
    let _ = writeln!(out, "## Reported Claude Code client gaps\n");
    let initialize = messages
        .iter()
        .find(|m| m.from == "client" && m.method() == "initialize");
    match initialize {
        None => {
            let _ = writeln!(
                out,
                "- no initialize recorded: capability and root checks not possible"
            );
        }
        Some(request) => {
            let params = &request.msg["params"];
            let watched = lookup(&params["capabilities"], "workspace.didChangeWatchedFiles");
            let verdict = match watched {
                None => "CONFIRMED: the capability is absent".to_string(),
                Some(value) => format!("REFUTED: present, {value}"),
            };
            let _ = writeln!(
                out,
                "- no `workspace.didChangeWatchedFiles` capability: {verdict}"
            );
            let root = params.get("rootUri");
            let verdict = match root {
                Some(Value::Null) => "CONFIRMED: `rootUri` is null".to_string(),
                None => "CONFIRMED (absent): no `rootUri` key".to_string(),
                Some(value) => format!("REFUTED: `rootUri` is {value}"),
            };
            let _ = writeln!(out, "- `rootUri: null`: {verdict}");
        }
    }
    for method in CHECKED_SERVER_REQUESTS {
        let asked: Vec<&Exchange<'_>> = server_requests
            .iter()
            .filter(|exchange| exchange.request.method() == method)
            .collect();
        let not_found = asked
            .iter()
            .filter(|exchange| exchange.error_code() == Some(-32601))
            .count();
        let verdict = if asked.is_empty() {
            "NOT OBSERVED: the server never sent one".to_string()
        } else if not_found == asked.len() {
            format!("CONFIRMED: all {} answered -32601", asked.len())
        } else {
            let answers: Vec<String> = asked.iter().map(|exchange| exchange.answer()).collect();
            format!(
                "REFUTED: {not_found} of {} answered -32601; answers: {}",
                asked.len(),
                answers.join("; ")
            )
        };
        let _ = writeln!(out, "- `{method}` answered -32601: {verdict}");
    }
    let mut opened: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for message in messages.iter().filter(|m| m.from == "client") {
        let uri = message.msg["params"]["textDocument"]["uri"]
            .as_str()
            .unwrap_or("");
        match message.method() {
            "textDocument/didOpen" => opened.entry(uri).or_default().0 += 1,
            "textDocument/didChange" => {
                if let Some(entry) = opened.get_mut(uri) {
                    entry.1 += 1;
                }
            }
            _ => {}
        }
    }
    let changed = opened.values().filter(|(_, changes)| *changes > 0).count();
    let verdict = if opened.is_empty() {
        "NOT OBSERVED: no didOpen was sent".to_string()
    } else if changed == 0 {
        format!(
            "CONFIRMED for this session: none of {} opened documents got a didChange (check the runbook's edits touched opened files)",
            opened.len()
        )
    } else {
        format!(
            "REFUTED: {changed} of {} opened documents got a didChange",
            opened.len()
        )
    };
    let _ = writeln!(out, "- no `didChange` after `didOpen`: {verdict}");
}

fn seconds(t_ms: f64) -> String {
    format!("{:.3}s", t_ms / 1000.0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::analyze;

    fn line(t_ms: f64, from: &str, kind: &str, msg: &str) -> String {
        format!(r#"{{"ts_ms":0,"t_ms":{t_ms},"from":"{from}","kind":"{kind}","msg":{msg}}}"#)
    }

    fn sample() -> String {
        [
            r#"{"ts_ms":0,"t_ms":0,"event":"start","server":"/ra","args":[],"pid":9,"cwd":"/w"}"#.to_string(),
            line(1.0, "client", "request", r#"{"id":1,"method":"initialize","params":{"rootUri":null,"workspaceFolders":[{"uri":"file:///w","name":"w"}],"capabilities":{"workspace":{"configuration":true}}}}"#),
            line(50.0, "server", "response", r#"{"id":1,"result":{"capabilities":{"textDocumentSync":2}}}"#),
            line(60.0, "client", "notification", r#"{"method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///w/src/a.rs","version":1,"text":"fn a(){}"}}}"#),
            line(70.0, "server", "request", r#"{"id":0,"method":"window/workDoneProgress/create","params":{}}"#),
            line(71.0, "client", "response", r#"{"id":0,"error":{"code":-32601,"message":"Method not found"}}"#),
            line(80.0, "server", "request", r#"{"id":1,"method":"workspace/configuration","params":{}}"#),
            line(81.0, "client", "response", r#"{"id":1,"result":[null]}"#),
            line(90.0, "client", "request", r#"{"id":2,"method":"textDocument/hover","params":{}}"#),
            line(130.0, "server", "response", r#"{"id":2,"result":null}"#),
            r#"{"ts_ms":0,"t_ms":200,"event":"exit","status":0,"signal":null}"#.to_string(),
        ]
        .join("\n")
    }

    #[test]
    fn reports_initialize_params_and_capabilities() {
        let report = analyze(&sample()).unwrap();
        assert!(report.contains("- `rootUri`: null"), "{report}");
        assert!(
            report.contains(r#"- `workspaceFolders`: [{"name":"w","uri":"file:///w"}]"#),
            "{report}"
        );
        assert!(
            report.contains("- `workspace.configuration`: true"),
            "{report}"
        );
        assert!(
            report.contains("- `workspace.didChangeWatchedFiles`: (absent)"),
            "{report}"
        );
    }

    #[test]
    fn reports_document_sync_with_times() {
        let report = analyze(&sample()).unwrap();
        assert!(
            report
                .contains("- 0.060s `textDocument/didOpen` file:///w/src/a.rs version 1 (8 bytes)"),
            "{report}"
        );
    }

    #[test]
    fn pairs_server_requests_with_the_client_answers_by_direction() {
        let report = analyze(&sample()).unwrap();
        assert!(
            report.contains("| 0.070s | 0 | `window/workDoneProgress/create` | error -32601 Method not found | 1 ms |"),
            "{report}"
        );
        // Server request id 1 pairs with the client's response 1, not the
        // server's response to the client's initialize (also id 1).
        assert!(
            report.contains("| 0.080s | 1 | `workspace/configuration` | result [null] | 1 ms |"),
            "{report}"
        );
    }

    #[test]
    fn reports_client_request_latencies() {
        let report = analyze(&sample()).unwrap();
        assert!(
            report.contains("| 0.090s | 2 | `textDocument/hover` | result null | 40 ms |"),
            "{report}"
        );
        assert!(
            report.contains("| `textDocument/hover` | 1 | 40 ms | 40 ms | 40 ms |"),
            "{report}"
        );
    }

    #[test]
    fn gives_verdicts_on_the_reported_gaps() {
        let report = analyze(&sample()).unwrap();
        for expected in [
            "- no `workspace.didChangeWatchedFiles` capability: CONFIRMED",
            "- `rootUri: null`: CONFIRMED",
            "- `window/workDoneProgress/create` answered -32601: CONFIRMED: all 1",
            "- `workspace/configuration` answered -32601: REFUTED: 0 of 1",
            "- `client/registerCapability` answered -32601: NOT OBSERVED",
            "- no `didChange` after `didOpen`: CONFIRMED for this session: none of 1",
        ] {
            assert!(
                report.contains(expected),
                "missing {expected:?} in\n{report}"
            );
        }
    }

    #[test]
    fn rejects_lines_that_are_not_json() {
        assert!(analyze("{}\nnot json").unwrap_err().starts_with("line 2"));
    }
}
