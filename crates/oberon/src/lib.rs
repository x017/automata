//! Oberon: the visual editor for the AUTOMATA game engine.
//!
//! A native Rust application: a winit window, a tiny-skia-rendered editor
//! UI, and the AUTOMATA engine's own wgpu renderer producing the live
//! viewport frame (composited into the UI each frame).
//!
//! The crate is a library plus a binary. The library holds the testable
//! pieces (scene document model, inspector property tables, scene-view math,
//! UI, viewport compositing) so they can be exercised headless; the binary
//! adds the winit shell.
//!
//! # Safety
//!
//! Every module declares `#![forbid(unsafe_code)]`; the workspace forbids
//! `unsafe` too. Presentation goes through winit and the engine's wgpu
//! surface, which need no `unsafe` in this crate.

// The editor UI is coordinate- and projection-heavy: it deals in f32 pixel
// positions, per-channel RGBA bytes, and draw helpers that take a rect plus a
// color. The pedantic lints that would only add noise here are relaxed
// crate-wide (the workspace `unsafe_code = "forbid"` and correctness lints
// still apply in full).
#![allow(
    clippy::many_single_char_names,
    clippy::too_many_arguments,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::cast_possible_wrap,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::unused_self,
    clippy::too_many_lines,
    clippy::struct_excessive_bools,
    clippy::single_match_else
)]

pub mod bridge;
pub mod build;
pub mod canvas;
pub mod code;
pub mod doc;
pub mod fields;
pub mod json;
pub mod lsp;
pub mod models;
pub mod play;
pub mod present;
pub mod project;
pub mod render;
pub mod sounds;
pub mod text;
pub mod theme;
pub mod ui;
pub mod view;
