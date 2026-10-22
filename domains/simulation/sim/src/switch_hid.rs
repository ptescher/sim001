//! Nintendo Switch Pro Controller over raw HID (USB or Bluetooth), like SDL/Steam do it.
//!
//! macOS's GameController framework does not deliver this controller's input to a bare
//! (non-.app) binary, and gilrs misparses its full-mode reports. This backend reads the
//! standard 0x30 input report directly, enabling full report mode if needed, and uses the
//! controller's factory stick calibration from SPI flash.

use bevy::input::gamepad::GamepadButton;
use bevy::math::Vec2;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VID: u16 = 0x057e;
const PID_PRO: u16 = 0x2009;

#[derive(Clone, Debug, Default)]
pub struct HidPadState {
    pub left: Vec2,
    pub right: Vec2,
    pub pressed: Vec<GamepadButton>,
    pub calibrated: bool,
    pub last_report: Option<Instant>,
}

pub type SharedHidPad = Arc<Mutex<Option<HidPadState>>>;

#[derive(Clone, Copy, Debug)]
struct AxisCal {
    center: f32,
    above: f32,
    below: f32,
}

impl AxisCal {
    fn nominal() -> Self {
        Self { center: 2048.0, above: 1500.0, below: 1500.0 }
    }
    fn apply(&self, raw: u16) -> f32 {
        let d = raw as f32 - self.center;
        let r = if d >= 0.0 { self.above } else { self.below };
        (d / r.max(100.0)).clamp(-1.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug)]
struct StickCal {
    x: AxisCal,
    y: AxisCal,
}

/// Spawns the reader thread. The returned state is `None` while no controller is connected.
pub fn start(debug: bool) -> SharedHidPad {
    let shared: SharedHidPad = Arc::new(Mutex::new(None));
    let out = shared.clone();
    std::thread::Builder::new()
        .name("switch-pro-hid".into())
        .spawn(move || run(out, debug))
        .expect("spawn HID thread");
    shared
}

fn run(shared: SharedHidPad, debug: bool) {
    let mut api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi unavailable: {e}");
            return;
        }
    };
    loop {
        let _ = api.refresh_devices();
        let found = api.device_list().any(|d| d.vendor_id() == VID && d.product_id() == PID_PRO);
        if found {
            if let Ok(dev) = api.open(VID, PID_PRO) {
                if debug {
                    println!("Switch Pro HID: opened");
                }
                read_loop(&dev, &shared, debug);
                *shared.lock().unwrap() = None;
                if debug {
                    println!("Switch Pro HID: disconnected");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn subcommand(dev: &hidapi::HidDevice, counter: &mut u8, id: u8, args: &[u8]) {
    let mut buf = vec![0x01, *counter & 0x0f, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40, id];
    buf.extend_from_slice(args);
    buf.resize(49, 0);
    *counter = counter.wrapping_add(1);
    let _ = dev.write(&buf);
}

fn read_loop(dev: &hidapi::HidDevice, shared: &SharedHidPad, debug: bool) {
    let mut buf = [0u8; 64];
    let mut counter = 0u8;
    let mut cal_left: Option<StickCal> = None;
    let mut cal_right: Option<StickCal> = None;
    let mut last_full = Instant::now();
    let mut last_cal_request = Instant::now() - Duration::from_secs(10);
    let mut cal_requests = 0;
    let mut kicked = false;

    loop {
        let n = match dev.read_timeout(&mut buf, 100) {
            Ok(n) => n,
            Err(_) => return,
        };

        // Ask for the factory stick calibration (left 0x603D, right 0x6046, 9 bytes each).
        if (cal_left.is_none() || cal_right.is_none()) && cal_requests < 6 && last_cal_request.elapsed() > Duration::from_millis(300) {
            last_cal_request = Instant::now();
            cal_requests += 1;
            let addr: u32 = if cal_left.is_none() { 0x603D } else { 0x6046 };
            let a = addr.to_le_bytes();
            subcommand(dev, &mut counter, 0x10, &[a[0], a[1], a[2], a[3], 9]);
        }

        if n == 0 {
            // No reports: switch the controller to full (0x30) report mode.
            if last_full.elapsed() > Duration::from_millis(800) && !kicked {
                kicked = true;
                let _ = dev.write(&[0x80, 0x02]); // USB handshake (ignored over Bluetooth)
                let _ = dev.write(&[0x80, 0x04]); // USB: disable timeout
                subcommand(dev, &mut counter, 0x03, &[0x30]);
                if debug {
                    println!("Switch Pro HID: requested full report mode");
                }
            }
            if last_full.elapsed() > Duration::from_secs(3) {
                return;
            }
            continue;
        }

        match buf[0] {
            0x21 if n >= 29 && buf[14] == 0x10 => {
                // SPI read reply: address at 15..19, length at 19, data from 20.
                let addr = u32::from_le_bytes([buf[15], buf[16], buf[17], buf[18]]);
                let d = &buf[20..29];
                let w = |lo: u8, hi: u8| ((hi as u16 & 0x0f) << 8 | lo as u16) as f32;
                let h = |lo: u8, hi: u8| ((hi as u16) << 4 | (lo as u16) >> 4) as f32;
                if d.iter().all(|&b| b == 0xff) {
                    continue;
                }
                match addr {
                    0x603D => {
                        // max above, center, min below
                        cal_left = Some(StickCal {
                            x: AxisCal { above: w(d[0], d[1]), center: w(d[3], d[4]), below: w(d[6], d[7]) },
                            y: AxisCal { above: h(d[1], d[2]), center: h(d[4], d[5]), below: h(d[7], d[8]) },
                        });
                        if debug {
                            println!("Switch Pro HID: left stick calibration {:?}", cal_left.unwrap());
                        }
                    }
                    0x6046 => {
                        // center, min below, max above
                        cal_right = Some(StickCal {
                            x: AxisCal { center: w(d[0], d[1]), below: w(d[3], d[4]), above: w(d[6], d[7]) },
                            y: AxisCal { center: h(d[1], d[2]), below: h(d[4], d[5]), above: h(d[7], d[8]) },
                        });
                        if debug {
                            println!("Switch Pro HID: right stick calibration {:?}", cal_right.unwrap());
                        }
                    }
                    _ => {}
                }
            }
            0x30 if n >= 12 => {
                last_full = Instant::now();
                let stick = |o: usize| -> (u16, u16) {
                    let x = buf[o] as u16 | ((buf[o + 1] as u16 & 0x0f) << 8);
                    let y = (buf[o + 1] as u16 >> 4) | ((buf[o + 2] as u16) << 4);
                    (x, y)
                };
                let (lx, ly) = stick(6);
                let (rx, ry) = stick(9);
                let nominal = StickCal { x: AxisCal::nominal(), y: AxisCal::nominal() };
                let cl = cal_left.unwrap_or(nominal);
                let cr = cal_right.unwrap_or(nominal);
                let (r, s, l) = (buf[3], buf[4], buf[5]);
                let mut pressed = Vec::new();
                for (byte, bit, b) in [
                    (r, 0x01, GamepadButton::West),  // Y
                    (r, 0x02, GamepadButton::North), // X
                    (r, 0x04, GamepadButton::South), // B
                    (r, 0x08, GamepadButton::East),  // A
                    (r, 0x40, GamepadButton::RightTrigger),
                    (r, 0x80, GamepadButton::RightTrigger2),
                    (s, 0x01, GamepadButton::Select), // -
                    (s, 0x02, GamepadButton::Start),  // +
                    (s, 0x04, GamepadButton::RightThumb),
                    (s, 0x08, GamepadButton::LeftThumb),
                    (s, 0x10, GamepadButton::Mode), // Home
                    (l, 0x01, GamepadButton::DPadDown),
                    (l, 0x02, GamepadButton::DPadUp),
                    (l, 0x04, GamepadButton::DPadRight),
                    (l, 0x08, GamepadButton::DPadLeft),
                    (l, 0x40, GamepadButton::LeftTrigger),
                    (l, 0x80, GamepadButton::LeftTrigger2),
                ] {
                    if byte & bit != 0 {
                        pressed.push(b);
                    }
                }
                *shared.lock().unwrap() = Some(HidPadState {
                    left: Vec2::new(cl.x.apply(lx), cl.y.apply(ly)),
                    right: Vec2::new(cr.x.apply(rx), cr.y.apply(ry)),
                    pressed,
                    calibrated: cal_left.is_some() && cal_right.is_some(),
                    last_report: Some(Instant::now()),
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sticks_from_real_report() {
        // Captured at rest from the user's controller.
        let b = [0x30u8, 0xd7, 0x81, 0x00, 0x80, 0x00, 0xce, 0xe8, 0x72, 0x74, 0xb8, 0x74];
        let x = b[6] as u16 | ((b[7] as u16 & 0x0f) << 8);
        let y = (b[7] as u16 >> 4) | ((b[8] as u16) << 4);
        assert_eq!((x, y), (2254, 1838));
        let cal = AxisCal { center: 2254.0, above: 1500.0, below: 1500.0 };
        assert!(cal.apply(2254).abs() < 1e-6);
        assert!((cal.apply(2254 + 1500) - 1.0).abs() < 1e-6);
    }
}
