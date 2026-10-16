//! Pieces shared by the sim and the ground station. No dependencies, so both sides (Bevy and
//! GPUI) can use it without their dependency trees meeting.

pub mod feed;
pub mod layout;
pub mod mappack;
