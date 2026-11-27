//! The step driver: one frame of platform work per animation frame.
//!
//! 1. On the first frame, hand the canvas to the engine. Emscripten's EGL
//!    draws to Pyodide's canvas (`Module.canvas`) whatever the handle
//!    says, so wgpu's GL backend gets a placeholder web handle.
//! 2. When the page reports a new canvas size, resize the surface.
//! 3. Feed the input the page queued (and the gamepads it polled), then
//!    run and draw the frame.
//! 4. Note whether the game wants the pointer locked, for the page.

use std::cell::RefCell;

use automata_core::platform::{FrameStep, Host};
use automata_input::InputEvent;
// Gamepads come from the browser only in the real (emscripten) build; the
// host build exists for `cargo clippy`.
#[cfg(target_os = "emscripten")]
use automata_input::PadSnapshot;
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WebDisplayHandle, WebWindowHandle, WindowHandle,
};
use tracing::{error, info};

/// What the page told the driver since the last frame.
#[derive(Default)]
struct Page {
    /// The canvas's drawing size in pixels.
    size: (u32, u32),
    /// The size the surface has, once it's attached.
    attached: Option<(u32, u32)>,
    /// Input events queued by the page.
    events: Vec<InputEvent>,
    /// The gamepads the page saw this frame, if it polled.
    #[cfg(target_os = "emscripten")]
    pads: Option<Vec<PadSnapshot>>,
    /// The page was hidden: the time away isn't a frame.
    skip_time: bool,
    /// Whether the game wanted the pointer locked after the last frame.
    cursor_locked: bool,
    /// A failure to report to the page (the surface couldn't be made).
    error: Option<String>,
}

thread_local! {
    static PAGE: RefCell<Page> = RefCell::new(Page::default());
}

/// Records the canvas's drawing size for the next frame.
pub(crate) fn set_canvas_size(width: u32, height: u32) {
    PAGE.with(|page| page.borrow_mut().size = (width.max(1), height.max(1)));
}

/// Queues an input event for the next frame.
pub(crate) fn queue_input(event: InputEvent) {
    PAGE.with(|page| page.borrow_mut().events.push(event));
}

/// Hands over the gamepads the page polled for the next frame.
#[cfg(target_os = "emscripten")]
pub(crate) fn set_gamepads(pads: Vec<PadSnapshot>) {
    PAGE.with(|page| page.borrow_mut().pads = Some(pads));
}

/// The page was hidden and is back: the next frame starts time afresh.
pub(crate) fn skip_time() {
    PAGE.with(|page| page.borrow_mut().skip_time = true);
}

/// Whether the game wanted the pointer locked after the last frame.
pub(crate) fn cursor_locked() -> bool {
    PAGE.with(|page| page.borrow().cursor_locked)
}

/// The failure the last frame hit, if any.
pub(crate) fn take_error() -> Option<String> {
    PAGE.with(|page| page.borrow_mut().error.take())
}

/// One frame: see the module docs.
pub(crate) fn step(host: &mut dyn Host) -> FrameStep {
    let (size, attached, events, skip) = PAGE.with(|page| {
        let mut page = page.borrow_mut();
        let events = std::mem::take(&mut page.events);
        let skip = std::mem::take(&mut page.skip_time);
        (page.size, page.attached, events, skip)
    });
    match attached {
        None => {
            if let Err(err) = host.attach_surface(Canvas.into(), size) {
                error!("WebGL2 surface: {err}");
                PAGE.with(|page| page.borrow_mut().error = Some(format!("WebGL2: {err}")));
                return FrameStep::Quit;
            }
            info!(width = size.0, height = size.1, "canvas attached");
            PAGE.with(|page| page.borrow_mut().attached = Some(size));
        }
        Some(old) if old != size => {
            host.resize(size.0, size.1);
            PAGE.with(|page| page.borrow_mut().attached = Some(size));
        }
        Some(_) => {}
    }
    for event in &events {
        host.input(event);
    }
    #[cfg(target_os = "emscripten")]
    if let Some(pads) = PAGE.with(|page| page.borrow_mut().pads.take()) {
        host.feed_gamepads(&pads);
    }
    if skip {
        host.skip_time();
    }
    let step = host.frame();
    let locked = host.cursor_locked();
    PAGE.with(|page| page.borrow_mut().cursor_locked = locked);
    if let Err(err) = host.render() {
        error!("render: {err}");
    }
    step
}

/// Pyodide's canvas, as wgpu takes a window.
struct Canvas;

impl HasWindowHandle for Canvas {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: a web handle is a plain id, not a pointer; nothing is
        // borrowed (and emscripten's EGL ignores it).
        #[allow(unsafe_code)]
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Web(WebWindowHandle::new(1))) })
    }
}

impl HasDisplayHandle for Canvas {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: the web's display handle carries no pointer.
        #[allow(unsafe_code)]
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Web(WebDisplayHandle::new())) })
    }
}
