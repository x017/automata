//! A language-server client for the script editor.
//!
//! Oberon starts a Python language server (basedpyright, pyright, pylsp or
//! jedi-language-server — whichever is installed, or the command set as
//! `language_server` in the preferences) and talks JSON-RPC to it over
//! stdio with `Content-Length` framing. A reader thread parses incoming
//! messages; the editor polls them once per frame, so the UI never blocks
//! on the server.
//!
//! Supported: document sync (full text), diagnostics, completion, hover and
//! go-to-definition. Columns are converted between the editor's char
//! columns and LSP's UTF-16 code units.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::code::{CodeEditor, CodeEvent, CompletionItem, Diagnostic, Pos, Severity};
use crate::json::Json;

/// Language servers tried in order when none is configured: `(program,
/// arguments)`.
pub const AUTO_SERVERS: [(&str, &[&str]); 4] = [
    ("basedpyright-langserver", &["--stdio"]),
    ("pyright-langserver", &["--stdio"]),
    ("pylsp", &[]),
    ("jedi-language-server", &[]),
];

/// The most completion items shown.
const MAX_COMPLETIONS: usize = 300;

/// Something the app should do after a server message.
#[derive(Debug, Clone, PartialEq)]
pub enum LspAction {
    /// A Console message (`true` = error).
    Log(String, bool),
    /// Open `path` with the caret at `pos` (go-to-definition).
    Goto(PathBuf, Pos),
}

#[derive(Debug, Clone)]
enum Pending {
    Initialize,
    Completion { path: PathBuf, pos: Pos },
    Hover { path: PathBuf, pos: Pos },
    Definition,
    Shutdown,
}

/// A running language server.
pub struct LspClient {
    /// The server program's name (for the status line).
    pub name: String,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Json>,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    initialized: bool,
    /// Messages held until the server finished initializing.
    queue: Vec<Json>,
    /// Documents the server knows: path → synced buffer version.
    open: HashMap<PathBuf, u64>,
    dead: bool,
}

impl LspClient {
    /// Starts the language server named by `command` (a command line), or
    /// the first installed one from [`AUTO_SERVERS`] when `command` is
    /// `None` / empty / `"auto"`. `extra_dirs` are searched before `PATH`
    /// (project virtualenvs). `"off"` disables it.
    ///
    /// # Errors
    /// When no server is installed or it fails to start.
    pub fn start(
        command: Option<&str>,
        root: &Path,
        extra_dirs: &[PathBuf],
    ) -> Result<Self, String> {
        let command = command
            .map(str::trim)
            .filter(|c| !c.is_empty() && *c != "auto");
        if command == Some("off") {
            return Err("disabled in preferences".to_string());
        }
        let (program, args): (PathBuf, Vec<String>) = if let Some(cmd) = command {
            let mut parts = split_command(cmd).into_iter();
            let program = parts.next().ok_or("empty command")?;
            let exe =
                find_executable(&program, extra_dirs).unwrap_or_else(|| PathBuf::from(&program));
            (exe, parts.collect())
        } else {
            AUTO_SERVERS
                .iter()
                .find_map(|(name, args)| {
                    find_executable(name, extra_dirs)
                        .map(|p| (p, args.iter().map(|a| (*a).to_string()).collect()))
                })
                .ok_or_else(|| {
                    "no Python language server found (install basedpyright, pyright, \
                     python-lsp-server or jedi-language-server, or set `language_server` \
                     in ~/.config/oberon/prefs.ron)"
                        .to_string()
                })?
        };
        let name = program.file_name().map_or_else(
            || program.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let mut child = Command::new(&program)
            .args(&args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{}: {e}", program.display()))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("oberon-lsp".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Some(msg) = read_message(&mut reader) {
                    if tx.send(msg).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut client = Self {
            name,
            child,
            stdin,
            rx,
            next_id: 1,
            pending: HashMap::new(),
            initialized: false,
            queue: Vec::new(),
            open: HashMap::new(),
            dead: false,
        };
        let root_uri = path_to_uri(root);
        let params = Json::obj([
            ("processId", Json::Int(i64::from(std::process::id()))),
            ("rootUri", Json::str(root_uri.clone())),
            ("rootPath", Json::str(root.display().to_string())),
            (
                "workspaceFolders",
                Json::Arr(vec![Json::obj([
                    ("uri", Json::str(root_uri)),
                    ("name", Json::str("project")),
                ])]),
            ),
            ("capabilities", capabilities()),
            (
                "clientInfo",
                Json::obj([
                    ("name", Json::str("Oberon")),
                    ("version", Json::str(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]);
        client.send_request("initialize", params, Pending::Initialize, true);
        Ok(client)
    }

    /// Whether the server process has exited.
    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Whether the server answered `initialize`.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.initialized
    }

    fn write(&mut self, msg: &Json) {
        let body = msg.dump();
        let framed = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        if self
            .stdin
            .write_all(framed.as_bytes())
            .and_then(|()| self.stdin.flush())
            .is_err()
        {
            self.dead = true;
        }
    }

    fn send(&mut self, msg: Json, immediately: bool) {
        if self.initialized || immediately {
            self.write(&msg);
        } else {
            self.queue.push(msg);
        }
    }

    fn send_request(&mut self, method: &str, params: Json, pending: Pending, immediately: bool) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        let msg = Json::obj([
            ("jsonrpc", Json::str("2.0")),
            ("id", Json::Int(id)),
            ("method", Json::str(method)),
            ("params", params),
        ]);
        self.send(msg, immediately);
    }

    fn notify(&mut self, method: &str, params: Json) {
        let msg = Json::obj([
            ("jsonrpc", Json::str("2.0")),
            ("method", Json::str(method)),
            ("params", params),
        ]);
        self.send(msg, false);
    }

    fn respond(&mut self, id: Json, result: Json) {
        let msg = Json::obj([
            ("jsonrpc", Json::str("2.0")),
            ("id", id),
            ("result", result),
        ]);
        self.write(&msg);
    }

    /// Mirrors the editor's open Python files to the server (open, change,
    /// close).
    pub fn sync(&mut self, ed: &CodeEditor) {
        let mut live = Vec::new();
        for tab in &ed.tabs {
            if !is_python(&tab.path) {
                continue;
            }
            live.push(tab.path.clone());
            let version = tab.buf.version();
            let uri = path_to_uri(&tab.path);
            match self.open.get(&tab.path).copied() {
                None => {
                    self.notify(
                        "textDocument/didOpen",
                        Json::obj([(
                            "textDocument",
                            Json::obj([
                                ("uri", Json::str(uri)),
                                ("languageId", Json::str("python")),
                                ("version", Json::Int(lsp_version(version))),
                                ("text", Json::str(tab.buf.text())),
                            ]),
                        )]),
                    );
                    self.open.insert(tab.path.clone(), version);
                }
                Some(v) if v != version => {
                    self.notify(
                        "textDocument/didChange",
                        Json::obj([
                            (
                                "textDocument",
                                Json::obj([
                                    ("uri", Json::str(uri)),
                                    ("version", Json::Int(lsp_version(version))),
                                ]),
                            ),
                            (
                                "contentChanges",
                                Json::Arr(vec![Json::obj([("text", Json::str(tab.buf.text()))])]),
                            ),
                        ]),
                    );
                    self.open.insert(tab.path.clone(), version);
                }
                Some(_) => {}
            }
        }
        let closed: Vec<PathBuf> = self
            .open
            .keys()
            .filter(|p| !live.contains(p))
            .cloned()
            .collect();
        for path in closed {
            self.open.remove(&path);
            self.notify(
                "textDocument/didClose",
                Json::obj([(
                    "textDocument",
                    Json::obj([("uri", Json::str(path_to_uri(&path)))]),
                )]),
            );
        }
    }

    /// Sends a request for an editor event (completion, hover, definition,
    /// save). Call [`LspClient::sync`] first so the server has the text.
    pub fn request(&mut self, ed: &CodeEditor, ev: &CodeEvent) {
        let (method, path, pos) = match ev {
            CodeEvent::Complete(p, pos) => ("textDocument/completion", p, *pos),
            CodeEvent::Hover(p, pos) => ("textDocument/hover", p, *pos),
            CodeEvent::Definition(p, pos) => ("textDocument/definition", p, *pos),
            CodeEvent::Saved(p) if is_python(p) => {
                let uri = path_to_uri(p);
                self.notify(
                    "textDocument/didSave",
                    Json::obj([("textDocument", Json::obj([("uri", Json::str(uri))]))]),
                );
                return;
            }
            _ => return,
        };
        if !is_python(path) {
            return;
        }
        let Some(tab) = ed.tabs.iter().find(|t| &t.path == path) else {
            return;
        };
        let line = tab.buf.line(pos.line.min(tab.buf.line_count() - 1));
        let mut params = Json::obj([
            (
                "textDocument",
                Json::obj([("uri", Json::str(path_to_uri(path)))]),
            ),
            (
                "position",
                Json::obj([
                    ("line", Json::Int(to_i64(pos.line))),
                    ("character", Json::Int(to_i64(char_to_utf16(line, pos.col)))),
                ]),
            ),
        ]);
        let pending = match ev {
            CodeEvent::Complete(..) => {
                let before = line.chars().take(pos.col).last();
                let context = if before == Some('.') {
                    Json::obj([
                        ("triggerKind", Json::Int(2)),
                        ("triggerCharacter", Json::str(".")),
                    ])
                } else {
                    Json::obj([("triggerKind", Json::Int(1))])
                };
                params.set("context", context);
                Pending::Completion {
                    path: path.clone(),
                    pos,
                }
            }
            CodeEvent::Hover(..) => Pending::Hover {
                path: path.clone(),
                pos,
            },
            _ => Pending::Definition,
        };
        self.send_request(method, params, pending, false);
    }

    /// Handles everything the server sent since the last call, updating
    /// the editor (diagnostics, completion, hover). Returns what the app
    /// should do.
    pub fn poll(&mut self, ed: &mut CodeEditor) -> Vec<LspAction> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => self.handle(&msg, ed, &mut out),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.dead {
                        self.dead = true;
                        out.push(LspAction::Log(
                            format!("Language server {} exited", self.name),
                            true,
                        ));
                    }
                    break;
                }
            }
        }
        out
    }

    fn handle(&mut self, msg: &Json, ed: &mut CodeEditor, out: &mut Vec<LspAction>) {
        let method = msg.get("method").and_then(Json::as_str).map(str::to_string);
        let id = msg.get("id").cloned();
        match (method, id) {
            // A request from the server: answer so it doesn't wait.
            (Some(method), Some(id)) => {
                let result = match method.as_str() {
                    "workspace/configuration" => {
                        let n = msg
                            .get("params")
                            .and_then(|p| p.get("items"))
                            .and_then(Json::as_arr)
                            .map_or(0, <[Json]>::len);
                        Json::Arr(vec![Json::Null; n])
                    }
                    "workspace/workspaceFolders" => Json::Arr(Vec::new()),
                    _ => Json::Null,
                };
                self.respond(id, result);
            }
            (Some(method), None) => self.notification(&method, msg.get("params"), ed, out),
            (None, Some(id)) => {
                let Some(pending) = id.as_i64().and_then(|i| self.pending.remove(&i)) else {
                    return;
                };
                if let Some(err) = msg.get("error") {
                    if matches!(pending, Pending::Initialize) {
                        let text = err.get("message").and_then(Json::as_str).unwrap_or("error");
                        out.push(LspAction::Log(
                            format!("{} failed to initialize: {text}", self.name),
                            true,
                        ));
                    }
                    return;
                }
                let result = msg.get("result").cloned().unwrap_or_default();
                self.response(pending, &result, ed, out);
            }
            (None, None) => {}
        }
    }

    fn notification(
        &mut self,
        method: &str,
        params: Option<&Json>,
        ed: &mut CodeEditor,
        out: &mut Vec<LspAction>,
    ) {
        let Some(params) = params else { return };
        match method {
            "textDocument/publishDiagnostics" => {
                let Some(path) = params
                    .get("uri")
                    .and_then(Json::as_str)
                    .and_then(uri_to_path)
                else {
                    return;
                };
                let Some(tab) = ed.tabs.iter_mut().find(|t| same_file(&t.path, &path)) else {
                    return;
                };
                let items = params
                    .get("diagnostics")
                    .and_then(Json::as_arr)
                    .unwrap_or(&[]);
                tab.diagnostics = items
                    .iter()
                    .filter_map(|d| {
                        let (start, end) = range_of(d.get("range")?, &tab.buf)?;
                        let severity = match d.get("severity").and_then(Json::as_i64) {
                            Some(2) => Severity::Warning,
                            Some(3) => Severity::Info,
                            Some(4) => Severity::Hint,
                            _ => Severity::Error,
                        };
                        let mut message = d.get("message").and_then(Json::as_str)?.to_string();
                        if let Some(src) = d.get("source").and_then(Json::as_str) {
                            message = format!("{message}  ({src})");
                        }
                        Some(Diagnostic {
                            start,
                            end,
                            severity,
                            message,
                        })
                    })
                    .collect();
            }
            "window/showMessage" => {
                let kind = params.get("type").and_then(Json::as_i64).unwrap_or(3);
                if let Some(text) = params.get("message").and_then(Json::as_str) {
                    if kind <= 2 {
                        out.push(LspAction::Log(format!("{}: {text}", self.name), kind == 1));
                    }
                }
            }
            _ => {}
        }
    }

    fn response(
        &mut self,
        pending: Pending,
        result: &Json,
        ed: &mut CodeEditor,
        out: &mut Vec<LspAction>,
    ) {
        match pending {
            Pending::Initialize => {
                self.initialized = true;
                self.write(&Json::obj([
                    ("jsonrpc", Json::str("2.0")),
                    ("method", Json::str("initialized")),
                    ("params", Json::obj(Vec::<(String, Json)>::new())),
                ]));
                for msg in std::mem::take(&mut self.queue) {
                    self.write(&msg);
                }
                out.push(LspAction::Log(
                    format!("Language server: {}", self.name),
                    false,
                ));
            }
            Pending::Completion { path, pos } => {
                let Some(tab) = ed.current() else { return };
                // Stale: the user moved elsewhere meanwhile.
                if tab.path != path
                    || tab.buf.cursor.line != pos.line
                    || tab.buf.cursor.col < pos.col
                {
                    return;
                }
                let items = result
                    .as_arr()
                    .or_else(|| result.get("items").and_then(Json::as_arr))
                    .unwrap_or(&[]);
                let mut items: Vec<(String, CompletionItem)> =
                    items.iter().filter_map(completion_item).collect();
                items.sort_by(|a, b| a.0.cmp(&b.0));
                items.truncate(MAX_COMPLETIONS);
                ed.show_completions(items.into_iter().map(|(_, i)| i).collect());
            }
            Pending::Hover { path, pos } => {
                let text = hover_text(result.get("contents").unwrap_or(&Json::Null));
                let current = ed.current().is_some_and(|t| t.path == path);
                if current && !text.trim().is_empty() {
                    ed.hover = Some((pos, text));
                }
            }
            Pending::Definition => {
                let first = match result {
                    Json::Arr(items) => items.first(),
                    Json::Obj(_) => Some(result),
                    _ => None,
                };
                let Some(loc) = first else {
                    out.push(LspAction::Log("No definition found".to_string(), false));
                    return;
                };
                let uri = loc
                    .get("uri")
                    .or_else(|| loc.get("targetUri"))
                    .and_then(Json::as_str);
                let range = loc.get("targetSelectionRange").or_else(|| loc.get("range"));
                if let (Some(path), Some(range)) = (uri.and_then(uri_to_path), range) {
                    let line = range
                        .get("start")
                        .and_then(|s| s.get("line"))
                        .and_then(Json::as_i64);
                    let ch = range
                        .get("start")
                        .and_then(|s| s.get("character"))
                        .and_then(Json::as_i64);
                    let line = usize::try_from(line.unwrap_or(0)).unwrap_or(0);
                    let ch = usize::try_from(ch.unwrap_or(0)).unwrap_or(0);
                    let col = std::fs::read_to_string(&path)
                        .ok()
                        .and_then(|t| t.lines().nth(line).map(|l| utf16_to_char(l, ch)))
                        .unwrap_or(ch);
                    out.push(LspAction::Goto(path, Pos::new(line, col)));
                }
            }
            Pending::Shutdown => {}
        }
    }

    /// Asks the server to exit (and kills it if it doesn't).
    pub fn shutdown(&mut self) {
        if !self.dead && self.initialized {
            self.send_request("shutdown", Json::Null, Pending::Shutdown, true);
            self.write(&Json::obj([
                ("jsonrpc", Json::str("2.0")),
                ("method", Json::str("exit")),
            ]));
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.dead = true;
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        if !self.dead {
            self.shutdown();
        }
    }
}

fn capabilities() -> Json {
    let t = || Json::Bool(true);
    Json::obj([
        (
            "textDocument",
            Json::obj([
                (
                    "synchronization",
                    Json::obj([("didSave", t()), ("dynamicRegistration", Json::Bool(false))]),
                ),
                (
                    "completion",
                    Json::obj([
                        (
                            "completionItem",
                            Json::obj([
                                ("snippetSupport", Json::Bool(false)),
                                (
                                    "documentationFormat",
                                    Json::Arr(vec![Json::str("plaintext")]),
                                ),
                            ]),
                        ),
                        ("contextSupport", t()),
                    ]),
                ),
                (
                    "hover",
                    Json::obj([(
                        "contentFormat",
                        Json::Arr(vec![Json::str("plaintext"), Json::str("markdown")]),
                    )]),
                ),
                ("definition", Json::obj([("linkSupport", t())])),
                (
                    "publishDiagnostics",
                    Json::obj([("relatedInformation", Json::Bool(false))]),
                ),
            ]),
        ),
        (
            "workspace",
            Json::obj([("configuration", t()), ("workspaceFolders", t())]),
        ),
        (
            "general",
            Json::obj([("positionEncodings", Json::Arr(vec![Json::str("utf-16")]))]),
        ),
    ])
}

/// Reads one `Content-Length`-framed message; `None` at end of stream.
fn read_message(reader: &mut impl BufRead) -> Option<Json> {
    loop {
        let mut len = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim();
            if header.is_empty() {
                break;
            }
            if let Some((k, v)) = header.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse::<usize>().ok();
                }
            }
        }
        let Some(len) = len else { continue };
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).ok()?;
        if let Ok(msg) = Json::parse(&String::from_utf8_lossy(&body)) {
            return Some(msg);
        }
    }
}

fn completion_item(item: &Json) -> Option<(String, CompletionItem)> {
    let label = item.get("label").and_then(Json::as_str)?.to_string();
    let insert = item
        .get("textEdit")
        .and_then(|e| e.get("newText"))
        .or_else(|| item.get("insertText"))
        .and_then(Json::as_str)
        .unwrap_or(&label)
        .to_string();
    let kind = match item.get("kind").and_then(Json::as_i64) {
        Some(2) => 'm',
        Some(3 | 4) => 'f',
        Some(7 | 8 | 22 | 25) => 'c',
        Some(9) => 'M',
        Some(14) => 'k',
        _ => 'v',
    };
    let detail = item
        .get("detail")
        .and_then(Json::as_str)
        .or_else(|| {
            item.get("labelDetails")
                .and_then(|d| d.get("detail"))
                .and_then(Json::as_str)
        })
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let sort = item
        .get("sortText")
        .and_then(Json::as_str)
        .unwrap_or(&label)
        .to_string();
    // Private names after public ones.
    let sort = if label.starts_with('_') {
        format!("~{sort}")
    } else {
        sort
    };
    Some((
        sort,
        CompletionItem {
            label,
            detail,
            insert,
            kind,
        },
    ))
}

/// Plain text of a hover result (`MarkupContent`, `MarkedString` or an
/// array of them), with Markdown code fences removed.
fn hover_text(contents: &Json) -> String {
    let raw = match contents {
        Json::Str(s) => s.clone(),
        Json::Arr(items) => items.iter().map(hover_text).collect::<Vec<_>>().join("\n"),
        Json::Obj(_) => contents
            .get("value")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t == "---" {
            continue;
        }
        out.push(line.replace("&nbsp;", " ").replace("\\_", "_"));
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn range_of(range: &Json, buf: &crate::code::Buffer) -> Option<(Pos, Pos)> {
    let pos = |p: &Json| -> Option<Pos> {
        let line = usize::try_from(p.get("line")?.as_i64()?).ok()?;
        let ch = usize::try_from(p.get("character")?.as_i64()?).ok()?;
        let line = line.min(buf.line_count() - 1);
        Some(Pos::new(line, utf16_to_char(buf.line(line), ch)))
    };
    Some((pos(range.get("start")?)?, pos(range.get("end")?)?))
}

/// UTF-16 offset of char column `col`.
#[must_use]
pub fn char_to_utf16(line: &str, col: usize) -> usize {
    line.chars().take(col).map(char::len_utf16).sum()
}

/// Char column of UTF-16 offset `units` (clamped to the line).
#[must_use]
pub fn utf16_to_char(line: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in line.chars().enumerate() {
        if n >= units {
            return i;
        }
        n += c.len_utf16();
    }
    line.chars().count()
}

fn to_i64(v: usize) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn lsp_version(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn is_python(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("py") || e.eq_ignore_ascii_case("pyi"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || std::fs::canonicalize(a)
            .ok()
            .zip(std::fs::canonicalize(b).ok())
            .is_some_and(|(x, y)| x == y)
}

/// A `file://` URI for `path` (absolute; percent-encoded).
#[must_use]
pub fn path_to_uri(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_or_else(|_| path.to_path_buf(), |d| d.join(path))
    };
    let s = abs.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !s.starts_with('/') {
        out.push('/');
    }
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The path of a `file://` URI.
#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let mut s = String::from_utf8(out).ok()?;
    // file:///C:/x → C:/x on Windows.
    if cfg!(windows) && s.len() > 2 && s.as_bytes()[2] == b':' {
        s.remove(0);
    }
    Some(PathBuf::from(s))
}

/// Splits a command line on whitespace, honoring double / single quotes.
#[must_use]
pub fn split_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = None;
    let mut any = false;
    for c in cmd.chars() {
        match (quote, c) {
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            _ => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Finds `name` in `extra` directories, then on `PATH`.
#[must_use]
pub fn find_executable(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let path_dirs = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    let exts: &[&str] = if cfg!(windows) {
        &["exe", "cmd", "bat"]
    } else {
        &[""]
    };
    extra.iter().chain(path_dirs.iter()).find_map(|dir| {
        exts.iter().find_map(|ext| {
            let candidate = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{name}.{ext}"))
            };
            candidate.is_file().then_some(candidate)
        })
    })
}

/// Directories where a project's tools may live: its virtualenvs, the
/// configured interpreter's directory and `~/.local/bin`.
#[must_use]
pub fn tool_dirs(root: &Path, python: Option<&str>) -> Vec<PathBuf> {
    let bin = if cfg!(windows) { "Scripts" } else { "bin" };
    let mut dirs = vec![root.join(".venv").join(bin), root.join("venv").join(bin)];
    if let Some(dir) = python
        .map(Path::new)
        .filter(|p| p.components().count() > 1)
        .and_then(Path::parent)
    {
        dirs.push(dir.to_path_buf());
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local").join("bin"));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_round_trip() {
        let line = "s = '😀é' + x";
        assert_eq!(char_to_utf16(line, 5), 5);
        assert_eq!(char_to_utf16(line, 6), 7); // the emoji is 2 units
        assert_eq!(utf16_to_char(line, 7), 6);
        assert_eq!(utf16_to_char(line, 8), 7);
        assert_eq!(utf16_to_char(line, 999), line.chars().count());
    }

    #[test]
    fn uris_round_trip() {
        let p = Path::new("/home/a b/proj/é.py");
        let uri = path_to_uri(p);
        assert_eq!(uri, "file:///home/a%20b/proj/%C3%A9.py");
        assert_eq!(uri_to_path(&uri).unwrap(), p);
    }

    #[test]
    fn splits_commands() {
        assert_eq!(split_command("pylsp -v"), vec!["pylsp", "-v"]);
        assert_eq!(
            split_command(r#""/opt/my tools/ls" --stdio"#),
            vec!["/opt/my tools/ls", "--stdio"]
        );
        assert_eq!(split_command("  "), Vec::<String>::new());
    }

    #[test]
    fn reads_framed_messages() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":null}"#;
        let raw = format!(
            "Content-Length: {}\r\nContent-Type: x\r\n\r\n{body}",
            body.len()
        );
        let mut r = BufReader::new(raw.as_bytes());
        let msg = read_message(&mut r).unwrap();
        assert_eq!(msg.get("id").and_then(Json::as_i64), Some(1));
        assert!(read_message(&mut r).is_none());
    }

    #[test]
    fn hover_text_strips_fences() {
        let md = Json::obj([
            ("kind", Json::str("markdown")),
            (
                "value",
                Json::str("```python\ndef jump(self) -> None\n```\n---\nMakes the player jump.\n"),
            ),
        ]);
        assert_eq!(
            hover_text(&md),
            "def jump(self) -> None\nMakes the player jump."
        );
    }

    #[test]
    fn completion_items_map_kinds() {
        let item = Json::obj([
            ("label", Json::str("on_update")),
            ("kind", Json::Int(2)),
            ("detail", Json::str("(self, ctx) -> None")),
        ]);
        let (_, c) = completion_item(&item).unwrap();
        assert_eq!((c.kind, c.insert.as_str()), ('m', "on_update"));
    }
}
