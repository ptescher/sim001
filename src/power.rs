//! Keeps the computer awake while the sim runs, and asks the OS for its gaming mode where one
//! can be asked for. Hold the returned guard for as long as the sim runs; dropping it (or the
//! process ending, however it ends) gives the machine back its normal sleep behaviour.
//!
//! - macOS: an `NSProcessInfo` activity that stops idle display and system sleep (and App Nap),
//!   marked latency-critical. Game Mode can't be asked for from code: macOS turns it on for a
//!   full-screen app whose bundle says it is a game (`LSApplicationCategoryType`,
//!   `LSSupportsGameMode`). See `tools/macos-app.sh` and `--fullscreen`.
//! - Windows: `SetThreadExecutionState`. Game Mode comes by itself for a full-screen game.
//! - Linux: `systemd-inhibit` (idle and sleep), and Feral's GameMode (`gamemoded -r`) when it is
//!   installed. Neither is tested here: both are best effort, and a missing tool is not an error.

/// Held while the machine should stay awake.
pub struct KeepAwake {
    _guard: imp::Guard,
}

/// Start keeping the machine awake. `None` if the OS wouldn't (the sim still runs).
pub fn keep_awake(reason: &str) -> Option<KeepAwake> {
    imp::start(reason).map(|_guard| KeepAwake { _guard })
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

    pub struct Guard {
        activity: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    pub fn start(reason: &str) -> Option<Guard> {
        // UserInitiated includes "no idle system sleep" and opts out of App Nap.
        let options = NSActivityOptions::UserInitiated
            | NSActivityOptions::IdleDisplaySleepDisabled
            | NSActivityOptions::LatencyCritical;
        let activity =
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &NSString::from_str(reason));
        Some(Guard { activity })
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: the activity came from `beginActivityWithOptions:reason:`.
            unsafe { NSProcessInfo::processInfo().endActivity(&self.activity) };
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    pub struct Guard;

    /// The state belongs to the calling thread: call this from the one that lives as long as the sim.
    pub fn start(_reason: &str) -> Option<Guard> {
        // SAFETY: plain Win32 call with documented flags.
        let before = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED) };
        (before != 0).then_some(Guard)
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: as above; ES_CONTINUOUS alone clears the request.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::process::{Child, Command, Stdio};

    pub struct Guard {
        inhibitor: Option<Child>,
    }

    pub fn start(reason: &str) -> Option<Guard> {
        // Feral GameMode: ask its daemon to apply its tuning to this process.
        let _ = Command::new("gamemoded")
            .args(["-r", &std::process::id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // Holds the inhibitor for as long as this process lives, even if it is killed: the
        // shell polls for its parent (this process, which systemd-inhibit execs the command under).
        let inhibitor = Command::new("systemd-inhibit")
            .args(["--what=idle:sleep", "--who=sim001", &format!("--why={reason}"), "--mode=block"])
            .args(["sh", "-c", "while kill -0 $PPID 2>/dev/null; do sleep 2; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
        inhibitor.as_ref()?;
        Some(Guard { inhibitor })
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(mut child) = self.inhibitor.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
mod imp {
    pub struct Guard;
    pub fn start(_reason: &str) -> Option<Guard> {
        None
    }
}
