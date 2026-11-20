//! Window presentation: the winit event loop and the glow blit of the UI.
//!
//! This module is the only place Oberon uses `unsafe`: creating an OpenGL
//! context and uploading the UI pixmap to the screen. All editor logic lives
//! in the safe modules (`ui`, `doc`, `render`).

// Stub — the full winit+glow shell is added after the UI core is verified.
