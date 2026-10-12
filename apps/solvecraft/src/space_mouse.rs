//! Windows 3Dconnexion HID navigation. The device reader never touches the document or UI;
//! it publishes the latest axes, and the egui thread applies them once per frame.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Context;
use hidapi::{DeviceInfo, HidApi};
use quick_xml::Reader;
use quick_xml::events::Event;
use solvecraft_engine::render::Camera;
use solvecraft_ui_egui::SolveApp;

const STALE_AFTER: Duration = Duration::from_millis(150);
const RETRY_AFTER: Duration = Duration::from_secs(2);
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Axes {
    translation: [i16; 3],
    rotation: [i16; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisRule {
    output: usize,
    enabled: bool,
    reversed: bool,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DriverMapping {
    axes: [AxisRule; 6],
    overall_scale: f64,
}

impl Default for DriverMapping {
    fn default() -> Self {
        Self { axes: std::array::from_fn(|output| AxisRule { output, enabled: true, reversed: false, scale: 1.0 }), overall_scale: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Motion {
    translation: [f64; 3],
    rotation: [f64; 3],
}

impl DriverMapping {
    fn apply(self, axes: Axes) -> Motion {
        let raw = [
            normalized(axes.translation[0]),
            normalized(axes.translation[1]),
            normalized(axes.translation[2]),
            normalized(axes.rotation[0]),
            normalized(axes.rotation[1]),
            normalized(axes.rotation[2]),
        ];
        let mut logical = [0.0; 6];
        for (input, rule) in self.axes.iter().enumerate() {
            if rule.enabled {
                let sign = if rule.reversed { -1.0 } else { 1.0 };
                logical[rule.output] = raw[input] * sign * rule.scale * self.overall_scale;
            }
        }
        Motion { translation: [logical[0], logical[1], logical[2]], rotation: [logical[3], logical[4], logical[5]] }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProfileAxis {
    input: Option<usize>,
    output: Option<usize>,
    enabled: bool,
    reversed: bool,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ProfilePart {
    Input,
    Output,
}

fn axis_index(name: &str) -> Option<usize> {
    match name.trim() {
        "HIDMultiAxis_X" => Some(0),
        "HIDMultiAxis_Y" => Some(1),
        "HIDMultiAxis_Z" => Some(2),
        "HIDMultiAxis_Rx" => Some(3),
        "HIDMultiAxis_Ry" => Some(4),
        "HIDMultiAxis_Rz" => Some(5),
        _ => None,
    }
}

/// 3DxWare stores the active KMJ application's axis selection, swapping, inversion and speed in
/// `%APPDATA%/3Dconnexion/3DxWare/Cfg/<app>-KMJ.xml`. Direct HID reads bypass the driver, so apply
/// that profile explicitly when it exists. Unknown or incomplete fields retain the raw HID axis.
fn parse_driver_mapping(xml: &[u8]) -> Option<DriverMapping> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut mapping = DriverMapping::default();
    let mut axis: Option<ProfileAxis> = None;
    let mut part = None;
    let mut field = Vec::new();
    let mut found = false;
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(start) => match start.local_name().as_ref() {
                b"Axis" => axis = Some(ProfileAxis { enabled: true, scale: 1.0, ..Default::default() }),
                b"Input" => part = Some(ProfilePart::Input),
                b"Output" => part = Some(ProfilePart::Output),
                name @ (b"ActionID" | b"Enabled" | b"Reversed" | b"Scale" | b"OverallScale") => field = name.to_vec(),
                _ => {}
            },
            Event::Text(text) => {
                let value = String::from_utf8_lossy(text.as_ref());
                match field.as_slice() {
                    b"ActionID" => {
                        if let Some(current) = axis.as_mut() {
                            match part {
                                Some(ProfilePart::Input) => current.input = axis_index(&value),
                                Some(ProfilePart::Output) => current.output = axis_index(&value),
                                None => {}
                            }
                        }
                    }
                    b"Enabled" => {
                        if let Some(current) = axis.as_mut() {
                            current.enabled = value.trim().eq_ignore_ascii_case("true");
                        }
                    }
                    b"Reversed" => {
                        if part == Some(ProfilePart::Output)
                            && let Some(current) = axis.as_mut()
                        {
                            current.reversed = value.trim().eq_ignore_ascii_case("true");
                        }
                    }
                    b"Scale" => {
                        if part == Some(ProfilePart::Output)
                            && let Some(scale) = value.trim().parse::<f64>().ok().filter(|scale| scale.is_finite())
                            && let Some(current) = axis.as_mut()
                        {
                            current.scale = scale.clamp(0.0, 10.0);
                        }
                    }
                    b"OverallScale" => {
                        if let Some(scale) = value.trim().parse::<f64>().ok().filter(|scale| scale.is_finite()) {
                            mapping.overall_scale = scale.clamp(0.0, 10.0);
                        }
                    }
                    _ => {}
                }
            }
            Event::End(end) => match end.local_name().as_ref() {
                b"Axis" => {
                    if let Some(current) = axis.take()
                        && let (Some(input), Some(output)) = (current.input, current.output)
                    {
                        mapping.axes[input] = AxisRule { output, enabled: current.enabled, reversed: current.reversed, scale: current.scale };
                        found = true;
                    }
                }
                b"Input" | b"Output" => part = None,
                b"ActionID" | b"Enabled" | b"Reversed" | b"Scale" | b"OverallScale" => field.clear(),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    found.then_some(mapping)
}

fn load_driver_mapping() -> Option<DriverMapping> {
    let profile = std::path::PathBuf::from(std::env::var_os("APPDATA")?).join("3Dconnexion").join("3DxWare").join("Cfg").join("solvecraft-KMJ.xml");
    if std::fs::metadata(&profile).ok()?.len() > MAX_PROFILE_BYTES {
        log::warn!("3D mouse: ignoring oversized 3DxWare profile {}", profile.display());
        return None;
    }
    let xml = std::fs::read(&profile).ok()?;
    parse_driver_mapping(&xml)
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
    driver_mapping: DriverMapping,
    mapping_checked_at: Instant,
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
        Self { shared, last_frame: now, driver_mapping: load_driver_mapping().unwrap_or_default(), mapping_checked_at: now }
    }

    pub fn tick(&mut self, app: &mut SolveApp, ctx: &Context) {
        let now = Instant::now();
        let seconds = now.duration_since(self.last_frame).as_secs_f64().min(0.05);
        self.last_frame = now;
        if now.duration_since(self.mapping_checked_at) >= RETRY_AFTER {
            let updated = load_driver_mapping().unwrap_or_default();
            if updated != self.driver_mapping {
                log::info!("3D mouse: applying updated 3DxWare axis profile");
                self.driver_mapping = updated;
            }
            self.mapping_checked_at = now;
        }
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
        navigate(
            &mut app.cam,
            self.driver_mapping.apply(axes),
            seconds * speed.clamp(0.1, 5.0),
            height,
            app.session.active_sketch.is_some(),
            app.preferences.zoom_reverse,
        );
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}

/// A small dead zone avoids drift at rest. A full cap deflection takes roughly one viewport
/// per second to pan and about two radians per second to turn, independent of frame rate.
fn normalized(value: i16) -> f64 {
    let v = f64::from(value);
    if v.abs() <= 20.0 { 0.0 } else { v.signum() * ((v.abs() - 20.0) / 330.0).min(1.0) }
}

fn navigate(cam: &mut Camera, motion: Motion, seconds: f64, height: f64, sketch: bool, reverse_zoom: bool) {
    if !(seconds > 0.0) || !seconds.is_finite() {
        return;
    }
    let [x, y, z] = motion.translation;
    let [rx, ry, rz] = motion.rotation;
    // Logical 3DxWare coordinates are X = left/right, Y = zoom and Z = up/down. The user's
    // profile above can swap the physical forward/back and up/down axes before they reach here.
    cam.pan(x * height * 0.6 * seconds, z * height * 0.6 * seconds, height);
    let zoom_sign = if reverse_zoom { -1.0 } else { 1.0 };
    cam.zoom_at((-y * zoom_sign * 1.5 * seconds).exp(), None);
    if !sketch {
        // These are logical 3DxWare coordinates after the profile mapping above: Rx is Tilt,
        // Ry is Roll and Rz is Spin. The KMJ profile swaps physical Ry/Rz for the selected
        // navigation mode, so swapping them again here would make Roll behave like Spin.
        cam.orbit(ry * 250.0 * seconds, rx * 250.0 * seconds);
        cam.roll_by(rz * 2.0 * seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(axes: Axes) -> Motion {
        DriverMapping::default().apply(axes)
    }

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
        navigate(&mut cam, raw(Axes { translation: [350, 350, 350], rotation: [350, 350, 350] }), 0.02, 600.0, false, false);
        assert_ne!(cam.target, before.target);
        assert!(cam.distance < before.distance);
        assert_ne!(cam.yaw, before.yaw);
        assert_ne!(cam.pitch, before.pitch);
        assert_ne!(cam.roll, before.roll);
        let orientation = (cam.yaw, cam.pitch, cam.roll);
        navigate(&mut cam, raw(Axes { translation: [350, 0, 0], rotation: [350, 350, 350] }), 0.02, 600.0, true, false);
        assert_eq!((cam.yaw, cam.pitch, cam.roll), orientation);
        assert!(cam.is_valid());
    }

    #[test]
    fn individual_axes_have_physical_spacemouse_meaning() {
        let before = Camera::default();

        let mut slide = before;
        navigate(&mut slide, raw(Axes { translation: [350, 0, 0], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_ne!(slide.target, before.target);
        assert_eq!(slide.distance, before.distance);

        let mut push = before;
        navigate(&mut push, raw(Axes { translation: [0, 350, 0], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_eq!(push.target, before.target);
        assert!(push.distance < before.distance);

        let mut lift = before;
        navigate(&mut lift, raw(Axes { translation: [0, 0, 350], rotation: [0; 3] }), 0.02, 600.0, false, false);
        assert_ne!(lift.target, before.target);
        assert_eq!(lift.distance, before.distance);

        let mut tilt = before;
        navigate(&mut tilt, raw(Axes { translation: [0; 3], rotation: [350, 0, 0] }), 0.02, 600.0, false, false);
        assert_eq!((tilt.yaw, tilt.roll), (before.yaw, before.roll));
        assert_ne!(tilt.pitch, before.pitch);

        let mut roll = before;
        navigate(&mut roll, raw(Axes { translation: [0; 3], rotation: [0, 350, 0] }), 0.02, 600.0, false, false);
        assert_eq!((roll.pitch, roll.roll), (before.pitch, before.roll));
        assert_ne!(roll.yaw, before.yaw);

        let mut twist = before;
        navigate(&mut twist, raw(Axes { translation: [0; 3], rotation: [0, 0, 350] }), 0.02, 600.0, false, false);
        assert_eq!((twist.yaw, twist.pitch), (before.yaw, before.pitch));
        assert_ne!(twist.roll, before.roll);
    }

    #[test]
    fn reverse_zoom_setting_applies_to_spacemouse() {
        let before = Camera::default();
        let motion = raw(Axes { translation: [0, 350, 0], rotation: [0; 3] });
        let mut normal = before;
        navigate(&mut normal, motion, 0.02, 600.0, false, false);
        let mut reversed = before;
        navigate(&mut reversed, motion, 0.02, 600.0, false, true);
        assert!(normal.distance < before.distance);
        assert!(reversed.distance > before.distance);
    }

    #[test]
    fn applies_3dxware_axis_swaps_and_reversals() {
        let profile = br#"
            <AppCfg><Devices><Device><AxisBank>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_X</ActionID></Input><Output><ActionID>HIDMultiAxis_X</ActionID><Reversed>false</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Y</ActionID></Input><Output><ActionID>HIDMultiAxis_Z</ActionID><Reversed>false</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Z</ActionID></Input><Output><ActionID>HIDMultiAxis_Y</ActionID><Reversed>true</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Rx</ActionID></Input><Output><ActionID>HIDMultiAxis_Rx</ActionID><Reversed>false</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Ry</ActionID></Input><Output><ActionID>HIDMultiAxis_Rz</ActionID><Reversed>false</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Rz</ActionID></Input><Output><ActionID>HIDMultiAxis_Ry</ActionID><Reversed>true</Reversed></Output></Axis>
            </AxisBank></Device></Devices></AppCfg>
        "#;
        let mapping = parse_driver_mapping(profile).expect("valid 3DxWare profile");
        let motion = mapping.apply(Axes { translation: [350, 0, 350], rotation: [350, 0, 350] });
        assert_eq!(motion.translation, [1.0, -1.0, 0.0]);
        assert_eq!(motion.rotation, [1.0, -1.0, 0.0]);
    }

    #[test]
    fn mapped_roll_and_spin_reach_distinct_camera_rotations() {
        let profile = br#"
            <AppCfg><Devices><Device><AxisBank>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Ry</ActionID></Input><Output><ActionID>HIDMultiAxis_Rz</ActionID><Reversed>false</Reversed></Output></Axis>
              <Axis><Enabled>true</Enabled><Input><ActionID>HIDMultiAxis_Rz</ActionID></Input><Output><ActionID>HIDMultiAxis_Ry</ActionID><Reversed>true</Reversed></Output></Axis>
            </AxisBank></Device></Devices></AppCfg>
        "#;
        let mapping = parse_driver_mapping(profile).expect("valid 3DxWare profile");
        let before = Camera::default();

        let mut physical_roll = before;
        navigate(&mut physical_roll, mapping.apply(Axes { translation: [0; 3], rotation: [0, 350, 0] }), 0.02, 600.0, false, false);
        assert_eq!((physical_roll.yaw, physical_roll.pitch), (before.yaw, before.pitch));
        assert_ne!(physical_roll.roll, before.roll);

        let mut physical_spin = before;
        navigate(&mut physical_spin, mapping.apply(Axes { translation: [0; 3], rotation: [0, 0, 350] }), 0.02, 600.0, false, false);
        assert_ne!(physical_spin.yaw, before.yaw);
        assert_eq!((physical_spin.pitch, physical_spin.roll), (before.pitch, before.roll));
    }
}
