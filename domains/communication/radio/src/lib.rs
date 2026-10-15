//! The TX15's telemetry link, and the whole chain from another aircraft to a marker on the
//! station's map.
//!
//! The station has one source of telemetry: the radio. The radio passes on what the plane
//! sends down the link, and the link is narrow. The plane sends its own telemetry, and it
//! passes on what its receivers hear of other aircraft: MAVLink positions from the network,
//! and Remote ID broadcasts as OPEN_DRONE_ID messages. [`Downlink`] is that narrow link.
//! [`Rig`] is the whole chain in one place, so that a test can put another plane in the air
//! and look at the station's map.
//!
//! ```text
//! other planes ─air─▶ receivers on your plane ─▶ Downlink (TX15) ─▶ Decoder ─▶ Tracker ─▶ map
//!   your plane's own telemetry ───────────────────▶ Downlink (first in line)
//! ```

mod downlink;
pub mod frame;
mod relay;
pub mod rig;
pub mod serve;

pub use downlink::{Downlink, Priority};
pub use relay::Relay;
pub use rig::{Rig, Sending};
