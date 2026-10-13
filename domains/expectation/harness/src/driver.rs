//! The system under test, as the runner sees it.

use expectation_model::{Event, Value, Variable};

/// Whether waiting could change anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Time passed (a frame, a tick, a sleep); observe again.
    Advanced,
    /// Nothing will change by waiting; the observations so far are final.
    Static,
}

pub struct Screenshot {
    pub bytes: Vec<u8>,
    /// File extension, e.g. "png" or "svg".
    pub extension: &'static str,
}

impl Screenshot {
    pub fn png(bytes: Vec<u8>) -> Self {
        Screenshot { bytes, extension: "png" }
    }

    pub fn jpeg(bytes: Vec<u8>) -> Self {
        Screenshot { bytes, extension: "jpg" }
    }

    pub fn svg(svg: String) -> Self {
        Screenshot { bytes: svg.into_bytes(), extension: "svg" }
    }
}

/// Something a plan needs the driver to do.
#[derive(Clone, Copy, Debug)]
pub enum Need<'a> {
    Arrange(&'a Variable),
    Set(&'a Variable),
    Perform(&'a Event),
    Observe(&'a Variable),
}

impl std::fmt::Display for Need<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Need::Arrange(v) => write!(f, "arrange {}", v.subject),
            Need::Set(v) => write!(f, "set {}", v.subject),
            Need::Perform(e) => write!(f, "make it happen that {}", e.text),
            Need::Observe(v) => write!(f, "observe {}", v.subject),
        }
    }
}

/// Drives the system under test in the vocabulary's terms.
///
/// Calls may block. A real-time system (like the sim) can let time pass in [`Driver::advance`]
/// and the runner keeps observing until a step's checks hold or its patience runs out, so
/// "Then the aircraft is flying" means *eventually*, as it would for a person watching.
pub trait Driver {
    /// Put the system in its starting state (what the Background describes), before each test.
    fn reset(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Make `var` hold `value` before the first step ("Given …").
    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        self.set(var, value)
    }

    /// Make `var` hold `value` as an action ("When the throttle is 80 %").
    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String>;

    /// Make `event` happen ("When the pilot presses reset").
    fn perform(&mut self, event: &Event) -> Result<(), String>;

    /// What `var` is now. Enumerated values are matched to the vocabulary ignoring case.
    fn observe(&mut self, var: &Variable) -> Result<Value, String>;

    /// Can it put `var` in `value` to start a test? [`Driver::supports`] only knows the state; a
    /// driver that can arrange some of its values and not others (a plane with no flight
    /// controller can't start with one fitted) says so here, and the tests that need the rest
    /// are "not supported yet" instead of failing.
    fn can_arrange(&self, var: &Variable, value: &Value) -> bool {
        let _ = (var, value);
        true
    }

    /// Let the system settle after an action, before anything is observed: long enough for
    /// what should happen to happen, and for what shouldn't to show. Without it, "Then the
    /// aircraft is in hand" could pass in the instant before a launch.
    fn settle(&mut self) {}

    /// Let a little time pass before the runner observes again.
    fn advance(&mut self) -> Progress {
        Progress::Static
    }

    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(None)
    }

    /// Can it do `need`? The runner checks every plan up front, so a missing binding fails
    /// the test before it starts instead of halfway through.
    fn supports(&self, need: Need<'_>) -> bool {
        let _ = need;
        true
    }
}
