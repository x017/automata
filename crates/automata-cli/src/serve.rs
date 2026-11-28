//! A small static web server for playing web builds on this computer.
//!
//! Browsers won't run a web build from a `file://` URL (the page fetches
//! its scripts, the engine and the game), so `automata-cli serve` and
//! Oberon's **Play in Browser** serve the build folder over HTTP, on
//! `127.0.0.1` unless asked otherwise. Only what a web build needs: `GET` and `HEAD` of files
//! under one folder, with the content types browsers insist on (JavaScript
//! modules must be `text/javascript`, WebAssembly `application/wasm`).
//! Nothing outside the folder is reachable.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// A running server; stops when dropped (or with [`Server::stop`]).
pub struct Server {
    addr: SocketAddr,
    root: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Serves `root` on `127.0.0.1:port` (`0`: any free port) in the
    /// background: only this computer can connect.
    ///
    /// # Errors
    ///
    /// When `root` isn't a folder or the port can't be opened.
    pub fn start(root: &Path, port: u16) -> io::Result<Server> {
        Server::start_on(root, "127.0.0.1", port)
    }

    /// [`Server::start`] on another address: `0.0.0.0` lets phones and
    /// other computers on the network play too.
    ///
    /// # Errors
    ///
    /// When `root` isn't a folder or the address can't be opened.
    pub fn start_on(root: &Path, host: &str, port: u16) -> io::Result<Server> {
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a folder", root.display()),
            ));
        }
        let listener = TcpListener::bind((host, port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let root = root.to_path_buf();
        let thread = {
            let stop = Arc::clone(&stop);
            let root = root.clone();
            std::thread::Builder::new()
                .name("automata-serve".into())
                .spawn(move || accept_loop(&listener, &root, &stop))?
        };
        Ok(Server {
            addr,
            root,
            stop,
            thread: Some(thread),
        })
    }

    /// The address it listens on.
    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The folder it serves.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The page's URL (`http://127.0.0.1:<port>/`; `localhost` when
    /// serving every address).
    #[must_use]
    pub fn url(&self) -> String {
        if self.addr.ip().is_unspecified() {
            format!("http://localhost:{}/", self.addr.port())
        } else {
            format!("http://{}/", self.addr)
        }
    }

    /// Stops serving and waits for the server to finish.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Blocks until the server stops (it doesn't by itself: for the CLI).
    pub fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(listener: &TcpListener, root: &Path, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.to_path_buf();
                let _ = std::thread::Builder::new()
                    .name("automata-serve-conn".into())
                    .spawn(move || {
                        let _ = handle(stream, &root);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Answers one request (connections aren't kept open).
fn handle(stream: TcpStream, root: &Path) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(16 * 1024));
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    // Skip the headers.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut out = stream;
    if method != "GET" && method != "HEAD" {
        return respond(
            &mut out,
            "405 Method Not Allowed",
            "text/plain",
            b"GET only\n",
            true,
        );
    }
    let Some(path) = resolve(root, target) else {
        return respond(
            &mut out,
            "404 Not Found",
            "text/plain",
            b"not found\n",
            true,
        );
    };
    match std::fs::read(&path) {
        Ok(body) => respond(
            &mut out,
            "200 OK",
            content_type(&path),
            &body,
            method == "GET",
        ),
        Err(_) => respond(
            &mut out,
            "404 Not Found",
            "text/plain",
            b"not found\n",
            true,
        ),
    }
}

fn respond(
    out: &mut TcpStream,
    status: &str,
    kind: &str,
    body: &[u8],
    with_body: bool,
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    out.write_all(head.as_bytes())?;
    if with_body {
        out.write_all(body)?;
    }
    out.flush()
}

/// The file a request target names under `root`: the query dropped,
/// `%XX` decoded, folders mean their `index.html`. `None` for anything
/// that would leave `root` or doesn't exist.
fn resolve(root: &Path, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let decoded = percent_decode(path)?;
    let mut file = root.to_path_buf();
    for part in Path::new(decoded.trim_start_matches('/')).components() {
        match part {
            Component::Normal(p) => file.push(p),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if file.is_dir() {
        file.push("index.html");
    }
    file.is_file().then_some(file)
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().filter(|s| !s.contains('\0'))
}

/// The `Content-Type` for a file, by extension.
#[must_use]
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "wasm" => "application/wasm",
        "zip" | "whl" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "txt" | "py" | "ron" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(server: &Server, request: &str) -> (String, Vec<u8>) {
        let mut stream = TcpStream::connect(server.addr()).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut all = Vec::new();
        stream.read_to_end(&mut all).unwrap();
        let split = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (
            String::from_utf8_lossy(&all[..split]).into_owned(),
            all[split + 4..].to_vec(),
        )
    }

    #[test]
    fn serves_files_with_their_types_and_nothing_outside() {
        let dir = std::env::temp_dir().join(format!("automata-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("site/pyodide")).unwrap();
        std::fs::write(dir.join("site/index.html"), "<p>hi</p>").unwrap();
        std::fs::write(dir.join("site/pyodide/pyodide.mjs"), "export {}").unwrap();
        std::fs::write(dir.join("site/a b.wasm"), [0, 97, 115, 109]).unwrap();
        std::fs::write(dir.join("secret.txt"), "no").unwrap();
        let server = Server::start(&dir.join("site"), 0).unwrap();
        assert!(server.url().starts_with("http://127.0.0.1:"));

        let (head, body) = get(&server, "GET /?touch=1 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        assert!(head.contains("Content-Type: text/html"));
        assert_eq!(body, b"<p>hi</p>");
        let (head, _) = get(&server, "GET /pyodide/pyodide.mjs HTTP/1.1\r\n\r\n");
        assert!(head.contains("Content-Type: text/javascript"), "{head}");
        let (head, body) = get(&server, "GET /a%20b.wasm HTTP/1.1\r\n\r\n");
        assert!(
            head.contains("application/wasm") && body.len() == 4,
            "{head}"
        );
        let (head, body) = get(&server, "HEAD /index.html HTTP/1.1\r\n\r\n");
        assert!(
            head.contains("Content-Length: 9") && body.is_empty(),
            "{head}"
        );
        for outside in ["/../secret.txt", "/%2e%2e/secret.txt", "/missing.js"] {
            let (head, _) = get(&server, &format!("GET {outside} HTTP/1.1\r\n\r\n"));
            assert!(head.starts_with("HTTP/1.1 404"), "{outside}: {head}");
        }
        let (head, _) = get(&server, "POST / HTTP/1.1\r\n\r\n");
        assert!(head.starts_with("HTTP/1.1 405"), "{head}");
        server.stop();
        assert!(Server::start(&dir.join("nowhere"), 0).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
