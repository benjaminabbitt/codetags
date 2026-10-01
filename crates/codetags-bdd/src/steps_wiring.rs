//! Steps for `features/lsp/wiring.feature` (M3 stages 1 and 2, PLAN.md
//! D16-D25): this repo's LSP wiring, set up by `tools/lsp/lsp-setup.sh` in a
//! scratch checkout of the Rust fixture, with the committed wrappers started
//! the way Claude Code and VS Code start them, against this toolchain's real
//! rust-analyzer through the real lspmux.
//!
//! The checkout is the shim steps' project (`steps_shim`), so their sessions
//! run in it. Every process runs in the scenario's isolated home
//! (`steps_lspmux`), without `CODETAGS_LSP_LOG_DIR` (a log directory makes
//! each session's server command different, and lspmux would start two
//! servers) and without the calling Claude Code session's
//! `CLAUDE_PROJECT_DIR`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use serde_json::{Value, json};

use crate::steps_lspmux::{an_isolated_home, repo_root, required_lspmux};
use crate::steps_shim::{
    file_uri, handshake, initialize, project, recorded_client_messages, session, spawn_session,
    vscode_capabilities,
};
use crate::{CODETAGS_BIN, CodetagsWorld};

/// How often the polling steps ask again.
const POLL: Duration = Duration::from_millis(250);

/// How long "until it is found" waits for rust-analyzer to load (V128).
const WARM_UP: Duration = Duration::from_secs(60);

/// LSP's ContentModified error: the server's state changed under the
/// request, which a client may resend.
const CONTENT_MODIFIED: i64 = -32801;

/// Variables a wiring process never inherits from the test's environment.
const NOT_INHERITED: [&str; 4] = [
    "CODETAGS_LSP_LOG_DIR",
    "CODETAGS_LSPMUX_XDG_CONFIG_HOME",
    "CLAUDE_PROJECT_DIR",
    "CLAUDE_PLUGIN_ROOT",
];

/// The Claude Code recording whose `initialize` the plugin's session sends.
const CLAUDE_RECORDING: &str = "claude-code-2.1.286/the-handshake";

/// The scratch checkout.
fn checkout(world: &CodetagsWorld) -> PathBuf {
    world
        .shim
        .project
        .clone()
        .expect("a Given step made the scratch checkout")
}

/// `program` in the checkout, with the scenario's environment.
fn wiring_command(world: &CodetagsWorld, program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    for name in NOT_INHERITED {
        command.env_remove(name);
    }
    for name in &world.env_removed {
        command.env_remove(name);
    }
    command.envs(world.env.iter().map(|(name, value)| (name, value)));
    command.current_dir(checkout(world));
    command
}

fn git(checkout: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=codetags",
            "-c",
            "user.email=codetags@invalid",
        ])
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .current_dir(checkout)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- the scratch checkout -------------------------------------------------

#[given("a scratch checkout of the Rust fixture with this repo's LSP wiring")]
fn a_scratch_checkout(world: &mut CodetagsWorld) {
    if world.lspmux.home.is_none() {
        an_isolated_home(world);
    }
    let checkout = world.scratch().join("checkout");
    let repo = repo_root();
    crate::steps_claude::copy_tree(&repo.join("tests").join("fixtures").join("rust"), &checkout);
    for part in [
        ["tools", "lsp"].as_slice(),
        &["tools", "vscode"],
        &["tools", "claude-plugins", "codetags-lsp"],
    ] {
        let relative: PathBuf = part.iter().collect();
        crate::steps_claude::copy_tree(&repo.join(&relative), &checkout.join(&relative));
    }
    std::fs::create_dir_all(checkout.join(".vscode")).expect("create .vscode");
    std::fs::copy(
        repo.join(".vscode").join("settings.json"),
        checkout.join(".vscode").join("settings.json"),
    )
    .expect("copy .vscode/settings.json");
    // Windows wrappers this checkout's own `just lsp-setup` installed.
    remove_installed_wrappers(&checkout.join("tools"));
    std::fs::write(checkout.join(".gitignore"), "/target/\n/.codetags/\n")
        .expect("write .gitignore");
    git(&checkout, &["init", "-q"]);
    git(&checkout, &["add", "-A"]);
    git(&checkout, &["commit", "-q", "-m", "fixture"]);
    world.shim.project = Some(checkout);
}

/// Deletes the gitignored `*.exe` and `*.codetags-lsp.toml` under `dir`.
fn remove_installed_wrappers(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("list a copied directory") {
        let path = entry.expect("read a directory entry").path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() {
            remove_installed_wrappers(&path);
        } else if name.ends_with(".exe") || name.ends_with(".codetags-lsp.toml") {
            std::fs::remove_file(&path).expect("remove an installed wrapper");
        }
    }
}

fn local_dir(world: &CodetagsWorld) -> PathBuf {
    checkout(world).join(".codetags").join("local")
}

#[given("lspmux is installed in the scratch checkout")]
fn lspmux_is_installed(world: &mut CodetagsWorld) {
    let bin = local_dir(world).join("bin");
    std::fs::create_dir_all(&bin).expect("create .codetags/local/bin");
    let name = format!("lspmux{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(required_lspmux(), bin.join(name)).expect("copy lspmux into the checkout");
}

#[given("lsp-setup has run in the scratch checkout")]
fn lsp_setup_has_run(world: &mut CodetagsWorld) {
    let script = checkout(world)
        .join("tools")
        .join("lsp")
        .join("lsp-setup.sh");
    let codetags = CODETAGS_BIN.get().expect("run() sets the binary path");
    let mut command = wiring_command(world, "sh");
    command
        .arg(script)
        .arg(crate::steps_lsp::lsp_binary())
        .arg(codetags)
        .arg("--yes");
    // Windows: lspmux listens on loopback TCP, so each scenario takes its
    // own free port (as "lspmux is set up in an isolated home" does).
    if cfg!(windows) {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .expect("find a free loopback port")
            .port();
        world.lspmux.home.as_mut().expect("the home exists").port = Some(port);
        command.arg("--listen").arg(format!("127.0.0.1:{port}"));
    }
    let output = command
        .stdin(Stdio::null())
        .output()
        .expect("run sh tools/lsp/lsp-setup.sh (sh must be on PATH)");
    assert!(
        output.status.success(),
        "lsp-setup.sh exited {:?}:\n{}{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[given("the scratch checkout's shim is a build with no serve command")]
fn shim_without_serve(world: &mut CodetagsWorld) {
    if cfg!(windows) {
        panic!("the stand-in stage-0 shim is a sh script (Linux and macOS only)");
    }
    let local = local_dir(world);
    let bin = local.join("bin");
    std::fs::create_dir_all(&bin).expect("create .codetags/local/bin");
    let shim = bin.join("codetags-lsp");
    std::fs::write(
        &shim,
        "#!/bin/sh\n\
         # A stand-in for a stage-0 codetags-lsp (just lsp-record-setup): no serve.\n\
         if [ \"$1\" = serve ]; then\n\
         \x20   echo \"error: unrecognized subcommand 'serve'\" >&2\n\
         \x20   exit 2\n\
         fi\n\
         exit 0\n",
    )
    .expect("write the stand-in shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in executable");
    }
    // `just lsp-record-setup` records the real server too.
    std::fs::write(
        local.join("rust-analyzer.path"),
        format!("{}\n", toolchain_rust_analyzer()),
    )
    .expect("write rust-analyzer.path");
}

/// This toolchain's rust-analyzer: `rustup which rust-analyzer` in this repo.
fn toolchain_rust_analyzer() -> String {
    let output = Command::new("rustup")
        .args(["which", "rust-analyzer"])
        .current_dir(repo_root())
        .output()
        .expect("run rustup (it must be on PATH)");
    assert!(
        output.status.success(),
        "no rust-analyzer for this toolchain; add it with: rustup component add rust-analyzer\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

// --- starting the clients' wrappers ---------------------------------------

/// Claude Code's start of the plugin's language server: the wrapper by its
/// path (no extension; Windows resolves the `.exe`, V119), in the project,
/// with the plugin's variables (V107, V114).
fn claude_command(world: &CodetagsWorld, args: &[&str]) -> Command {
    let plugin = checkout(world)
        .join("tools")
        .join("claude-plugins")
        .join("codetags-lsp");
    let mut command = wiring_command(world, plugin.join("scripts").join("rust-analyzer"));
    command
        .args(args)
        .env("CLAUDE_PROJECT_DIR", checkout(world))
        .env("CLAUDE_PLUGIN_ROOT", &plugin);
    command
}

#[when(expr = "Claude Code's plugin wrapper runs with the arguments {string}")]
fn claude_wrapper_runs(world: &mut CodetagsWorld, args: String) {
    let args: Vec<&str> = args.split_whitespace().collect();
    let output = claude_command(world, &args)
        .stdin(Stdio::null())
        .output()
        .expect("run the plugin's wrapper");
    world.last = Some(output.into());
}

#[when(expr = "Claude Code's plugin starts its language server as session {string}")]
fn claude_starts(world: &mut CodetagsWorld, name: String) {
    let root = project(world);
    let initialize = recorded_client_messages(CLAUDE_RECORDING, &root)
        .into_iter()
        .find(|message| message["method"] == "initialize")
        .expect("the recording has an initialize");
    let command = claude_command(world, &[]);
    let mut session = spawn_session(world, &name, command);
    handshake(&mut session, &initialize);
    world.shim.sessions.insert(name, session);
}

/// `node tests/support/spawn-like-vscode.mjs`, which starts
/// `rust-analyzer.server.path` as VS Code's extension does (V118).
fn vscode_command(world: &CodetagsWorld, args: &[&str]) -> Command {
    let helper = repo_root()
        .join("tests")
        .join("support")
        .join("spawn-like-vscode.mjs");
    let mut command = wiring_command(world, "node");
    command.arg(helper).args(args);
    command
}

#[when("VS Code checks its rust-analyzer's version")]
fn vscode_checks_version(world: &mut CodetagsWorld) {
    let output = vscode_command(world, &["--version"])
        .stdin(Stdio::null())
        .output()
        .expect("run node (the VS Code steps need Node on PATH)");
    world.last = Some(output.into());
}

#[when(expr = "VS Code starts its rust-analyzer as session {string}")]
fn vscode_starts(world: &mut CodetagsWorld, name: String) {
    let root = project(world);
    let command = vscode_command(world, &[]);
    let mut session = spawn_session(world, &name, command);
    handshake(
        &mut session,
        &initialize(&name, &root, vscode_capabilities()),
    );
    world.shim.sessions.insert(name, session);
}

// --- talking to rust-analyzer ---------------------------------------------

/// Every symbol name in a `workspace/symbol` or `textDocument/documentSymbol`
/// result, children included.
fn symbol_names(result: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut stack: Vec<&Value> = result.as_array().into_iter().flatten().collect();
    while let Some(symbol) = stack.pop() {
        if let Some(name) = symbol["name"].as_str() {
            names.push(name.to_string());
        }
        stack.extend(symbol["children"].as_array().into_iter().flatten());
    }
    names.sort();
    names
}

/// The names a session's `workspace/symbol` for `query` returned; `Err`
/// with the answer if it was an error.
fn search(world: &mut CodetagsWorld, name: &str, query: &str) -> Result<Vec<String>, Value> {
    let answer = session(world, name).request("workspace/symbol", json!({"query": query}));
    match answer.get("result") {
        Some(result) => Ok(symbol_names(result)),
        None => Err(answer),
    }
}

/// The names a session's `textDocument/documentSymbol` for `file` returned;
/// `Err` with the answer if it was an error.
fn document_symbols(
    world: &mut CodetagsWorld,
    name: &str,
    file: &str,
) -> Result<Vec<String>, Value> {
    let uri = file_uri(&project(world).join(file));
    let answer = session(world, name).request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": uri}}),
    );
    match answer.get("result") {
        Some(result) => Ok(symbol_names(result)),
        None => Err(answer),
    }
}

/// Asks `ask` every [`POLL`] until `done` accepts an answer, for at most
/// `limit`; panics with the last answer.
fn poll_until(
    world: &mut CodetagsWorld,
    limit: Duration,
    what: &str,
    mut ask: impl FnMut(&mut CodetagsWorld) -> Result<Vec<String>, Value>,
    done: impl Fn(&[String]) -> bool,
) {
    let deadline = Instant::now() + limit;
    loop {
        let answer = ask(world);
        if answer.as_deref().is_ok_and(&done) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("{what}: not within {limit:?}; the last answer was {answer:?}");
        }
        std::thread::sleep(POLL);
    }
}

#[when(expr = "session {string} searches the workspace for {string} until it is found")]
fn search_until_found(world: &mut CodetagsWorld, name: String, query: String) {
    poll_until(
        world,
        WARM_UP,
        &format!("session {name:?} searching for {query:?}"),
        |world| search(world, &name, &query),
        |names| names.contains(&query),
    );
}

/// The zero-based line and UTF-16 column of the first whole-word `word` in
/// `text`.
fn position_of(text: &str, word: &str) -> Option<(usize, usize)> {
    let pattern = regex::Regex::new(&format!(r"\b{}\b", regex::escape(word))).ok()?;
    text.lines().enumerate().find_map(|(line, content)| {
        pattern
            .find(content)
            .map(|found| (line, content[..found.start()].encode_utf16().count()))
    })
}

#[when(expr = "session {string} hovers on {string} in {string}")]
fn hovers(world: &mut CodetagsWorld, name: String, word: String, file: String) {
    let path = project(world).join(&file);
    let text = std::fs::read_to_string(&path).expect("read the file to hover in");
    let (line, character) =
        position_of(&text, &word).unwrap_or_else(|| panic!("{file} does not contain {word:?}"));
    let params = json!({
        "textDocument": {"uri": file_uri(&path)},
        "position": {"line": line, "character": character},
    });
    // rust-analyzer answers ContentModified while it is still settling after
    // the warm-up (V130); LSP lets a client resend the request.
    let deadline = Instant::now() + WARM_UP;
    loop {
        let answer = session(world, &name).request("textDocument/hover", params.clone());
        if answer["error"]["code"] != json!(CONTENT_MODIFIED) || Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(POLL);
    }
}

#[when(expr = "session {string} opens {string}")]
fn opens(world: &mut CodetagsWorld, name: String, file: String) {
    let path = project(world).join(&file);
    let text = std::fs::read_to_string(&path).expect("read the file to open");
    session(world, &name).notify(
        "textDocument/didOpen",
        json!({"textDocument": {"uri": file_uri(&path), "languageId": "rust", "version": 1, "text": text}}),
    );
}

#[then(expr = "session {string}'s hover is not empty")]
fn hover_not_empty(world: &mut CodetagsWorld, name: String) {
    let session = session(world, &name);
    let answer = session
        .answers
        .get("textDocument/hover")
        .unwrap_or_else(|| panic!("session {name:?} hovered nowhere"));
    let contents = &answer["result"]["contents"];
    let empty = match contents {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object
            .get("value")
            .and_then(Value::as_str)
            .is_none_or(|text| text.trim().is_empty()),
        _ => false,
    };
    assert!(
        !empty,
        "the hover was empty: {answer}\n{}",
        session.diagnostics()
    );
}

#[then(expr = "within {int} s session {string}'s workspace search for {string} finds it")]
fn search_finds_within(world: &mut CodetagsWorld, seconds: u64, name: String, query: String) {
    poll_until(
        world,
        Duration::from_secs(seconds),
        &format!("session {name:?}'s search for {query:?} finding it"),
        |world| search(world, &name, &query),
        |names| names.contains(&query),
    );
}

#[then(expr = "within {int} s session {string}'s workspace search for {string} finds nothing")]
fn search_finds_nothing_within(
    world: &mut CodetagsWorld,
    seconds: u64,
    name: String,
    query: String,
) {
    poll_until(
        world,
        Duration::from_secs(seconds),
        &format!("session {name:?}'s search for {query:?} finding nothing"),
        |world| search(world, &name, &query),
        |names| !names.contains(&query),
    );
}

#[then(expr = "within {int} s session {string}'s symbols of {string} include {string}")]
fn symbols_include_within(
    world: &mut CodetagsWorld,
    seconds: u64,
    name: String,
    file: String,
    symbol: String,
) {
    poll_until(
        world,
        Duration::from_secs(seconds),
        &format!("session {name:?}'s symbols of {file} including {symbol:?}"),
        |world| document_symbols(world, &name, &file),
        |names| names.contains(&symbol),
    );
}

#[then(expr = "for {int} s session {string}'s symbols of {string} do not include {string}")]
fn symbols_exclude_for(
    world: &mut CodetagsWorld,
    seconds: u64,
    name: String,
    file: String,
    symbol: String,
) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut answered = 0;
    while Instant::now() < deadline {
        if let Ok(names) = document_symbols(world, &name, &file) {
            assert!(
                !names.contains(&symbol),
                "session {name:?}'s symbols of {file} include {symbol:?}: {names:?}"
            );
            if !names.is_empty() {
                answered += 1;
            }
        }
        std::thread::sleep(POLL);
    }
    assert!(
        answered > 0,
        "session {name:?} got no symbols of {file} in {seconds} s"
    );
}

#[then(expr = "lspmux status shows {int} instance(s) with {int} client(s)")]
fn lspmux_status(world: &mut CodetagsWorld, instances: usize, clients: usize) {
    let lspmux = local_dir(world)
        .join("bin")
        .join(format!("lspmux{}", std::env::consts::EXE_SUFFIX));
    let output = wiring_command(world, lspmux)
        .args(["status", "--json"])
        .output()
        .expect("run lspmux status");
    let text = String::from_utf8_lossy(&output.stdout);
    let status: Value = serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "lspmux status --json printed no JSON ({error}):\n{text}{}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    let root = canonical(&checkout(world));
    let ours: Vec<&Value> = status["instances"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|instance| {
            instance["workspaceRoot"]["path"]
                .as_str()
                .is_some_and(|path| canonical(Path::new(path)) == root)
        })
        .collect();
    assert_eq!(ours.len(), instances, "lspmux status: {status:#}");
    for instance in ours {
        let count = instance["clients"].as_array().map_or(0, Vec::len);
        assert_eq!(count, clients, "lspmux status: {status:#}");
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

// --- changes on disk, from no client ----------------------------------------

#[when(expr = "{string} gets the line {string} before the line starting {string}")]
fn insert_line(world: &mut CodetagsWorld, file: String, line: String, prefix: String) {
    let path = project(world).join(&file);
    let text = std::fs::read_to_string(&path).expect("read the file to change");
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = String::with_capacity(text.len() + line.len() + 2);
    let mut found = false;
    for existing in text.split_inclusive('\n') {
        if !found && existing.starts_with(&prefix) {
            out.push_str(&line);
            out.push_str(newline);
            found = true;
        }
        out.push_str(existing);
    }
    assert!(found, "no line of {file} starts with {prefix:?}");
    std::fs::write(&path, out).expect("write the changed file");
}

#[when(expr = "the new file {string} holds {string}")]
fn new_file(world: &mut CodetagsWorld, file: String, text: String) {
    let path = project(world).join(&file);
    std::fs::write(&path, format!("{text}\n")).expect("write the new file");
}

#[when(expr = "git restores {string}")]
fn git_restores(world: &mut CodetagsWorld, file: String) {
    git(&checkout(world), &["checkout", "--", &file]);
}

// --- commands -----------------------------------------------------------------

#[when("codetags doctor runs in the scratch checkout")]
fn doctor_runs(world: &mut CodetagsWorld) {
    let codetags = CODETAGS_BIN.get().expect("run() sets the binary path");
    let output = wiring_command(world, codetags)
        .arg("doctor")
        .stdin(Stdio::null())
        .output()
        .expect("run codetags doctor");
    world.last = Some(output.into());
}

#[then("the scratch checkout's shim can serve")]
fn shim_can_serve(world: &mut CodetagsWorld) {
    let shim = local_dir(world)
        .join("bin")
        .join(format!("codetags-lsp{}", std::env::consts::EXE_SUFFIX));
    let output = wiring_command(world, &shim)
        .args(["serve", "--help"])
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", shim.display()));
    assert!(
        output.status.success(),
        "{} serve --help exited {:?}: {}",
        shim.display(),
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then("the scratch checkout's rust-analyzer path names this toolchain's rust-analyzer")]
fn rust_analyzer_path(world: &mut CodetagsWorld) {
    let recorded = std::fs::read_to_string(local_dir(world).join("rust-analyzer.path"))
        .expect("read .codetags/local/rust-analyzer.path");
    assert_eq!(recorded.trim(), toolchain_rust_analyzer());
}

#[cfg(test)]
mod tests {
    use super::position_of;

    #[test]
    fn positions_are_whole_words_in_utf16() {
        let text = "use a::LedgerX;\npub use ledger::Ledger;\n";
        assert_eq!(position_of(text, "Ledger"), Some((1, 16)));
        assert_eq!(position_of("é Ledger", "Ledger"), Some((0, 2)));
        assert_eq!(position_of(text, "Missing"), None);
    }
}
