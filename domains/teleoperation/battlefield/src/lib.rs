//! The battlefield view of a real-time strategy screen, shared by the ground station (the console)
//! and the commander app of Towers.
//!
//! - [`link`]: the 3D picture of the field from the operator's camera, from a 3D view;
//! - [`lag`]: how far that picture trails the camera;
//! - [`camera`]: the camera and its controls (keys, the mouse at an edge, the wheel, turns);
//! - [`lens`]: where the field is on screen, through the camera and on the minimap;
//! - [`paint`]: what both apps draw on the battlefield and the minimap;
//! - [`widgets`]: the panels, cards and command card of the bars;
//! - [`draw`] and [`theme`]: the drawing primitives and the look.

pub mod camera;
pub mod draw;
pub mod lag;
pub mod lens;
pub mod link;
pub mod orders;
pub mod paint;
pub mod theme;
pub mod widgets;

pub use camera::Rig;
pub use lens::{Flat, Lens};
pub use link::Battlefield;
pub use teleoperation_shared::battlefield::Eye;
