//! Engine logs to logcat, through SDL's log (tag `SDL/APP`).

use std::ffi::CString;
use std::io::{self, Write};

use tracing_subscriber::EnvFilter;

/// Sends `tracing` output to SDL's log. Wins over the engine's own stderr
/// logger, which only installs itself when no logger exists yet.
/// `AUTOMATA_LOG` overrides the filter.
pub(crate) fn init() {
    let filter = std::env::var("AUTOMATA_LOG")
        .ok()
        .and_then(|f| EnvFilter::try_new(f).ok())
        .unwrap_or_else(|| EnvFilter::new("info,wgpu_core=warn,wgpu_hal=warn,naga=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .without_time()
        .with_writer(SdlLine::default)
        .try_init();
}

/// One log record, written to SDL's log when it's dropped.
#[derive(Default)]
struct SdlLine(Vec<u8>);

impl Write for SdlLine {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for SdlLine {
    fn drop(&mut self) {
        while self.0.last().is_some_and(u8::is_ascii_whitespace) {
            self.0.pop();
        }
        self.0.retain(|&b| b != 0);
        if self.0.is_empty() {
            return;
        }
        let Ok(line) = CString::new(std::mem::take(&mut self.0)) else {
            return;
        };
        // SAFETY: both pointers are valid NUL-terminated strings for the
        // call, and the "%s" format consumes exactly the one argument.
        #[allow(unsafe_code)]
        unsafe {
            sdl3::sys::log::SDL_Log(c"%s".as_ptr(), line.as_ptr());
        }
    }
}
