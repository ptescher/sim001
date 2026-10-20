//! What a ground station is before it has a screen: the MAVLink link, the vehicle's state and
//! history, and the geometry of the map. `teleoperation_console` (GPUI) and `teleoperation_ios`
//! (SwiftUI) both draw this.

pub mod debug;
pub mod geo;
pub mod link;
pub mod vehicle;
