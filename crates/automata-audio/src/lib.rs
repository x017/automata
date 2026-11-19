//! AUTOMATA audio: mixing and playback via kira.
//!
//! Two backends exist: a real device backend (`cpal` feature) and a silent
//! backend that advances playheads without touching audio hardware — used
//! for headless runs, CI, and machines with no output device. Full
//! implementation lands with the vendored kira dependency.
