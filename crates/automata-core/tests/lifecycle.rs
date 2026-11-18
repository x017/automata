//! End-to-end lifecycle tests for the engine core.
//!
//! These exercise the public API exactly as a game (or the Python binding
//! layer) would consume it.

// The closures passed to `Application::run` guide lifetime inference for
// `AppContext<'_>`; clippy's suggested method-path rewrite does not typecheck.
#![allow(clippy::redundant_closure_for_method_calls)]

use automata_core::{AppState, Application, EngineConfig, Error};

/// A configuration without frame pacing so tests run at full speed.
fn test_config() -> EngineConfig {
    EngineConfig {
        target_fps: None,
        ..EngineConfig::new("lifecycle-test")
    }
}

#[test]
fn application_runs_frames_and_shuts_down_cleanly() {
    let mut app = Application::new(test_config());
    assert_eq!(app.state(), AppState::Created);

    let mut updates = 0_u64;
    app.run(|ctx| {
        updates += 1;
        if ctx.time().frame_count() >= 10 {
            ctx.request_quit();
        }
    })
    .expect("main loop should run to completion");

    assert_eq!(updates, 10);
    assert_eq!(app.state(), AppState::Shutdown);
    assert_eq!(app.time().frame_count(), 10);
}

#[test]
fn application_cannot_run_twice() {
    let mut app = Application::new(test_config());
    app.run(|ctx| ctx.request_quit())
        .expect("first run should succeed");

    let result = app.run(|_| {});
    assert!(
        matches!(result, Err(Error::InvalidState { .. })),
        "second run must report an invalid state, got: {result:?}"
    );
    assert_eq!(app.state(), AppState::Shutdown);
}

#[test]
fn delta_time_is_positive_after_the_first_frame() {
    let mut app = Application::new(test_config());

    app.run(|ctx| {
        if ctx.time().frame_count() >= 2 {
            assert!(ctx.time().delta_seconds() >= 0.0);
            ctx.request_quit();
        }
    })
    .expect("main loop should run to completion");

    assert_eq!(app.time().frame_count(), 2);
}
