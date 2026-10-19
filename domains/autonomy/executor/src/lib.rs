//! The plan executor, on board. `no_std`, no allocation, a C ABI: ArduPlane's AGENT mode calls
//! it every control step, and the training environment calls the same code, so what is trained
//! is what flies.
//!
//! Phase 0: the ABI and the build only. The executor asks for wings level. Angles are in
//! centidegrees, as ArduPilot's `nav_roll_cd` and `nav_pitch_cd`.

#![cfg_attr(feature = "ffi-static", no_std)]

/// Bumped when the C ABI changes, so the C++ side can refuse a library that does not match.
pub const ABI_VERSION: u32 = 1;

/// What the executor asks the stock controllers for.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Demand {
    /// Roll demand (cd, positive right wing down).
    pub roll_cd: i32,
    /// Pitch demand (cd, positive nose up).
    pub pitch_cd: i32,
    /// Throttle 0 to 1, or a negative number to leave it to the pilot.
    pub throttle: f32,
}

/// Wings level, nose level, throttle left to the pilot.
pub fn level() -> Demand {
    Demand { roll_cd: 0, pitch_cd: 0, throttle: -1.0 }
}

#[unsafe(no_mangle)]
pub extern "C" fn agent_exec_abi_version() -> u32 {
    ABI_VERSION
}

/// Fills `out` with this step's demand. Returns 0, or -1 if `out` is null.
///
/// # Safety
/// `out` must be null or point to a writable `Demand`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn agent_exec_step(out: *mut Demand) -> i32 {
    match unsafe { out.as_mut() } {
        Some(d) => {
            *d = level();
            0
        }
        None => -1,
    }
}

#[cfg(feature = "ffi-static")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_asks_for_wings_level() {
        let mut d = Demand { roll_cd: 9, pitch_cd: 9, throttle: 1.0 };
        assert_eq!(unsafe { agent_exec_step(&mut d) }, 0);
        assert_eq!(d, level());
    }

    #[test]
    fn a_null_output_is_refused() {
        assert_eq!(unsafe { agent_exec_step(core::ptr::null_mut()) }, -1);
    }

    #[test]
    fn the_abi_version_is_exported() {
        assert_eq!(agent_exec_abi_version(), ABI_VERSION);
    }
}
