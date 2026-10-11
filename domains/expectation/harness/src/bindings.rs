//! A [`Driver`] made of closures over the system under test, one per vocabulary term:
//!
//! ```ignore
//! let mut driver = Bindings::new(World::default())
//!     .observe("the aircraft", |w| w.phase.label())
//!     .set("the throttle", |w, v| w.throttle = v.as_f64().unwrap())
//!     .on("the pilot presses reset", |w| w.reset())
//!     .on_settle(|w| w.tick(1.0))
//!     .on_advance(|w| { w.tick(0.1); Progress::Advanced });
//! ```

use crate::driver::{Driver, Need, Progress, Screenshot};
use expectation_model::text;
use expectation_model::{Event, Value, Variable};
use std::collections::HashMap;

type Observer<W> = Box<dyn FnMut(&W) -> Value>;
type Setter<W> = Box<dyn FnMut(&mut W, &Value)>;
type Action<W> = Box<dyn FnMut(&mut W)>;
type Advancer<W> = Box<dyn FnMut(&mut W) -> Progress>;
type Shooter<W> = Box<dyn FnMut(&W) -> Screenshot>;

pub struct Bindings<W> {
    pub world: W,
    observers: HashMap<String, Observer<W>>,
    setters: HashMap<String, Setter<W>>,
    arrangers: HashMap<String, Setter<W>>,
    events: HashMap<String, Action<W>>,
    reset: Option<Action<W>>,
    settle: Option<Action<W>>,
    advance: Option<Advancer<W>>,
    screenshot: Option<Shooter<W>>,
}

impl<W> Bindings<W> {
    pub fn new(world: W) -> Self {
        Bindings {
            world,
            observers: HashMap::new(),
            setters: HashMap::new(),
            arrangers: HashMap::new(),
            events: HashMap::new(),
            reset: None,
            settle: None,
            advance: None,
            screenshot: None,
        }
    }

    /// How to read a state. Subjects match the vocabulary ignoring case and a leading "the".
    pub fn observe<V: Into<Value>>(mut self, subject: &str, mut f: impl FnMut(&W) -> V + 'static) -> Self {
        self.observers.insert(text::key(subject), Box::new(move |w| f(w).into()));
        self
    }

    /// How to change a state, as an action and (unless [`Bindings::arrange`] says otherwise)
    /// when arranging the start.
    pub fn set(mut self, subject: &str, f: impl FnMut(&mut W, &Value) + 'static) -> Self {
        self.setters.insert(text::key(subject), Box::new(f));
        self
    }

    /// How to put a state in place before the first step, when that differs from [`Bindings::set`].
    pub fn arrange(mut self, subject: &str, f: impl FnMut(&mut W, &Value) + 'static) -> Self {
        self.arrangers.insert(text::key(subject), Box::new(f));
        self
    }

    pub fn on(mut self, event: &str, f: impl FnMut(&mut W) + 'static) -> Self {
        self.events.insert(text::key(event), Box::new(f));
        self
    }

    pub fn on_reset(mut self, f: impl FnMut(&mut W) + 'static) -> Self {
        self.reset = Some(Box::new(f));
        self
    }

    pub fn on_settle(mut self, f: impl FnMut(&mut W) + 'static) -> Self {
        self.settle = Some(Box::new(f));
        self
    }

    pub fn on_advance(mut self, f: impl FnMut(&mut W) -> Progress + 'static) -> Self {
        self.advance = Some(Box::new(f));
        self
    }

    pub fn on_screenshot(mut self, f: impl FnMut(&W) -> Screenshot + 'static) -> Self {
        self.screenshot = Some(Box::new(f));
        self
    }
}

fn unbound(what: &str, name: &str) -> String {
    format!("nothing is bound to {what} `{name}`")
}

impl<W> Driver for Bindings<W> {
    fn reset(&mut self) -> Result<(), String> {
        if let Some(f) = &mut self.reset {
            f(&mut self.world);
        }
        Ok(())
    }

    fn arrange(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let f = self.arrangers.get_mut(&var.key).or_else(|| self.setters.get_mut(&var.key));
        f.map(|f| f(&mut self.world, value)).ok_or_else(|| unbound("arrange", &var.subject))
    }

    fn set(&mut self, var: &Variable, value: &Value) -> Result<(), String> {
        let f = self.setters.get_mut(&var.key);
        f.map(|f| f(&mut self.world, value)).ok_or_else(|| unbound("set", &var.subject))
    }

    fn perform(&mut self, event: &Event) -> Result<(), String> {
        let f = self.events.get_mut(&event.key);
        f.map(|f| f(&mut self.world)).ok_or_else(|| unbound("the event", &event.text))
    }

    fn observe(&mut self, var: &Variable) -> Result<Value, String> {
        let f = self.observers.get_mut(&var.key);
        f.map(|f| f(&self.world)).ok_or_else(|| unbound("observe", &var.subject))
    }

    fn settle(&mut self) {
        if let Some(f) = &mut self.settle {
            f(&mut self.world);
        }
    }

    fn advance(&mut self) -> Progress {
        self.advance.as_mut().map_or(Progress::Static, |f| f(&mut self.world))
    }

    fn screenshot(&mut self) -> Result<Option<Screenshot>, String> {
        Ok(self.screenshot.as_mut().map(|f| f(&self.world)))
    }

    fn supports(&self, need: Need<'_>) -> bool {
        match need {
            Need::Arrange(v) => self.arrangers.contains_key(&v.key) || self.setters.contains_key(&v.key),
            Need::Set(v) => self.setters.contains_key(&v.key),
            Need::Perform(e) => self.events.contains_key(&e.key),
            Need::Observe(v) => self.observers.contains_key(&v.key),
        }
    }
}
