//! The SDL3 window driver: an SDL window and event loop around the
//! engine's [`Host`].
//!
//! 1. Make a full-screen SDL window and hand its `ANativeWindow` to the
//!    engine, which creates the GPU surface on it.
//! 2. Translate SDL events (touch, keys, mouse, focus, resize) into the
//!    engine's input events.
//! 3. Run a frame when its deadline comes, then draw it.
//! 4. When Android sends the app to the background, let go of the surface
//!    (Android destroys it) and stop running frames; on return, attach the
//!    new one. Textures, meshes and the scene all stay in GPU memory.

use std::time::{Duration, Instant};

use automata_core::platform::{FrameStep, Host};
use automata_core::{Error, Result};
use automata_input::{ElementState, InputEvent, ModifiersState, TouchPhase};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle,
};
use sdl3::event::{Event, WindowEvent};
use sdl3::video::Window;
use tracing::info;

use crate::keys;

/// SDL's id for mouse events it made up from touches.
const TOUCH_MOUSE_ID: u32 = u32::MAX;
/// SDL's id for touches it made up from the mouse.
const MOUSE_TOUCH_ID: u64 = u64::MAX;

/// How long the loop sleeps at a time while frames are paused.
const PAUSED_WAIT: Duration = Duration::from_millis(250);

/// Runs a windowed game to completion with SDL.
pub(crate) fn run(host: &mut dyn Host) -> Result<()> {
    // Touches are touches: the engine already treats the mouse as finger
    // 0 when no finger is down, so neither kind should mimic the other.
    sdl3::hint::set("SDL_TOUCH_MOUSE_EVENTS", "0");
    sdl3::hint::set("SDL_MOUSE_TOUCH_EVENTS", "0");
    // Back is a key the game sees (`back_button`), not "close the app".
    sdl3::hint::set("SDL_ANDROID_TRAP_BACK_BUTTON", "1");
    // By default SDL blocks any event pump from the moment it queues the
    // background events until the app is back, so they only arrive on
    // return (unless handled in an event watch). The loop sleeps in short
    // slices while paused instead, and sees them right away. (SDL also
    // pauses its own audio only when blocking; the engine's audio isn't
    // SDL's.)
    sdl3::hint::set("SDL_ANDROID_BLOCK_ON_PAUSE", "0");

    let sdl = sdl3::init().map_err(platform)?;
    info!(
        block_on_pause = ?sdl3::hint::get("SDL_ANDROID_BLOCK_ON_PAUSE"),
        "SDL initialised"
    );
    let video = sdl.video().map_err(platform)?;
    let (width, height) = host.window_config().size;
    let mut builder = video.window(host.title(), width, height);
    builder.fullscreen().high_pixel_density();
    if host.window_config().resizable {
        builder.resizable();
    }
    let window = builder.build().map_err(platform)?;
    let mut events = sdl.event_pump().map_err(platform)?;

    let mut driver = SdlDriver {
        host,
        window,
        attached: false,
        foreground: true,
        focused: true,
        modifiers: ModifiersState::empty(),
        deadline: None,
    };
    driver.attach()?;
    info!("SDL window ready");

    loop {
        // Sleep until the next frame (or the next event). When frames
        // are paused, sleep until something happens.
        let event = if driver.active() {
            match driver.deadline {
                Some(deadline) => {
                    let wait = deadline.saturating_duration_since(Instant::now());
                    if wait.is_zero() {
                        None
                    } else {
                        events.wait_event_timeout(wait)
                    }
                }
                None => None,
            }
        } else {
            events.wait_event_timeout(PAUSED_WAIT)
        };
        for event in event.into_iter().chain(events.poll_iter()) {
            if driver.handle(event)? == Flow::Quit {
                return Ok(());
            }
        }
        if !driver.active() || driver.deadline.is_some_and(|d| Instant::now() < d) {
            continue;
        }
        match driver.host.frame() {
            FrameStep::Quit => return Ok(()),
            FrameStep::Next(deadline) => driver.deadline = deadline,
        }
        if driver.attached {
            driver.host.render()?;
        }
    }
}

/// Whether the event loop goes on.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
}

/// The driver's state for one run.
struct SdlDriver<'a> {
    host: &'a mut dyn Host,
    window: Window,
    /// Whether the engine has a surface on the window.
    attached: bool,
    /// Whether the app is on screen.
    foreground: bool,
    /// Whether the window has the focus. Losing it pauses frames too:
    /// Android takes it first when the app is leaving (and when the
    /// notification shade comes down, where mobile games pause anyway).
    focused: bool,
    modifiers: ModifiersState,
    /// When the next frame is due (`None`: now).
    deadline: Option<Instant>,
}

impl SdlDriver<'_> {
    /// Whether frames run: on screen and focused.
    fn active(&self) -> bool {
        self.foreground && self.focused
    }

    /// Gives the window's current native surface to the engine.
    fn attach(&mut self) -> Result<()> {
        if self.attached {
            return Ok(());
        }
        let Some(target) = NativeWindow::of(&self.window) else {
            // No surface yet; SDL says when there is one.
            return Ok(());
        };
        let size = self.window.size_in_pixels();
        self.host.attach_surface(target.into(), size)?;
        self.attached = true;
        Ok(())
    }

    /// Lets go of the surface before Android destroys it.
    fn detach(&mut self) {
        if self.attached {
            self.host.detach_surface();
            self.attached = false;
        }
    }

    /// Handles one SDL event.
    fn handle(&mut self, event: Event) -> Result<Flow> {
        match event {
            Event::Quit { .. } | Event::AppTerminating { .. } => return Ok(Flow::Quit),
            Event::AppWillEnterBackground { .. } | Event::AppDidEnterBackground { .. } => {
                if self.foreground {
                    info!("entering the background: pausing");
                }
                self.foreground = false;
                self.detach();
                // Releases that happen while away never arrive.
                self.host.input(&InputEvent::FocusLost);
            }
            Event::AppDidEnterForeground { .. } => {
                info!("back in the foreground: resuming");
                self.foreground = true;
                self.focused = true;
                self.deadline = None;
                self.attach()?;
            }
            Event::Window { win_event, .. } => return self.window_event(win_event),
            event => {
                if let Some(input) = self.input_event(&event) {
                    self.host.input(&input);
                }
            }
        }
        Ok(Flow::Continue)
    }

    /// Handles a window event.
    fn window_event(&mut self, event: WindowEvent) -> Result<Flow> {
        match event {
            WindowEvent::CloseRequested => return Ok(Flow::Quit),
            WindowEvent::PixelSizeChanged(width, height) => {
                let size = (
                    u32::try_from(width).unwrap_or(0),
                    u32::try_from(height).unwrap_or(0),
                );
                if self.attached {
                    self.host.resize(size.0, size.1);
                } else if self.foreground {
                    self.attach()?;
                }
            }
            WindowEvent::FocusLost => {
                if self.focused {
                    info!("focus lost: pausing frames");
                }
                self.focused = false;
                self.host.input(&InputEvent::FocusLost);
            }
            WindowEvent::FocusGained => {
                if !self.focused {
                    info!("focus back: running frames");
                }
                self.focused = true;
                self.deadline = None;
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// The engine input event for an SDL input event, if any.
    fn input_event(&mut self, event: &Event) -> Option<InputEvent> {
        let (scale_x, scale_y) = self.pixel_scale();
        Some(match *event {
            Event::FingerDown { .. }
            | Event::FingerMotion { .. }
            | Event::FingerUp { .. }
            | Event::FingerCanceled { .. } => return self.touch_event(event),
            Event::KeyDown { .. } | Event::KeyUp { .. } => return self.key_event(event),
            Event::MouseMotion {
                which,
                x,
                y,
                xrel,
                yrel,
                ..
            } => {
                if which == TOUCH_MOUSE_ID {
                    return None;
                }
                self.host.input(&InputEvent::MouseMotion {
                    dx: f64::from(xrel) * scale_x,
                    dy: f64::from(yrel) * scale_y,
                });
                InputEvent::CursorMoved {
                    x: f64::from(x) * scale_x,
                    y: f64::from(y) * scale_y,
                }
            }
            Event::MouseButtonDown {
                which, mouse_btn, ..
            }
            | Event::MouseButtonUp {
                which, mouse_btn, ..
            } => {
                if which == TOUCH_MOUSE_ID {
                    return None;
                }
                let state = if matches!(event, Event::MouseButtonDown { .. }) {
                    ElementState::Pressed
                } else {
                    ElementState::Released
                };
                InputEvent::MouseButton {
                    button: keys::mouse_button(mouse_btn)?,
                    state,
                }
            }
            Event::MouseWheel { which, x, y, .. } => {
                if which == TOUCH_MOUSE_ID {
                    return None;
                }
                InputEvent::Scroll {
                    x: f64::from(x),
                    y: f64::from(y),
                }
            }
            _ => return None,
        })
    }

    /// A finger event as an engine touch.
    fn touch_event(&self, event: &Event) -> Option<InputEvent> {
        let (phase, touch_id, finger_id, x, y) = match *event {
            Event::FingerDown {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => (TouchPhase::Started, touch_id, finger_id, x, y),
            Event::FingerMotion {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => (TouchPhase::Moved, touch_id, finger_id, x, y),
            Event::FingerUp {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => (TouchPhase::Ended, touch_id, finger_id, x, y),
            Event::FingerCanceled {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => (TouchPhase::Cancelled, touch_id, finger_id, x, y),
            _ => return None,
        };
        if touch_id == MOUSE_TOUCH_ID {
            return None;
        }
        // SDL's touch positions are 0..1 across the window.
        let (width, height) = self.window.size_in_pixels();
        Some(InputEvent::Touch {
            id: finger_id,
            phase,
            x: f64::from(x) * f64::from(width),
            y: f64::from(y) * f64::from(height),
        })
    }

    /// A key event as an engine key (after sending any modifier change).
    fn key_event(&mut self, event: &Event) -> Option<InputEvent> {
        let (scancode, keymod, repeat, state) = match *event {
            Event::KeyDown {
                scancode,
                keymod,
                repeat,
                ..
            } => (scancode, keymod, repeat, ElementState::Pressed),
            Event::KeyUp {
                scancode,
                keymod,
                repeat,
                ..
            } => (scancode, keymod, repeat, ElementState::Released),
            _ => return None,
        };
        let modifiers = keys::modifiers(keymod);
        if modifiers != self.modifiers {
            self.modifiers = modifiers;
            self.host.input(&InputEvent::Modifiers(modifiers));
        }
        if repeat {
            return None;
        }
        Some(InputEvent::Key {
            key: keys::key(scancode?)?,
            state,
        })
    }

    /// Physical pixels per SDL window coordinate, across and down.
    fn pixel_scale(&self) -> (f64, f64) {
        let (points_x, points_y) = self.window.size();
        let (pixels_x, pixels_y) = self.window.size_in_pixels();
        let ratio = |pixels: u32, points: u32| {
            if points == 0 {
                1.0
            } else {
                f64::from(pixels) / f64::from(points)
            }
        };
        (ratio(pixels_x, points_x), ratio(pixels_y, points_y))
    }
}

/// An `SdlError`-like value as the engine's platform error.
fn platform(error: impl std::fmt::Display) -> Error {
    Error::Platform(format!("SDL: {error}"))
}

/// The window's native surface, as wgpu takes it.
struct NativeWindow {
    window: RawWindowHandle,
    display: RawDisplayHandle,
}

impl NativeWindow {
    /// The window's current `ANativeWindow`, if Android has given it one
    /// (it hasn't while the app is in the background).
    fn of(window: &Window) -> Option<Self> {
        #[cfg(target_os = "android")]
        {
            use raw_window_handle::{AndroidDisplayHandle, AndroidNdkWindowHandle};
            use sdl3::sys::properties::SDL_GetPointerProperty;
            use sdl3::sys::video::{
                SDL_GetWindowProperties, SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER,
            };

            // SAFETY: `window.raw()` is a live SDL window for the duration
            // of the calls, and the property name is a NUL-terminated
            // constant. The result is only checked for null and stored.
            #[allow(unsafe_code)]
            let native = unsafe {
                SDL_GetPointerProperty(
                    SDL_GetWindowProperties(window.raw()),
                    SDL_PROP_WINDOW_ANDROID_WINDOW_POINTER,
                    std::ptr::null_mut(),
                )
            };
            let native = std::ptr::NonNull::new(native)?;
            Some(Self {
                window: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(native)),
                display: RawDisplayHandle::Android(AndroidDisplayHandle::new()),
            })
        }
        // Other platforms run the engine's own winit driver.
        #[cfg(not(target_os = "android"))]
        {
            let _ = window;
            None
        }
    }
}

// SAFETY: the handles are plain pointers to the `ANativeWindow`, which
// SDL keeps a reference to (and so alive) while the window has a surface.
// The engine drops its GPU surface on `detach`, before the app goes to the
// background, so the pointer is never used after Android replaces it.
#[allow(unsafe_code)]
unsafe impl Send for NativeWindow {}
// SAFETY: as for `Send`; the handles are only read.
#[allow(unsafe_code)]
unsafe impl Sync for NativeWindow {}

impl HasWindowHandle for NativeWindow {
    fn window_handle(&self) -> std::result::Result<WindowHandle<'_>, HandleError> {
        // SAFETY: see the `Send` impl: valid while the engine holds it.
        #[allow(unsafe_code)]
        Ok(unsafe { WindowHandle::borrow_raw(self.window) })
    }
}

impl HasDisplayHandle for NativeWindow {
    fn display_handle(&self) -> std::result::Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: Android's display handle carries no pointer.
        #[allow(unsafe_code)]
        Ok(unsafe { DisplayHandle::borrow_raw(self.display) })
    }
}
