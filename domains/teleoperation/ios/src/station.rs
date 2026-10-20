//! The station's state and buttons, as the SwiftUI screens and the acceptance driver see them.
//! The keys of [`Station::observe`] are the vocabulary's state keys (`expectations/`).

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use teleoperation_core::link::{self, Cmd, Shared};
use teleoperation_core::vehicle::{Vehicle, mode_number};

/// How long ARM waits for its confirming press, as the console does.
const ARM_CONFIRM: Duration = Duration::from_secs(3);
/// How long after DISARM a second press forces it.
const FORCE_WINDOW: Duration = Duration::from_secs(8);

#[derive(Default)]
struct Buttons {
    arm_pressed: Option<Instant>,
    disarm_pressed: Option<Instant>,
}

pub struct Station {
    pub vehicle: Shared,
    tx: Sender<Cmd>,
    buttons: Mutex<Buttons>,
}

impl Station {
    /// Connect to `address` (any rust-mavlink address, e.g. `tcpout:127.0.0.1:5760`). It keeps
    /// trying until the vehicle answers.
    pub fn start(address: &str) -> Station {
        let vehicle: Shared = Arc::new(Mutex::new(Vehicle::new(address)));
        let tx = link::spawn(address.to_string(), vehicle.clone());
        Station { vehicle, tx, buttons: Mutex::default() }
    }

    /// A button was pressed: `arm`, `disarm` or `mode-<NAME>`. Unknown ids do nothing.
    pub fn press(&self, id: &str) {
        let armed = self.vehicle.lock().unwrap().armed;
        let mut b = self.buttons.lock().unwrap();
        let _ = match id {
            "arm" if !armed && b.arm_pressed.is_some_and(|t| t.elapsed() < ARM_CONFIRM) => {
                b.arm_pressed = None;
                self.tx.send(Cmd::Arm(true))
            }
            "arm" => {
                b.arm_pressed = Some(Instant::now());
                Ok(())
            }
            "disarm" if b.disarm_pressed.is_some_and(|t| t.elapsed() < FORCE_WINDOW) => {
                b.disarm_pressed = None;
                self.tx.send(Cmd::ForceDisarm)
            }
            "disarm" => {
                b.disarm_pressed = Some(Instant::now());
                self.tx.send(Cmd::Arm(false))
            }
            _ => match id.strip_prefix("mode-").and_then(mode_number) {
                Some(m) => self.tx.send(Cmd::SetMode(m)),
                None => Ok(()),
            },
        };
    }

    /// What the ARM/DISARM button says now.
    pub fn arm_button(&self) -> &'static str {
        let armed = self.vehicle.lock().unwrap().armed;
        let b = self.buttons.lock().unwrap();
        match armed {
            true if b.disarm_pressed.is_some_and(|t| t.elapsed() < FORCE_WINDOW) => "FORCE DISARM",
            true => "DISARM",
            false if b.arm_pressed.is_some_and(|t| t.elapsed() < ARM_CONFIRM) => "CONFIRM ARM",
            false => "ARM",
        }
    }

    /// The state called `key` in the vocabulary, as the vocabulary writes its values.
    pub fn observe(&self, key: &str) -> Option<String> {
        if key == "arm button" {
            return Some(self.arm_button().into());
        }
        let v = self.vehicle.lock().unwrap();
        Some(match key {
            "link" => if v.link.alive() { "connected" } else { "lost" }.into(),
            "vehicle" => if v.armed { "armed" } else { "disarmed" }.into(),
            "autopilot mode" => v.mode().into(),
            "home position" => if v.home.is_some() { "set" } else { "not set" }.into(),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A station nothing answers: nothing on this port.
    fn alone() -> Station {
        Station::start("tcpout:127.0.0.1:1")
    }

    #[test]
    fn with_no_vehicle_the_link_is_lost_and_nothing_is_known() {
        let s = alone();
        assert_eq!(s.observe("link").as_deref(), Some("lost"));
        assert_eq!(s.observe("vehicle").as_deref(), Some("disarmed"));
        assert_eq!(s.observe("home position").as_deref(), Some("not set"));
        assert_eq!(s.observe("no such state"), None);
    }

    #[test]
    fn arm_asks_twice() {
        let s = alone();
        assert_eq!(s.arm_button(), "ARM");
        s.press("arm");
        assert_eq!(s.arm_button(), "CONFIRM ARM");
        s.press("arm");
        assert_eq!(s.arm_button(), "ARM");
    }

    #[test]
    fn disarm_forces_on_the_second_press() {
        let s = alone();
        s.vehicle.lock().unwrap().armed = true;
        assert_eq!(s.arm_button(), "DISARM");
        s.press("disarm");
        assert_eq!(s.arm_button(), "FORCE DISARM");
        s.press("disarm");
        assert_eq!(s.arm_button(), "DISARM");
    }
}
