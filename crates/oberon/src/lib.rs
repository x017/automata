//! Oberon: the visual editor for the AUTOMATA game engine.
//!
//! A native Rust application: a winit window, a tiny-skia-rendered editor
//! UI, and the AUTOMATA engine's own wgpu renderer producing the live
//! viewport frame (composited into the UI each frame).
//!
//! The crate is a binary plus a library. The library holds the testable
//! pieces (scene document model, UI rendering, viewport compositing) so
//! they can be exercised headless; the binary adds the winit/glow shell.
//!
//! # Safety
//!
//! Oberon presents a window via `glow`, whose GL bindings are `unsafe` FFI.
//! The engine's own crates forbid `unsafe`, but a windowed application shell
//! cannot avoid it; all `unsafe` is contained to [`present`], which opts out
//! of the crate-level `forbid` with a scoped `allow`.

#![forbid(unsafe_code)]

pub mod canvas;
pub mod doc;
pub mod render;
pub mod text;
pub mod theme;
pub mod ui;

/// Window presentation.
///
/// The editor renders into an in-memory RGBA buffer (`canvas`). Because this
/// crate forbids `unsafe`, Oberon does not link an OpenGL blitter itself; the
/// [`present`] module exposes the buffer and the winit integration, and the
/// window surface upload happens through safe APIs only.
pub mod present {
    //! The winit event loop and frame delivery. The UI pixmap is handed to
    //! the platform through safe APIs (see the binary); this module holds the
    //! safe event-loop glue shared with tests.

    // Stub — the winit shell is added after the UI core is verified.
}
