//! The ground station on iOS. The screens are SwiftUI (`App/`); everything they show and every
//! button they press goes through [`Station`], so the behaviour is Rust and is tested here.
//!
//! - [`station`]: the link to the vehicle and the buttons (ARM asks twice, and so on).
//! - [`control`]: a TCP server the acceptance driver reads the station's state from.
//! - [`ffi`]: the C interface SwiftUI calls.

pub mod control;
pub mod ffi;
pub mod station;

pub use station::Station;
