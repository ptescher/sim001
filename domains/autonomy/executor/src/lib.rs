//! The plan executor, on board. `no_std`, no allocation, a C ABI: ArduPlane's AGENT mode calls
//! it every control step, and the training environment calls the same code, so what is trained
//! is what flies. The types are `autonomy_plan`'s; the header is `include/agent_executor.h`.
//!
//! Stub: wings level, whatever the plan. (Phase 1 replaces the body of `step`.)

#![cfg_attr(feature = "ffi-static", no_std)]

pub use autonomy_plan::{Demand, Limits, Plan, State, Track};

/// Bumped when the C ABI changes, so the C++ side can refuse a library that does not match.
pub const ABI_VERSION: u32 = 2;

/// Wings level, nose level, throttle left to the pilot.
pub fn level() -> Demand {
    Demand { roll_cd: 0, pitch_cd: 0, throttle: -1.0, ..Demand::default() }
}

#[unsafe(no_mangle)]
pub extern "C" fn agent_exec_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn agent_exec_reset() {}

/// # Safety
/// Each pointer must be null or valid: `tracks` for `MAX_TRACKS` entries, `plan` may be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn agent_exec_step(state: *const State, _plan: *const Plan, _tracks: *const Track, limits: *const Limits, out: *mut Demand) -> i32 {
    if state.is_null() || limits.is_null() || out.is_null() {
        return -1;
    }
    unsafe { *out = level() };
    0
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
        let (s, l) = (State::default(), Limits::default());
        let mut d = Demand { roll_cd: 9, pitch_cd: 9, throttle: 1.0, ..Demand::default() };
        assert_eq!(unsafe { agent_exec_step(&s, core::ptr::null(), core::ptr::null(), &l, &mut d) }, 0);
        assert_eq!(d, level());
    }

    #[test]
    fn a_null_pointer_is_refused() {
        assert_eq!(unsafe { agent_exec_step(core::ptr::null(), core::ptr::null(), core::ptr::null(), core::ptr::null(), core::ptr::null_mut()) }, -1);
    }
}
