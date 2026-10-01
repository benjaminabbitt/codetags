// Starts the rust-analyzer that .vscode/settings.json names, the way VS Code's
// rust-analyzer extension does (V118), for features/lsp/wiring.feature.
//
//     node spawn-like-vscode.mjs --version
//     node spawn-like-vscode.mjs
//
// Run it in the workspace folder. It reads `rust-analyzer.server.path` from
// .vscode/settings.json (JSON with comments) and substitutes
// ${workspaceFolder} with the current directory. Then, through
// child_process.spawn with no shell, as the extension does, so that on
// Windows libuv resolves a path with no extension to `<path>.exe` (V119):
//
// - with --version: runs `<path> --version`, passes its output through, and
//   exits with its status (non-zero if it could not start);
// - with no arguments: runs the same --version check, which must exit 0
//   (its output goes to stderr), then starts `<path>` with no arguments and
//   this process's stdio, and exits with its status.

import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/** `text` with JSONC comments and trailing commas removed. */
function stripJsonc(text) {
  let out = "";
  let inString = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      out += c;
      if (c === "\\") {
        out += text[++i] ?? "";
      } else if (c === '"') {
        inString = false;
      }
    } else if (c === '"') {
      inString = true;
      out += c;
    } else if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      out += "\n";
    } else if (c === "/" && text[i + 1] === "*") {
      i += 2;
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) i++;
      i++;
    } else {
      out += c;
    }
  }
  return out.replace(/,(\s*[}\]])/g, "$1");
}

function serverPath() {
  const folder = process.cwd();
  const file = join(folder, ".vscode", "settings.json");
  const settings = JSON.parse(stripJsonc(readFileSync(file, "utf8")));
  const path = settings["rust-analyzer.server.path"];
  if (typeof path !== "string" || path === "") {
    console.error(`spawn-like-vscode: ${file} sets no rust-analyzer.server.path`);
    process.exit(2);
  }
  return path.replaceAll("${workspaceFolder}", folder);
}

/**
 * Runs `command args` with no shell; resolves to its exit status (127 if it
 * could not start). With `inherit`, it gets this process's stdio; otherwise
 * its stdout goes to `stdout` and its stderr to stderr.
 */
function run(command, args, { inherit = false, stdout = process.stdout } = {}) {
  return new Promise((resolve) => {
    const stdio = inherit ? "inherit" : ["ignore", "pipe", "pipe"];
    const child = spawn(command, args, { shell: false, stdio });
    if (!inherit) {
      child.stdout.on("data", (data) => stdout.write(data));
      child.stderr.on("data", (data) => process.stderr.write(data));
    }
    child.on("error", (error) => {
      console.error(`spawn-like-vscode: cannot start ${command}: ${error.message}`);
      resolve(127);
    });
    child.on("close", (code, signal) => resolve(code ?? (signal ? 128 : 1)));
  });
}

const path = serverPath();
const args = process.argv.slice(2);
if (args.length === 1 && args[0] === "--version") {
  process.exit(await run(path, ["--version"]));
} else if (args.length === 0) {
  // stdout is the LSP channel, so the check's output goes to stderr.
  const checked = await run(path, ["--version"], { stdout: process.stderr });
  if (checked !== 0) {
    console.error(`spawn-like-vscode: ${path} --version exited ${checked}`);
    process.exit(checked);
  }
  process.exit(await run(path, [], { inherit: true }));
} else {
  console.error("usage: node spawn-like-vscode.mjs [--version]");
  process.exit(2);
}
