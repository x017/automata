//! The Profiler tab of the bottom dock: where each frame's time goes
//! (scripts, physics, scene rendering, the editor's own interface), as a
//! rolling graph with current, average and worst figures.

use std::collections::VecDeque;

use crate::canvas::Canvas;

use super::{color, layout, ButtonStyle, CursorHint, Rect, Ui};

/// Frames kept in the graph (about four seconds at 60 fps).
pub const HISTORY: usize = 240;

/// One frame's timings in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameSample {
    /// Python scripts (as the runner measured its last answered frame).
    pub scripts: f32,
    /// Physics and animation stepping (play mode).
    pub physics: f32,
    /// Rendering the scene (and previews), plus presenting.
    pub render: f32,
    /// Drawing the editor interface.
    pub ui: f32,
    /// The whole frame, start to start (includes waiting for the display).
    pub total: f32,
}

impl FrameSample {
    /// The measured parts, in graph order.
    #[must_use]
    pub fn parts(&self) -> [f32; 4] {
        [self.scripts, self.physics, self.render, self.ui]
    }
}

/// The names of [`FrameSample::parts`].
pub const PART_NAMES: [&str; 4] = ["Scripts", "Physics", "Render", "Editor UI"];

fn part_color(i: usize) -> tiny_skia::Color {
    match i {
        0 => color::warn(),
        1 => color::axis_y(),
        2 => color::axis_z(),
        _ => tiny_skia::Color::from_rgba8(178, 132, 226, 255),
    }
}

/// Rolling frame timings.
#[derive(Debug, Default)]
pub struct Profiler {
    samples: VecDeque<FrameSample>,
    /// Stop recording (the graph holds still for a closer look).
    pub paused: bool,
    /// Finer timings inside a frame (panels, editor work), by name.
    sections: Vec<Section>,
}

/// One named slice of a frame, averaged as frames go by.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// What it times, as the Profiler lists it.
    pub name: &'static str,
    /// The latest frame's time (ms); 0 when it didn't run that frame.
    pub last: f32,
    /// A running average (ms) over roughly the last second.
    pub avg: f32,
    /// The worst frame (ms), slowly forgotten.
    pub max: f32,
    /// Time added this frame so far.
    pending: f32,
}

/// How fast section averages follow (per frame): about a second's worth.
const SECTION_SMOOTHING: f32 = 0.05;

/// Current, average and worst of one series.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Summary {
    /// The latest frame.
    pub last: f32,
    /// Mean over the history.
    pub avg: f32,
    /// Worst over the history.
    pub max: f32,
}

impl Profiler {
    /// Adds a frame (dropped while paused); keeps the last [`HISTORY`].
    pub fn record(&mut self, sample: FrameSample) {
        if self.paused {
            return;
        }
        if self.samples.len() == HISTORY {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// The recorded frames, oldest first.
    pub fn samples(&self) -> impl Iterator<Item = &FrameSample> {
        self.samples.iter()
    }

    /// Number of recorded frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// No frames recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Forgets the history.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.sections.clear();
    }

    /// Adds `ms` to this frame's time for `name` (a section may run more
    /// than once a frame). [`end_frame`](Self::end_frame) files it.
    pub fn add(&mut self, name: &'static str, ms: f32) {
        if self.paused {
            return;
        }
        match self.sections.iter_mut().find(|s| s.name == name) {
            Some(s) => s.pending += ms,
            None => self.sections.push(Section {
                name,
                last: 0.0,
                avg: 0.0,
                max: 0.0,
                pending: ms,
            }),
        }
    }

    /// Adds the time since `start` to `name` (see [`add`](Self::add)).
    pub fn since(&mut self, name: &'static str, start: std::time::Instant) {
        self.add(name, start.elapsed().as_secs_f32() * 1000.0);
    }

    /// Files this frame's section times into their averages.
    pub fn end_frame(&mut self) {
        if self.paused {
            return;
        }
        for s in &mut self.sections {
            s.last = std::mem::take(&mut s.pending);
            s.avg += (s.last - s.avg) * SECTION_SMOOTHING;
            s.max = (s.max * (1.0 - SECTION_SMOOTHING * 0.2)).max(s.last);
        }
    }

    /// The sections, busiest (by average) first.
    #[must_use]
    pub fn sections(&self) -> Vec<Section> {
        let mut out = self.sections.clone();
        out.sort_by(|a, b| b.avg.total_cmp(&a.avg));
        out
    }

    /// A plain-text summary to paste into a bug report: the frame parts
    /// and every section, with now / average / worst in milliseconds.
    #[must_use]
    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::from("Oberon profiler (ms: now / avg / max)\n");
        let line = |out: &mut String, name: &str, s: Summary| {
            let _ = writeln!(
                out,
                "{name:<34} {:>8.2} {:>8.2} {:>8.2}",
                s.last, s.avg, s.max
            );
        };
        for (i, name) in PART_NAMES.iter().enumerate() {
            line(&mut out, name, self.summary(|s| s.parts()[i]));
        }
        let frame = self.summary(|s| s.total);
        line(&mut out, "Frame", frame);
        if frame.avg > 0.0 {
            let _ = writeln!(out, "{:.0} fps", 1000.0 / frame.avg);
        }
        out.push_str("-- inside the frame --\n");
        for s in self.sections() {
            line(
                &mut out,
                s.name,
                Summary {
                    last: s.last,
                    avg: s.avg,
                    max: s.max,
                },
            );
        }
        out
    }

    /// Summary of one series (`pick` reads it from a sample).
    pub fn summary(&self, pick: impl Fn(&FrameSample) -> f32) -> Summary {
        let Some(last) = self.samples.back() else {
            return Summary::default();
        };
        let mut sum = 0.0;
        let mut max = 0.0_f32;
        for s in &self.samples {
            let v = pick(s);
            sum += v;
            max = max.max(v);
        }
        #[allow(clippy::cast_precision_loss)]
        let avg = sum / self.samples.len() as f32;
        Summary {
            last: pick(last),
            avg,
            max,
        }
    }

    /// The graph's vertical range: the next of 16.7/33.3/66.7/… ms above
    /// the busiest frame, so budget lines stay put.
    #[must_use]
    pub fn scale_ms(&self) -> f32 {
        let peak = self
            .samples
            .iter()
            .map(|s| s.total.max(s.parts().iter().sum()))
            .fold(0.0_f32, f32::max);
        let mut top = 1000.0 / 60.0;
        while top < peak && top < 2000.0 {
            top *= 2.0;
        }
        top
    }
}

impl Ui {
    /// The Profiler tab's toolbar (right side of the dock's tab strip).
    pub(super) fn profiler_toolbar(&mut self, canvas: &mut Canvas, strip: Rect) {
        let h = strip.h - 8.0;
        let y = strip.y + 4.0;
        let clear = Rect::new(strip.x + strip.w - 64.0, y, 56.0, h);
        if self.button(canvas, clear, "Clear", ButtonStyle::Ghost) {
            self.profiler.clear();
        }
        let label = if self.profiler.paused {
            "Resume"
        } else {
            "Pause"
        };
        let pause = Rect::new(clear.x - 72.0, y, 68.0, h);
        self.tip(pause, "Hold the graph still");
        let style = if self.profiler.paused {
            ButtonStyle::Active
        } else {
            ButtonStyle::Ghost
        };
        if self.button(canvas, pause, label, style) {
            self.profiler.paused = !self.profiler.paused;
        }
        let copy = Rect::new(pause.x - 64.0, y, 60.0, h);
        self.tip(copy, "Copy the timings as text (for a bug report)");
        if self.button(canvas, copy, "Copy", ButtonStyle::Ghost) {
            let report = self.profiler.report();
            self.code.clipboard.set(&report);
            self.log(super::LogLevel::Info, "Copied the Profiler's timings");
        }
    }

    /// The Profiler's middle column: the busiest slices of the frame
    /// (panels, editor work, presenting), by average time. Returns its
    /// width (0 when there's no room).
    fn profiler_sections(&mut self, canvas: &mut Canvas, rect: Rect, x: f32) -> f32 {
        const W: f32 = 290.0;
        if rect.w < 900.0 {
            return 0.0;
        }
        let pad = layout::PAD;
        let row_h = layout::ROW_H.min((rect.h - 2.0 * pad) / 6.0).max(14.0);
        let mut y = rect.y + pad;
        let cols = [x + W - 116.0, x + W - 60.0];
        canvas.text(
            x,
            y,
            layout::SMALL_SIZE,
            "Inside the frame",
            color::text_faint(),
        );
        for (cx, head) in cols.iter().zip(["avg", "max"]) {
            canvas.text(*cx, y, layout::SMALL_SIZE, head, color::text_faint());
        }
        y += row_h;
        let sections = self.profiler.sections();
        if sections.is_empty() {
            canvas.text(x, y, layout::SMALL_SIZE, "—", color::text_faint());
        }
        for s in sections {
            if y + row_h > rect.y + rect.h {
                break;
            }
            canvas.text_clipped(
                x,
                y,
                layout::SMALL_SIZE,
                s.name,
                color::text_dim(),
                cols[0] - x - 6.0,
            );
            for (cx, v) in cols.iter().zip([s.avg, s.max]) {
                canvas.text(
                    *cx,
                    y,
                    layout::SMALL_SIZE,
                    &format!("{v:.2}"),
                    color::text_dim(),
                );
            }
            y += row_h;
        }
        W + pad
    }

    /// The Profiler tab: a legend with figures on the left, the stacked
    /// per-frame graph on the right.
    pub(super) fn profiler_body(&mut self, canvas: &mut Canvas, rect: Rect) {
        if rect.h < 24.0 {
            return;
        }
        let pad = layout::PAD;
        let legend_w = 250.0_f32.min(rect.w * 0.45);
        let row_h = layout::ROW_H.min((rect.h - 2.0 * pad) / 6.0).max(14.0);
        let mut y = rect.y + pad;
        let x = rect.x + pad;
        let cols = [x + 104.0, x + 152.0, x + 200.0];
        for (cx, head) in cols.iter().zip(["now", "avg", "max"]) {
            canvas.text(*cx, y, layout::SMALL_SIZE, head, color::text_faint());
        }
        y += row_h;
        let mut rows: Vec<(String, tiny_skia::Color, Summary)> = (0..4)
            .map(|i| {
                (
                    PART_NAMES[i].to_string(),
                    part_color(i),
                    self.profiler.summary(|s| s.parts()[i]),
                )
            })
            .collect();
        rows.push((
            "Frame".to_string(),
            color::text(),
            self.profiler.summary(|s| s.total),
        ));
        for (i, (name, c, sum)) in rows.iter().enumerate() {
            if y + row_h > rect.y + rect.h {
                break;
            }
            if i < 4 {
                canvas.fill_rounded(x, y + 3.0, 9.0, 9.0, 2.0, *c);
            }
            let fg = if i < 4 {
                color::text_dim()
            } else {
                color::text()
            };
            canvas.text(x + 16.0, y, layout::SMALL_SIZE, name, fg);
            for (cx, v) in cols.iter().zip([sum.last, sum.avg, sum.max]) {
                canvas.text(*cx, y, layout::SMALL_SIZE, &format!("{v:.2}"), fg);
            }
            y += row_h;
        }
        let fps = self.profiler.summary(|s| s.total).avg;
        if fps > 0.0 && y + row_h <= rect.y + rect.h {
            canvas.text(
                x + 16.0,
                y,
                layout::SMALL_SIZE,
                &format!("{:.0} fps · ms per frame", 1000.0 / fps),
                color::text_faint(),
            );
        }

        let middle = self.profiler_sections(canvas, rect, rect.x + legend_w + pad);
        let legend_w = legend_w + middle;

        // --- Graph ---
        let g = Rect::new(
            rect.x + legend_w + pad,
            rect.y + pad,
            (rect.w - legend_w - 2.0 * pad).max(0.0),
            (rect.h - 2.0 * pad).max(0.0),
        );
        if g.w < 40.0 || g.h < 20.0 {
            return;
        }
        canvas.fill_rounded(g.x, g.y, g.w, g.h, layout::RADIUS, color::field());
        let top = self.profiler.scale_ms();
        let to_y = |ms: f32| g.y + g.h - (ms / top).min(1.0) * g.h;
        // Budget lines: 60 fps and 30 fps (and beyond as the scale grows).
        let mut line = 1000.0 / 60.0;
        while line <= top + 0.01 {
            let ly = to_y(line).round();
            if ly > g.y + 2.0 {
                canvas.fill_rect(g.x, ly, g.w, 1.0, color::border());
                let fps = (1000.0 / line).round();
                canvas.text(
                    g.x + 4.0,
                    ly + 2.0,
                    layout::SMALL_SIZE,
                    &format!("{line:.1} ms · {fps} fps"),
                    color::text_faint(),
                );
            }
            line *= 2.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let bar = g.w / HISTORY as f32;
        let n = self.profiler.len();
        let hover = self.hot(g).then(|| {
            let from_right = (g.x + g.w - self.p.x) / bar;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let back = from_right.max(0.0) as usize;
            n.checked_sub(back + 1)
        });
        let hover = hover.flatten();
        let samples: Vec<FrameSample> = self.profiler.samples().copied().collect();
        for (i, s) in samples.iter().enumerate() {
            // Edges on whole pixels, so neighbouring bars and stacked parts
            // meet without seams.
            #[allow(clippy::cast_precision_loss)]
            let x0 = (g.x + g.w - (n - i) as f32 * bar).round();
            #[allow(clippy::cast_precision_loss)]
            let x1 = (g.x + g.w - (n - i - 1) as f32 * bar).round().max(x0 + 1.0);
            let mut acc = 0.0;
            for (k, v) in s.parts().iter().enumerate() {
                if *v <= 0.0 {
                    continue;
                }
                let (y0, y1) = (to_y(acc).round(), to_y(acc + v).round());
                let mut c = part_color(k);
                if hover.is_some_and(|h| h != i) {
                    c.apply_opacity(0.55);
                }
                if y0 > y1 {
                    canvas.fill_rect(x0, y1, x1 - x0, y0 - y1, c);
                }
                acc += v;
            }
            // The whole frame as a faint cap above the measured parts.
            let ty = to_y(s.total).round();
            canvas.fill_rect(x0, ty, x1 - x0, 1.0, color::text_faint());
        }
        if n == 0 {
            canvas.text(
                g.x + 10.0,
                g.y + g.h / 2.0 - 7.0,
                layout::SMALL_SIZE,
                "Frame timings appear here as the editor runs.",
                color::text_faint(),
            );
        }
        if let Some(i) = hover {
            let s = samples[i];
            self.cursor = CursorHint::Default;
            let text = format!(
                "scripts {:.2} · physics {:.2} · render {:.2} · ui {:.2} · frame {:.2} ms",
                s.scripts, s.physics, s.render, s.ui, s.total
            );
            let tw = canvas.measure_text(layout::SMALL_SIZE, &text) + 12.0;
            let tx = (g.x + g.w - tw - 4.0).max(g.x);
            canvas.fill_rounded(tx, g.y + 4.0, tw, 20.0, layout::RADIUS, color::raised());
            canvas.text(
                tx + 6.0,
                g.y + 7.0,
                layout::SMALL_SIZE,
                &text,
                color::text(),
            );
        }
        canvas.stroke_rect(g.x, g.y, g.w, g.h, color::border(), 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(total: f32) -> FrameSample {
        FrameSample {
            scripts: 1.0,
            physics: 2.0,
            render: 3.0,
            ui: 4.0,
            total,
        }
    }

    #[test]
    fn keeps_a_bounded_history_and_summarises_it() {
        let mut p = Profiler::default();
        assert!(p.is_empty());
        for i in 0..(HISTORY + 10) {
            #[allow(clippy::cast_precision_loss)]
            p.record(sample(10.0 + i as f32));
        }
        assert_eq!(p.len(), HISTORY);
        let s = p.summary(|s| s.total);
        #[allow(clippy::cast_precision_loss)]
        let last = 10.0 + (HISTORY + 9) as f32;
        assert!((s.last - last).abs() < 1e-3);
        assert!((s.max - last).abs() < 1e-3);
        assert!(s.avg < s.max);
        assert!((p.summary(|s| s.physics).avg - 2.0).abs() < 1e-4);
        // Paused: nothing is recorded.
        p.paused = true;
        p.record(sample(999.0));
        assert!((p.summary(|s| s.total).last - last).abs() < 1e-3);
        p.clear();
        assert!(p.is_empty());
    }

    #[test]
    fn sections_average_per_frame_and_report_busiest_first() {
        let mut p = Profiler::default();
        assert_eq!(p.sections(), Vec::<Section>::new());
        for _ in 0..200 {
            p.add("UI: inspector", 1.0);
            p.add("UI: inspector", 1.0); // twice in a frame adds up
            p.add("Scene render", 8.0);
            p.end_frame();
        }
        let s = p.sections();
        assert_eq!(s[0].name, "Scene render", "busiest first");
        assert!((s[0].avg - 8.0).abs() < 0.01);
        assert!((s[1].last - 2.0).abs() < 1e-4);
        assert!((s[1].avg - 2.0).abs() < 0.01);
        // A frame without the section counts as zero.
        p.end_frame();
        assert!(p.sections().iter().all(|s| s.last == 0.0));
        let report = p.report();
        assert!(report.contains("Scene render"));
        assert!(report.contains("Editor UI"));
        // Paused: nothing moves.
        p.paused = true;
        p.add("Scene render", 500.0);
        p.end_frame();
        assert!(p.sections()[0].max < 9.0);
        p.clear();
        assert_eq!(p.sections(), Vec::<Section>::new());
    }

    #[test]
    fn scale_steps_through_frame_budgets() {
        let mut p = Profiler::default();
        assert!((p.scale_ms() - 1000.0 / 60.0).abs() < 1e-3);
        p.record(sample(20.0));
        assert!((p.scale_ms() - 2000.0 / 60.0).abs() < 1e-3);
        p.record(sample(50.0));
        assert!((p.scale_ms() - 4000.0 / 60.0).abs() < 1e-3);
    }
}
