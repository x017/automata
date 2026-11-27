//! The backend for the browser: the frame loop renders the mix, and the
//! page plays it.
//!
//! A browser's audio thread (an `AudioWorklet`) can't call into the game's
//! WebAssembly, so the mix is made where the game runs: each frame
//! [`StreamBackend::render`] mixes that frame's worth of sound into a
//! queue, and the page takes it with [`take_output`] and posts it to its
//! worklet, which plays it after a short buffer. Rendering follows frame
//! time like the silent backend, so playback positions and sound states
//! behave the same with or without speakers; while the page is hidden
//! (no frames), sound simply waits.

use std::collections::VecDeque;
use std::sync::Mutex;

use kira::backend::{Backend, Renderer};

/// The sample rate the browser mix runs at (the page opens its
/// `AudioContext` at the same rate).
pub const STREAM_SAMPLE_RATE: u32 = 48_000;

/// The most a frame renders (seconds): after a long stall (a hidden tab
/// coming back) sound resumes instead of rushing to catch up.
const MAX_FRAME_SECONDS: f64 = 0.1;

/// How much mixed sound waits for the page at most (seconds); older sound
/// is dropped, so a page that stops taking it doesn't grow the queue.
const MAX_QUEUED_SECONDS: f64 = 0.5;

/// Interleaved stereo samples waiting for the page.
static OUTPUT: Mutex<VecDeque<f32>> = Mutex::new(VecDeque::new());

/// Moves the mixed sound waiting for the page onto `out`: interleaved
/// stereo (left, right, left, …) at [`STREAM_SAMPLE_RATE`].
pub fn take_output(out: &mut Vec<f32>) {
    let mut queue = OUTPUT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    out.extend(queue.drain(..));
}

/// Settings for [`StreamBackend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBackendSettings {
    /// The mix's sample rate.
    pub sample_rate: u32,
}

impl Default for StreamBackendSettings {
    fn default() -> Self {
        Self {
            sample_rate: STREAM_SAMPLE_RATE,
        }
    }
}

/// A kira backend whose mix is rendered on demand into [`take_output`]'s
/// queue.
pub struct StreamBackend {
    sample_rate: u32,
    renderer: Option<Renderer>,
    buffer: Vec<f32>,
    /// The fraction of a sample frame left over from the last render.
    carry: f64,
}

impl Backend for StreamBackend {
    type Settings = StreamBackendSettings;
    type Error = ();

    fn setup(settings: Self::Settings, _internal_buffer_size: usize) -> Result<(Self, u32), ()> {
        Ok((
            Self {
                sample_rate: settings.sample_rate,
                renderer: None,
                buffer: Vec::new(),
                carry: 0.0,
            },
            settings.sample_rate,
        ))
    }

    fn start(&mut self, renderer: Renderer) -> Result<(), ()> {
        self.renderer = Some(renderer);
        Ok(())
    }
}

impl StreamBackend {
    /// Mixes `seconds` of sound (at most a tenth of a second) onto the
    /// output queue.
    pub fn render(&mut self, seconds: f64) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        let rate = f64::from(self.sample_rate);
        let wanted = seconds.min(MAX_FRAME_SECONDS) * rate + self.carry;
        let frames = wanted.floor();
        self.carry = wanted - frames;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        // 0 ≤ frames ≤ a tenth of a second of samples.
        let frames = frames as usize;
        if frames == 0 {
            return;
        }
        self.buffer.clear();
        self.buffer.resize(frames * 2, 0.0);
        renderer.on_start_processing();
        renderer.process(&mut self.buffer, 2);

        let mut queue = OUTPUT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.extend(self.buffer.iter().copied());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cap = (MAX_QUEUED_SECONDS * rate) as usize * 2;
        if queue.len() > cap {
            let extra = queue.len() - cap;
            queue.drain(..extra);
        }
    }
}
