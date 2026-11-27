// The web player's sound output: an AudioWorklet that plays what the
// engine mixed. The engine renders each frame's sound where the game runs
// (`automata._core.web_audio()`: interleaved stereo float32 at the
// context's rate) and player.js posts it here; this plays it after a short
// buffer, filling gaps with silence, and keeps the delay bounded by
// dropping the oldest sound when too much piles up (a frame hitch, a tab
// coming back).

// Start (or restart after running dry) once this much is queued, or one
// and a half frames' worth when frames are slower than that (a phone, a
// software renderer), so the next frame's sound arrives in time.
const START_SECONDS = 0.05;
const START_MAX_SECONDS = 0.2;
// Never lag the game by more than this.
const MAX_SECONDS = 0.25;

class AutomataOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunks = []; // Float32Arrays, interleaved L R L R …
    this.offset = 0; // samples of chunks[0] already played
    this.queued = 0; // frames waiting
    this.playing = false;
    this.played = 0;
    this.dropped = 0;
    this.underruns = 0;
    this.lastReport = 0;
    this.frameFrames = 0; // how much sound a game frame brings, smoothed
    this.port.onmessage = (event) => this.push(event.data);
  }

  push(chunk) {
    if (!(chunk instanceof Float32Array) || chunk.length < 2) return;
    this.chunks.push(chunk);
    this.queued += chunk.length / 2;
    this.frameFrames = this.frameFrames * 0.9 + (chunk.length / 2) * 0.1;
    const max = MAX_SECONDS * sampleRate;
    while (this.queued > max && this.chunks.length > 1) {
      const old = this.chunks.shift();
      const left = (old.length - this.offset) / 2;
      this.queued -= left;
      this.dropped += left;
      this.offset = 0;
    }
  }

  process(_inputs, outputs) {
    const channels = outputs[0];
    const left = channels[0];
    const right = channels[1] ?? channels[0];
    const frames = left.length;
    const start = Math.min(Math.max(START_SECONDS * sampleRate, this.frameFrames * 1.5), START_MAX_SECONDS * sampleRate);
    if (!this.playing && this.queued >= start) this.playing = true;
    let i = 0;
    if (this.playing) {
      while (i < frames && this.chunks.length) {
        const chunk = this.chunks[0];
        while (i < frames && this.offset < chunk.length) {
          left[i] = chunk[this.offset];
          right[i] = chunk[this.offset + 1];
          this.offset += 2;
          i += 1;
        }
        if (this.offset >= chunk.length) {
          this.chunks.shift();
          this.offset = 0;
        }
      }
      this.queued -= i;
      this.played += i;
      if (i < frames) {
        this.playing = false;
        this.underruns += 1;
      }
    }
    for (; i < frames; i += 1) {
      left[i] = 0;
      right[i] = 0;
    }
    if (currentFrame - this.lastReport >= sampleRate / 4) {
      this.lastReport = currentFrame;
      this.port.postMessage({
        played: this.played,
        queued: this.queued,
        dropped: this.dropped,
        underruns: this.underruns,
        buffer: Math.round(start),
      });
    }
    return true;
  }
}

registerProcessor("automata-output", AutomataOutput);
