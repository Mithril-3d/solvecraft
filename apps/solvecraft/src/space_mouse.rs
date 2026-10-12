//! Windows 3Dconnexion HID navigation. The device reader never touches the document or UI;
//! it publishes the latest axes, and the egui thread applies them once per frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Context;
use hidapi::{DeviceInfo, HidApi};
use solvecraft_engine::render::Camera;
use solvecraft_ui_egui::SolveApp;

const STALE_AFTER: Duration = Duration::from_millis(150);
const RETRY_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Axes {
    translation: [i16; 3],
    rotation: [i16; 3],
}

#[derive(Clone, Copy)]
struct Reading {
    axes: Axes,
    translation_at: Instant,
    rotation_at: Instant,
}

impl Reading {
    fn new(now: Instant) -> Self {
        Self { axes: Axes::default(), translation_at: now, rotation_at: now }
    }

    fn current(self, now: Instant) -> Axes {
        Axes {
            translation: if now.duration_since(self.translation_at) < STALE_AFTER { self.axes.translation } else { [0; 3] },
            rotation: if now.duration_since(self.rotation_at) < STALE_AFTER { self.axes.rotation } else { [0; 3] },
        }
    }
}

/// The old SpaceNavigator uses Logitech's VID; newer 3Dconnexion devices use their own.
/// The HID usage excludes ordinary Logitech mice and the keyboard interfaces of SpaceMice.
fn is_3d_mouse(info: &DeviceInfo) -> bool {
    matches!(info.vendor_id(), 0x046d | 0x256f) && info.usage_page() == 0x01 && info.usage() == 0x08
}

fn three_axes(bytes: &[u8]) -> Option<[i16; 3]> {
    let &[x0, x1, y0, y1, z0, z1] = bytes.get(..6)? else { return None };
    Some([i16::from_le_bytes([x0, x1]), i16::from_le_bytes([y0, y1]), i16::from_le_bytes([z0, z1])])
}

/// SpaceNavigator reports 1 (translation) and 2 (rotation), each with three signed
/// little-endian 16-bit values. The HID layer may pad either report to its maximum size.
fn decode_report(report: &[u8], reading: &mut Reading, now: Instant) -> bool {
    match report.first().copied() {
        Some(1) => {
            let Some(translation) = report.get(1..).and_then(three_axes) else { return false };
            reading.axes.translation = translation;
            reading.translation_at = now;
            true
        }
        Some(2) => {
            let Some(rotation) = report.get(1..).and_then(three_axes) else { return false };
            reading.axes.rotation = rotation;
            reading.rotation_at = now;
            true
        }
        _ => false,
    }
}

fn reader(ctx: Context, shared: Arc<Mutex<Reading>>) {
    let Ok(mut api) = HidApi::new() else {
        log::warn!("3D mouse: HID initialization failed");
        return;
    };
    loop {
        if let Err(error) = api.refresh_devices() {
            log::warn!("3D mouse: device scan failed: {error}");
        } else if let Some(device) = api.device_list().find(|info| is_3d_mouse(info)).and_then(|info| info.open_device(&api).ok()) {
            let mut report = [0_u8; 64];
            loop {
                match device.read_timeout(&mut report, 100) {
                    Ok(0) => {}
                    Ok(size) => {
                        if let Ok(mut reading) = shared.lock()
                            && decode_report(&report[..size], &mut reading, Instant::now())
                        {
                            ctx.request_repaint();
                        }
                    }
                    Err(error) => {
                        log::info!("3D mouse disconnected: {error}");
                        break;
                    }
                }
            }
            if let Ok(mut reading) = shared.lock() {
                *reading = Reading::new(Instant::now());
            }
            ctx.request_repaint();
        }
        std::thread::sleep(RETRY_AFTER);
    }
}

pub struct SpaceMouse {
    shared: Arc<Mutex<Reading>>,
    last_frame: Instant,
}

impl SpaceMouse {
    pub fn start(ctx: &Context) -> Self {
        let now = Instant::now();
        let shared = Arc::new(Mutex::new(Reading::new(now)));
        let worker_state = Arc::clone(&shared);
        let worker_ctx = ctx.clone();
        if let Err(error) = std::thread::Builder::new().name("solvecraft-3d-mouse".into()).spawn(move || reader(worker_ctx, worker_state)) {
            log::warn!("3D mouse: reader could not start: {error}");
        }
        Self { shared, last_frame: now }
    }

    pub fn tick(&mut self, app: &mut SolveApp, ctx: &Context) {
        let now = Instant::now();
        let seconds = now.duration_since(self.last_frame).as_secs_f64().min(0.05);
        self.last_frame = now;
        if !app.preferences.space_mouse_enabled || app.home.open {
            return;
        }
        let axes = match self.shared.lock() {
            Ok(reading) => reading.current(now),
            Err(_) => return,
        };
        if axes == Axes::default() {
            return;
        }
        let speed = f64::from(app.preferences.space_mouse_speed);
        if !speed.is_finite() {
            return;
        }
        app.begin_external_navigation();
        let height = app.viewport.rect.map_or(600.0, |rect| f64::from(rect.height().max(1.0)));
        navigate(&mut app.cam, axes, seconds * speed.clamp(0.1, 5.0), height, app.session.active_sketch.is_some());
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

/// A small dead zone avoids drift at rest. A full cap deflection takes roughly one viewport
/// per second to pan and about two radians per second to turn, independent of frame rate.
fn normalized(value: i16) -> f64 {
    let v = f64::from(value);
    if v.abs() <= 20.0 { 0.0 } else { v.signum() * ((v.abs() - 20.0) / 330.0).min(1.0) }
}

fn navigate(cam: &mut Camera, axes: Axes, seconds: f64, height: f64, sketch: bool) {
    if !(seconds > 0.0) || !seconds.is_finite() {
        return;
    }
    let [x, y, z] = axes.translation.map(normalized);
    let [rx, ry, rz] = axes.rotation.map(normalized);
    cam.pan(x * height * 0.6 * seconds, -y * height * 0.6 * seconds, height);
    cam.zoom_at((-z * 1.5 * seconds).exp(), None);
    if !sketch {
        cam.orbit(ry * 250.0 * seconds, rx * 250.0 * seconds);
        cam.roll_by(rz * 2.0 * seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_separate_reports_without_stale_motion_or_padding_interference() {
        let now = Instant::now();
        let mut reading = Reading::new(now);
        assert!(!decode_report(&[1, 1, 2], &mut reading, now));
        assert_eq!(reading.current(now), Axes::default());
        assert!(decode_report(&[1, 0x9c, 0xff, 0x2c, 0x01, 0, 0], &mut reading, now));
        assert_eq!(reading.current(now).translation, [-100, 300, 0]);
        assert!(decode_report(&[2, 0, 0, 1, 0, 0xff, 0xff], &mut reading, now));
        assert_eq!(reading.current(now).rotation, [0, 1, -1]);
        let later = now + STALE_AFTER + Duration::from_millis(1);
        assert_eq!(reading.current(later), Axes::default());
        assert!(decode_report(&[2, 4, 0, 5, 0, 6, 0], &mut reading, later));
        assert!(decode_report(&[1, 1, 0, 2, 0, 3, 0, 0, 0, 0, 0, 0, 0], &mut reading, later));
        assert_eq!(reading.current(later), Axes { translation: [1, 2, 3], rotation: [4, 5, 6] });
    }

    #[test]
    fn six_axes_move_the_camera_and_sketches_keep_their_orientation() {
        let mut cam = Camera::default();
        let before = cam;
        navigate(&mut cam, Axes { translation: [350, 350, 350], rotation: [350, 350, 350] }, 0.02, 600.0, false);
        assert_ne!(cam.target, before.target);
        assert!(cam.distance < before.distance);
        assert_ne!(cam.yaw, before.yaw);
        assert_ne!(cam.pitch, before.pitch);
        assert_ne!(cam.roll, before.roll);
        let orientation = (cam.yaw, cam.pitch, cam.roll);
        navigate(&mut cam, Axes { translation: [350, 0, 0], rotation: [350, 350, 350] }, 0.02, 600.0, true);
        assert_eq!((cam.yaw, cam.pitch, cam.roll), orientation);
        assert!(cam.is_valid());
    }
}
