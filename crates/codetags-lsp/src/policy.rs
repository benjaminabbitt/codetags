//! What the shim does with each message, by session role (PLAN.md D16;
//! docs/proxy-zero-change.md §5): pure functions, so the rules are tested
//! without processes.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::root::{self, RootRule};
use crate::uri::{self, Rewrite};

/// Who is on the other end of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// An editor (VS Code): its buffers are the truth for the files it has
    /// open, so it sends document contents.
    Editor,
    /// An agent (Claude Code): it never owns documents (D16), so the server
    /// reads its files from disk.
    Agent,
}

impl Role {
    /// `"editor"` or `"agent"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::Agent => "agent",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "editor" => Ok(Self::Editor),
            "agent" => Ok(Self::Agent),
            other => Err(format!("unknown role {other:?}: expected editor or agent")),
        }
    }
}

/// The document notifications an agent session never forwards (D16).
pub const DOCUMENT_NOTIFICATIONS: [&str; 4] = [
    "textDocument/didOpen",
    "textDocument/didChange",
    "textDocument/didSave",
    "textDocument/didClose",
];

/// The LSP error code for invalid parameters, used to refuse a multi-root
/// `initialize`.
pub const INVALID_PARAMS: i64 = -32602;
/// The LSP error for requests before a successful `initialize`.
pub const SERVER_NOT_INITIALIZED: i64 = -32002;

/// What to do with a message from the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromClient {
    /// Pass it to lspmux unchanged.
    Forward,
    /// Drop it, for the reason given.
    Drop(&'static str),
    /// The editor's `exit`: end the session.
    Exit,
}

/// The rule for a message from the editor with `method` (`None` for a
/// response).
pub fn from_client(role: Role, method: Option<&str>) -> FromClient {
    match method {
        Some("exit") => FromClient::Exit,
        // Brief §4.2: the proxy's watcher is authoritative; until it exists
        // (stage 3) the server watches for itself, since the shim takes the
        // capability out of `initialize` ([`prepare_initialize`]).
        Some("workspace/didChangeWatchedFiles") => {
            FromClient::Drop("a client's own file events (the proxy's watcher is authoritative)")
        }
        Some(method) if role == Role::Agent && DOCUMENT_NOTIFICATIONS.contains(&method) => {
            FromClient::Drop("agent sessions never own documents (D16)")
        }
        _ => FromClient::Forward,
    }
}

/// What to do with a message from lspmux (the server side).
#[derive(Debug, Clone, PartialEq)]
pub enum FromServer {
    /// Pass it to the editor unchanged.
    Forward,
    /// Do not pass it on; send this response back to lspmux instead.
    Answer(Value),
}

/// The rule for `message` from lspmux. lspmux forwards `workspace/configuration`
/// to one arbitrary client and drops an error answer, so the server would
/// wait forever (V5). An agent such as Claude Code refuses it with an error,
/// so an agent's shim answers it itself, with `null` for every item ("the
/// client cannot provide it", per the LSP specification). TODO(P3.6): answer
/// from the editor's persisted answers once configuration merge exists.
pub fn from_server(role: Role, message: &Value) -> FromServer {
    if role != Role::Agent
        || message.get("method").and_then(Value::as_str) != Some("workspace/configuration")
    {
        return FromServer::Forward;
    }
    let Some(id) = message.get("id").filter(|id| !id.is_null()) else {
        return FromServer::Forward;
    };
    let items = message
        .pointer("/params/items")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    FromServer::Answer(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": vec![Value::Null; items],
    }))
}

/// The client's `initialize`, prepared for lspmux.
#[derive(Debug, Clone, PartialEq)]
pub enum Initialize {
    /// Answer the client with this error response; start no session.
    Reject(Value),
    /// Send this `initialize` on.
    Forward {
        /// The rewritten message.
        message: Value,
        /// The canonical project root (D26), if the message named a root.
        root: Option<PathBuf>,
        /// The rewrite between the client's spelling of the root and the
        /// canonical one, for the rest of the session; `None` when they
        /// agree.
        rewrite: Option<Rewrite>,
    },
}

/// Prepares the client's first message for lspmux:
///
/// - more than one workspace folder: an error response (brief §4.2,
///   multi-root is [OPEN]), so lspmux's `assert!` is never reached (V4);
/// - the root (`workspaceFolders[0]`, `rootUri`, `rootPath`) normalized to
///   the project root under `rule`, spelled canonically (D26), and any other
///   URI under the client's root respelled to match;
/// - `capabilities.workspace.didChangeWatchedFiles` removed, so the server
///   watches files itself while the shim drops the clients' own events.
///   TODO(stage 3): advertise it once the watcher client feeds the server.
///
/// Anything but an `initialize` request is forwarded unchanged.
pub fn prepare_initialize(mut message: Value, rule: RootRule) -> Initialize {
    if message.get("method").and_then(Value::as_str) != Some("initialize") {
        return Initialize::Forward {
            message,
            root: None,
            rewrite: None,
        };
    }
    let folders = message
        .pointer("/params/workspaceFolders")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if folders > 1 {
        return Initialize::Reject(json!({
            "jsonrpc": "2.0",
            "id": message.get("id").cloned().unwrap_or(Value::Null),
            "error": {
                "code": INVALID_PARAMS,
                "message": format!(
                    "codetags-lsp: a multi-root workspace ({folders} folders) cannot share a server yet; \
                     open a single folder (brief §4.2, multi-root is an open question)"
                ),
            },
        }));
    }
    if let Some(workspace) = message
        .pointer_mut("/params/capabilities/workspace")
        .and_then(Value::as_object_mut)
    {
        workspace.remove("didChangeWatchedFiles");
    }
    let (root, rewrite) = match normalize_root(&mut message, rule) {
        Some((root, rewrite)) => (Some(root), rewrite),
        None => (None, None),
    };
    Initialize::Forward {
        message,
        root,
        rewrite,
    }
}

/// Rewrites the root fields of an `initialize` to the project root, spelled
/// canonically, and returns that root with the rewrite for the session. The
/// client's root comes from `workspaceFolders[0].uri`, else `rootUri`, else
/// `rootPath`, as lspmux reads it. When the client's spelling of the project
/// root is already canonical, the client's own strings are kept.
fn normalize_root(message: &mut Value, rule: RootRule) -> Option<(PathBuf, Option<Rewrite>)> {
    let params = message.get("params")?;
    let folder_uri = params
        .pointer("/workspaceFolders/0/uri")
        .and_then(Value::as_str)
        .map(str::to_string);
    let root_uri = params
        .get("rootUri")
        .and_then(Value::as_str)
        .map(str::to_string);
    let root_path = params
        .get("rootPath")
        .and_then(Value::as_str)
        .map(str::to_string);
    let client_root = folder_uri
        .as_deref()
        .or(root_uri.as_deref())
        .and_then(root::file_uri_to_path)
        .or_else(|| root_path.as_deref().map(PathBuf::from))?;
    let project = root::project_root(&client_root, rule);
    let levels = root::levels_above(&project, &client_root);
    // The project root as the client spells it, and canonically.
    let client_uri = folder_uri.as_deref().or(root_uri.as_deref()).map_or_else(
        || uri::path_to_uri(&project),
        |uri| root::uri_parent(uri, levels),
    );
    let canonical = root::canonical(&project).unwrap_or_else(|| project.clone());
    let canonical_uri = uri::path_to_uri(&canonical);
    let rewrite = Rewrite::new(&client_uri, &canonical_uri);
    if let Some(rewrite) = &rewrite {
        rewrite.to_server(message);
        let params = message.get_mut("params")?;
        if folder_uri.is_some() {
            params["workspaceFolders"][0]["uri"] = json!(canonical_uri);
            if let Some(name) = canonical.file_name() {
                params["workspaceFolders"][0]["name"] = json!(name.to_string_lossy());
            }
        }
        if root_uri.is_some() {
            params["rootUri"] = json!(canonical_uri);
        }
        if root_path.is_some() {
            params["rootPath"] = json!(canonical.to_string_lossy());
        }
        return Some((canonical, Some(rewrite.clone())));
    }
    let params = message.get_mut("params")?;
    if levels > 0 {
        if let Some(uri) = &folder_uri {
            let parent = root::uri_parent(uri, levels);
            params["workspaceFolders"][0]["uri"] = json!(parent);
            if let Some(name) = project.file_name() {
                params["workspaceFolders"][0]["name"] = json!(name.to_string_lossy());
            }
        }
        if let Some(uri) = &root_uri {
            params["rootUri"] = json!(root::uri_parent(uri, levels));
        }
        if let Some(path) = &root_path
            && let Some(parent) = Path::new(path).ancestors().nth(levels)
        {
            params["rootPath"] = json!(parent.to_string_lossy());
        }
    }
    Some((canonical, None))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn agents_never_send_documents_and_nobody_sends_file_events() {
        for method in DOCUMENT_NOTIFICATIONS {
            assert!(matches!(
                from_client(Role::Agent, Some(method)),
                FromClient::Drop(_)
            ));
            assert_eq!(from_client(Role::Editor, Some(method)), FromClient::Forward);
        }
        for role in [Role::Agent, Role::Editor] {
            assert!(matches!(
                from_client(role, Some("workspace/didChangeWatchedFiles")),
                FromClient::Drop(_)
            ));
            assert_eq!(from_client(role, Some("exit")), FromClient::Exit);
            assert_eq!(
                from_client(role, Some("textDocument/hover")),
                FromClient::Forward
            );
            assert_eq!(from_client(role, None), FromClient::Forward);
        }
    }

    #[test]
    fn an_agent_answers_workspace_configuration_itself() {
        let request = json!({"jsonrpc":"2.0","id":"tag:7","method":"workspace/configuration",
            "params":{"items":[{"section":"a"},{"section":"b"}]}});
        assert_eq!(
            from_server(Role::Agent, &request),
            FromServer::Answer(json!({"jsonrpc":"2.0","id":"tag:7","result":[null,null]}))
        );
        assert_eq!(from_server(Role::Editor, &request), FromServer::Forward);
        let other = json!({"jsonrpc":"2.0","method":"window/logMessage","params":{}});
        assert_eq!(from_server(Role::Agent, &other), FromServer::Forward);
    }

    #[test]
    fn multi_root_is_refused() {
        let message = json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{
            "workspaceFolders":[{"uri":"file:///a","name":"a"},{"uri":"file:///b","name":"b"}]}});
        let Initialize::Reject(error) = prepare_initialize(message, RootRule::Client) else {
            panic!("not rejected");
        };
        assert_eq!(error["id"], 3);
        assert_eq!(error["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn watched_files_capability_is_removed() {
        let message = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "capabilities":{"workspace":{"didChangeWatchedFiles":{"dynamicRegistration":true},
            "configuration":true}}}});
        let Initialize::Forward { message, root, .. } =
            prepare_initialize(message, RootRule::Client)
        else {
            panic!("rejected");
        };
        assert!(root.is_none());
        assert_eq!(
            message["params"]["capabilities"]["workspace"],
            json!({"configuration": true})
        );
    }

    #[test]
    fn the_root_moves_up_to_the_cargo_workspace_in_the_clients_spelling() {
        let scratch = tempfile::tempdir().unwrap();
        // Canonical already (macOS's temporary directory is behind a
        // symlink), so the client's spelling is kept.
        let project = root::canonical(scratch.path()).unwrap().join("proj");
        std::fs::create_dir_all(project.join("crates/m")).unwrap();
        std::fs::write(
            project.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/m\"]\n",
        )
        .unwrap();
        std::fs::write(project.join("crates/m/Cargo.toml"), "[package]\n").unwrap();
        let member = project.join("crates/m");
        let uri = format!("file://{}", member.display()).replace('\\', "/");
        let message = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "rootUri": uri, "rootPath": member.display().to_string(),
            "workspaceFolders":[{"uri": uri, "name":"m"}]}});
        let Initialize::Forward { message, root, .. } =
            prepare_initialize(message, RootRule::Cargo)
        else {
            panic!("rejected");
        };
        if cfg!(unix) {
            assert_eq!(root.as_deref(), Some(project.as_path()));
            let expected = format!("file://{}", project.display());
            assert_eq!(message["params"]["rootUri"], json!(expected));
            assert_eq!(
                message["params"]["workspaceFolders"][0]["uri"],
                json!(expected)
            );
            assert_eq!(
                message["params"]["workspaceFolders"][0]["name"],
                json!("proj")
            );
            assert_eq!(
                message["params"]["rootPath"],
                json!(project.display().to_string())
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_becomes_canonical_and_other_uris_follow() {
        let scratch = tempfile::tempdir().unwrap();
        let base = root::canonical(scratch.path()).unwrap();
        std::fs::create_dir_all(base.join("real/proj/src")).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();
        let client = base.join("link/proj");
        let uri = format!("file://{}", client.display());
        let message = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
            "rootUri": uri, "rootPath": client.display().to_string(),
            "workspaceFolders":[{"uri": uri, "name":"proj"}],
            "initializationOptions": {"linked": format!("{uri}/src/lib.rs")}}});
        let Initialize::Forward {
            message,
            root,
            rewrite,
        } = prepare_initialize(message, RootRule::Client)
        else {
            panic!("rejected");
        };
        let canonical = base.join("real/proj");
        let canonical_uri = format!("file://{}", canonical.display());
        assert_eq!(root.as_deref(), Some(canonical.as_path()));
        assert!(rewrite.is_some());
        let params = &message["params"];
        assert_eq!(params["rootUri"], json!(canonical_uri));
        assert_eq!(params["workspaceFolders"][0]["uri"], json!(canonical_uri));
        assert_eq!(params["rootPath"], json!(canonical.display().to_string()));
        assert_eq!(
            params["initializationOptions"]["linked"],
            json!(format!("{canonical_uri}/src/lib.rs"))
        );
    }
}
