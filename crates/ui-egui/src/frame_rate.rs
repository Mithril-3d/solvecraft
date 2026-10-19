//! The viewport's frame-rate readout (Preferences ▸ Graphics ▸ "Frame rate").

use egui::{FontId, Rect, Stroke, pos2, vec2};

use crate::SolveApp;
use crate::theme::Tokens;

/// Weight of the newest frame in the running mean (roughly the last 20 frames count).
const NEWEST: f64 = 0.05;

/// Frames per second, averaged over recent frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameRate {
    mean_dt: f64,
}

impl FrameRate {
    /// Takes the time since the previous frame, in seconds. A gap over a second (the app idle
    /// before the readout was turned on, a window being moved) is not a frame of a running view
    /// and is left out, as is anything that isn't a positive finite time.
    pub fn record(&mut self, dt: f64) {
        if !dt.is_finite() || dt <= 0.0 || dt > 1.0 {
            return;
        }
        self.mean_dt = if self.mean_dt > 0.0 { self.mean_dt + NEWEST * (dt - self.mean_dt) } else { dt };
    }

    /// The averaged rate; `None` before the first frame was recorded.
    pub fn fps(&self) -> Option<f64> {
        (self.mean_dt > 0.0).then(|| 1.0 / self.mean_dt)
    }

    /// The readout's text: rate and the matching time per frame.
    pub fn label(&self) -> Option<String> {
        self.fps().map(|fps| format!("{fps:.0} FPS · {:.1} ms", 1000.0 * self.mean_dt))
    }
}

/// Records this frame and paints the readout in the view's bottom-right corner. While it is on
/// the view redraws every frame, so the rate is what the view can draw rather than how often
/// something happened to change.
pub(crate) fn paint(app: &mut SolveApp, ctx: &egui::Context, painter: &egui::Painter, rect: Rect) {
    if !app.preferences.show_frame_rate {
        app.frame_rate = FrameRate::default();
        return;
    }
    app.frame_rate.record(f64::from(ctx.input(|i| i.unstable_dt)));
    ctx.request_repaint();
    let Some(text) = app.frame_rate.label() else { return };
    let t = Tokens::get();
    let font = FontId::monospace(12.0);
    let galley = painter.layout_no_wrap(text, font, t.text);
    let size = galley.size() + vec2(16.0, 8.0);
    let r = Rect::from_min_size(pos2(rect.right() - 12.0 - size.x, rect.bottom() - 12.0 - size.y), size);
    painter.rect(r, 4.0, t.overlay, Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    crate::scenario::publish_handle("viewport:frame_rate", r.center());
    painter.galley(r.center() - galley.size() / 2.0, galley, t.text);
}

#[cfg(test)]
mod tests {
    use super::FrameRate;

    #[test]
    fn steady_frames_read_their_rate() {
        let mut f = FrameRate::default();
        assert_eq!(f.fps(), None);
        for _ in 0..200 {
            f.record(1.0 / 60.0);
        }
        assert!((f.fps().unwrap_or(0.0) - 60.0).abs() < 1e-6);
        assert_eq!(f.label().as_deref(), Some("60 FPS · 16.7 ms"));
    }

    #[test]
    fn pauses_and_hostile_times_are_left_out() {
        let mut f = FrameRate::default();
        for dt in [0.0, -1.0, f64::NAN, f64::INFINITY, 5.0] {
            f.record(dt);
        }
        assert_eq!(f.fps(), None);
        f.record(0.01);
        f.record(3.0);
        assert!((f.fps().unwrap_or(0.0) - 100.0).abs() < 1e-6);
    }

    #[test]
    fn the_mean_follows_a_slower_view() {
        let mut f = FrameRate::default();
        f.record(1.0 / 120.0);
        for _ in 0..300 {
            f.record(1.0 / 30.0);
        }
        assert!((f.fps().unwrap_or(0.0) - 30.0).abs() < 0.1);
    }
}
