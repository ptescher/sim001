//! The iOS ground station as the expectation framework drives it. State is read from the app's
//! control server (what the Rust station says); events are touches the XCUITest bridge makes on
//! the controls by their accessibility ids, the way a person's finger would.

use crate::bridge::{Bridge, base64_decode};
use expectation_harness::{Driver, Need, Progress, Screenshot};
use expectation_model::{Event, Value, Variable};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// The states the app reports (the control server's keys are the vocabulary's).
const STATES: &[&str] = &["link", "vehicle", "autopilot mode", "home position"];
const SETTABLE: &[&str] = &["autopilot mode"];

/// What each event of the vocabulary does on the app.
enum Act {
    /// Touch the control with this id.
    Button(&'static str),
    /// ARM, then CONFIRM ARM, until the vehicle arms.
    Arm,
}

const EVENTS: &[(&str, Act)] = &[
    ("operator arms the vehicle", Act::Arm),
    ("operator disarms the vehicle", Act::Button("disarm")),
    ("operator switches to forward flight", Act::Button("mode-CRUISE")),
    ("operator switches to hover", Act::Button("mode-QLOITER")),
    ("operator lands vertically", Act::Button("mode-QLAND")),
];

const MODES: &[&str] =
    &["MANUAL", "FBWA", "CRUISE", "AUTO", "RTL", "LOITER", "QSTABILIZE", "QHOVER", "QLOITER", "QLAND", "QRTL"];

const LOOK: Duration = Duration::from_millis(100);
const SETTLE: Duration = Duration::from_millis(600);

pub struct IosDriver {
    bridge: Bridge,
    control_port: u16,
    control: Option<BufReader<TcpStream>>,
}

impl IosDriver {
    pub fn new(bridge: Bridge, control_port: u16) -> Self {
        IosDriver { bridge, control_port, control: None }
    }

    /// Ask the app's control server, connecting (or reconnecting after a relaunch) as needed.
    fn read(&mut self, key: &str) -> Result<Value, String> {
        let mut last = String::from("not started");
        for attempt in 0..2 {
            if self.control.is_none() {
                self.control = Some(self.connect()?);
            }
            let conn = self.control.as_mut().unwrap();
            let answer = writeln!(conn.get_mut(), "observe {key}").map_err(|e| e.to_string()).and_then(|_| {
                let mut line = String::new();
                match conn.read_line(&mut line) {
                    Ok(0) => Err("the app closed the connection".to_string()),
                    Ok(_) => Ok(line.trim_end().to_string()),
                    Err(e) => Err(e.to_string()),
                }
            });
            match answer {
                Ok(line) => {
                    return match line.split_once(' ') {
                        Some(("ok", v)) => Ok(Value::from(v)),
                        Some(("err", why)) => Err(why.to_string()),
                        _ => Err(format!("unexpected answer {line}")),
                    };
                }
                Err(e) => {
                    last = e;
                    self.control = None;
                    if attempt == 0 {
                        std::thread::sleep(LOOK);
                    }
                }
            }
        }
        Err(last)
    }

    fn connect(&self) -> Result<BufReader<TcpStream>, String> {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            match TcpStream::connect(("127.0.0.1", self.control_port)) {
                Ok(c) => {
                    c.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
                    return Ok(BufReader::new(c));
                }
                Err(e) if Instant::now() > until => return Err(format!("the app's control server isn't up: {e}")),
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    fn tap(&mut self, id: &str) -> Result<(), String> {
        self.bridge.ask(&format!("tap {id}")).map(|_| ())
    }

    fn is(&mut self, key: &str, want: &str) -> bool {
        self.read(key).is_ok_and(|v| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case(want)))
    }

    fn wait_until(&mut self, seconds: u64, key: &str, want: &str) -> bool {
        let until = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < until {
            if self.is(key, want) {
                return true;
            }
            std::thread::sleep(LOOK);
        }
        false
    }

    /// ARM, CONFIRM ARM, again and again until the vehicle arms: one that has only just come up
    /// refuses until its pre-arm checks pass.
    fn arm(&mut self) -> Result<(), String> {
        for _ in 0..12 {
            self.tap("arm")?;
            self.tap("arm")?;
            if self.wait_until(5, "vehicle", "armed") {
                return Ok(());
            }
        }
        Err("the vehicle wouldn't arm".into())
    }
}

fn enum_value(value: &Value) -> Result<String, String> {
    value.as_str().map(str::to_string).ok_or_else(|| "expected a name".to_string())
}

impl Driver for IosDriver {
    /// A fresh app. The vehicle is the SITL that is already running: a test that leaves it armed
    /// starts the next from a disarmed one.
    fn reset(&mut self) -> Result<(), String> {
        self.control = None;
        self.bridge.ask("launch")?;
        if !self.wait_until(30, "link", "connected") {
            return Err("the app has no link to the vehicle: is ArduPilot SITL running on tcp:5760?".into());
        }
        if self.is("vehicle", "armed") {
            self.tap("disarm")?;
            if !self.wait_until(10, "vehicle", "disarmed") {
                self.tap("disarm")?;
                self.wait_until(10, "vehicle", "disarmed");
            }
        }
        Ok(())
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        match var.key.as_str() {
            "vehicle" => match enum_value(value)?.as_str() {
                "armed" if !self.is("vehicle", "armed") => self.arm(),
                "disarmed" if self.is("vehicle", "armed") => self.tap("disarm"),
                _ => Ok(()),
            },
            "autopilot mode" => self.set(var, value),
            // What the app finds out, not what it is told: it is already in the state or it isn't.
            key => {
                let now = self.read(key)?;
                if now.as_str().is_some_and(|s| value.as_str().is_some_and(|w| s.eq_ignore_ascii_case(w))) {
                    Ok(())
                } else {
                    Err(format!("{key} is {now}, and can't be put to {value} from the app"))
                }
            }
        }
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        match var.key.as_str() {
            "autopilot mode" => {
                let mode = enum_value(value)?;
                let id = MODES.iter().find(|m| **m == mode).ok_or_else(|| format!("no mode called {mode}"))?;
                self.tap(&format!("mode-{id}"))
            }
            other => Err(format!("{other} isn't something to set")),
        }
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        match EVENTS.iter().find(|(k, _)| *k == event.key).map(|(_, a)| a).ok_or("no way to do it")? {
            Act::Button(id) => self.tap(id),
            Act::Arm => self.arm(),
        }
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        self.read(&var.key)
    }

    fn can_arrange(&self, var: &Variable, _value: &Value) -> bool {
        STATES.contains(&var.key.as_str())
    }

    fn settle(&mut self) {
        std::thread::sleep(SETTLE);
    }

    fn advance(&mut self) -> Progress {
        std::thread::sleep(LOOK);
        Progress::Advanced
    }

    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(Some(Screenshot::png(base64_decode(&self.bridge.ask("screenshot")?))))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Observe(v) | Need::Arrange(v) => STATES.contains(&v.key.as_str()),
            Need::Set(v) => SETTABLE.contains(&v.key.as_str()),
            Need::Perform(e) => EVENTS.iter().any(|(k, _)| *k == e.key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_harness::Acceptance;
    use std::path::PathBuf;

    /// Which plans the app can run today: the number moves up as the app grows, never down.
    #[test]
    fn the_driver_covers_arming_and_the_mode_buttons() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../expectations");
        let a = Acceptance::load_for(&dir, "iOS").unwrap_or_else(|e| panic!("{e}")).with_coverage();
        let v = &a.suite.vocabulary;
        let ok = |p: &expectation_story::Plan| {
            p.start.iter().all(|(x, _)| STATES.contains(&v.var(x).key.as_str()))
                && p.steps
                    .iter()
                    .flat_map(|s| &s.checks)
                    .chain(&p.start_checks)
                    .all(|c| STATES.contains(&v.var(c.var).key.as_str()))
                && p.steps
                    .iter()
                    .filter_map(|s| s.action.event)
                    .all(|e| EVENTS.iter().any(|(k, _)| *k == v.event(e).key))
                && p.steps
                    .iter()
                    .flat_map(|s| s.action.changes.vars())
                    .all(|x| SETTABLE.contains(&v.var(x).key.as_str()))
        };
        let runnable: Vec<&str> = a.plans.iter().filter(|p| ok(p)).map(|p| p.title.as_str()).collect();
        for want in ["Arming sets home", "DISARM disarms"] {
            assert!(runnable.iter().any(|t| t.contains(want)), "no test for `{want}` can run; runnable: {runnable:?}");
        }
    }
}
