//! The system clipboard for the code editor.
//!
//! * X11: the `CLIPBOARD` selection through x11rb's pure-Rust connection.
//!   Copying starts a small thread that owns the selection and answers
//!   paste requests until another client takes it over.
//! * Wayland-only sessions: `wl-copy` / `wl-paste` when installed.
//! * Always: an in-process copy, so copy/paste inside Oberon works even
//!   without a display (tests, CI) or tools.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Clipboard access with graceful fallbacks.
#[derive(Debug, Default)]
pub struct Clipboard {
    internal: String,
    /// Whether our X11 owner thread still holds the selection.
    owned: Arc<AtomicBool>,
}

impl Clipboard {
    /// A clipboard handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies `text` to the system clipboard (and the in-process copy).
    pub fn set(&mut self, text: &str) {
        self.internal = text.to_string();
        if wayland_only() {
            let _ = pipe_to("wl-copy", &[], text);
            return;
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            self.owned = Arc::new(AtomicBool::new(false));
            x11::serve(text.to_string(), Arc::clone(&self.owned));
        }
    }

    /// The clipboard text (the system one when available).
    pub fn get(&mut self) -> String {
        if self.owned.load(Ordering::Relaxed) {
            return self.internal.clone();
        }
        if wayland_only() {
            if let Some(text) = run_capture("wl-paste", &["--no-newline"]) {
                return text;
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(text) = x11::read() {
            return text;
        }
        self.internal.clone()
    }
}

fn wayland_only() -> bool {
    std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn pipe_to(cmd: &str, args: &[&str], text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    // wl-copy forks to serve the selection; don't wait on it.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
mod x11 {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode,
        SelectionNotifyEvent, Window, WindowClass, SELECTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    type Res<T> = Result<T, Box<dyn std::error::Error>>;

    struct Atoms {
        clipboard: Atom,
        utf8: Atom,
        targets: Atom,
        property: Atom,
    }

    fn setup() -> Res<(RustConnection, Window, Atoms)> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        let win = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )?;
        let intern =
            |name: &[u8]| -> Res<Atom> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        let atoms = Atoms {
            clipboard: intern(b"CLIPBOARD")?,
            utf8: intern(b"UTF8_STRING")?,
            targets: intern(b"TARGETS")?,
            property: intern(b"OBERON_CLIPBOARD")?,
        };
        Ok((conn, win, atoms))
    }

    /// Takes ownership of CLIPBOARD on a background thread.
    pub fn serve(text: String, owned: Arc<AtomicBool>) {
        std::thread::spawn(move || {
            let _ = serve_inner(&text, &owned);
            owned.store(false, Ordering::Relaxed);
        });
    }

    fn serve_inner(text: &str, owned: &AtomicBool) -> Res<()> {
        let (conn, win, atoms) = setup()?;
        conn.set_selection_owner(win, atoms.clipboard, x11rb::CURRENT_TIME)?;
        conn.flush()?;
        if conn.get_selection_owner(atoms.clipboard)?.reply()?.owner != win {
            return Ok(());
        }
        owned.store(true, Ordering::Relaxed);
        loop {
            match conn.wait_for_event()? {
                Event::SelectionRequest(req) => {
                    // ICCCM: obsolete clients pass no property.
                    let mut property = if req.property == x11rb::NONE {
                        req.target
                    } else {
                        req.property
                    };
                    if req.target == atoms.targets {
                        conn.change_property32(
                            PropMode::REPLACE,
                            req.requestor,
                            property,
                            AtomEnum::ATOM,
                            &[atoms.targets, atoms.utf8, AtomEnum::STRING.into()],
                        )?;
                    } else if req.target == atoms.utf8 || req.target == u32::from(AtomEnum::STRING)
                    {
                        conn.change_property8(
                            PropMode::REPLACE,
                            req.requestor,
                            property,
                            req.target,
                            text.as_bytes(),
                        )?;
                    } else {
                        property = x11rb::NONE;
                    }
                    let notify = SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: req.time,
                        requestor: req.requestor,
                        selection: req.selection,
                        target: req.target,
                        property,
                    };
                    conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify)?;
                    conn.flush()?;
                }
                Event::SelectionClear(_) => return Ok(()),
                _ => {}
            }
        }
    }

    /// Reads CLIPBOARD as UTF-8 (`None` without a display or owner).
    pub fn read() -> Option<String> {
        read_inner().ok().flatten()
    }

    fn read_inner() -> Res<Option<String>> {
        let (conn, win, atoms) = setup()?;
        if conn.get_selection_owner(atoms.clipboard)?.reply()?.owner == x11rb::NONE {
            return Ok(None);
        }
        conn.convert_selection(
            win,
            atoms.clipboard,
            atoms.utf8,
            atoms.property,
            x11rb::CURRENT_TIME,
        )?;
        conn.flush()?;
        let deadline = Instant::now() + Duration::from_millis(400);
        while Instant::now() < deadline {
            if let Some(Event::SelectionNotify(ev)) = conn.poll_for_event()? {
                if ev.property == x11rb::NONE {
                    return Ok(None);
                }
                let reply = conn
                    .get_property(true, win, atoms.property, AtomEnum::ANY, 0, u32::MAX / 4)?
                    .reply()?;
                return Ok(Some(String::from_utf8_lossy(&reply.value).into_owned()));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_in_process_copy() {
        // Headless test environments have no display: the in-process copy
        // still round-trips.
        let mut c = Clipboard::new();
        c.set("print('hi')\n");
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            assert_eq!(c.get(), "print('hi')\n");
        }
    }
}
