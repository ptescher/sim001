//! The ArduPilot the sim starts for itself, so that a plane with a flight controller flies with
//! no ground station: `sim001` alone, with the T1-S VTOL. A native ArduPlane SITL build (see
//! ardupilot/build_native.sh) is the light way; the Docker image the station uses is the fallback.

use crate::sim::SimCore;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
pub use simulation_sitl::launch::{DOCKER_NAME, MAVLINK_PORT, Plan, docker_image_exists, native_binary, plan, plane_mode_name};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// MAV_AUTOPILOT_ARDUPILOTMEGA.
const ARDUPILOTMEGA: u32 = 3;

/// Where the sim's files are: the checkout, or the app bundle's resources (`SIM001_HOME`, set by the launcher).
pub fn resource_root() -> PathBuf {
    std::env::var_os("SIM001_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// Where what the sim writes goes (flight logs): the checkout, or the user's data folder (`SIM001_DATA`).
pub fn data_dir(root: &Path) -> PathBuf {
    std::env::var_os("SIM001_DATA").map(PathBuf::from).unwrap_or_else(|| root.to_path_buf())
}

/// A started SITL, stopped when this is dropped.
pub struct Running {
    child: Child,
    docker: bool,
    root: PathBuf,
    plan: Plan,
}

impl Running {
    fn halt(&mut self) {
        if self.docker {
            let _ = Command::new("docker").args(["rm", "-f", DOCKER_NAME]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// A fresh ArduPilot: it is the only way back to a plane on the ground, disarmed, with its
    /// estimators at rest (the tests do this between one test and the next).
    pub fn restart(&mut self) -> Result<(), String> {
        self.halt();
        let (child, docker) = spawn(&self.root, self.plan.clone())?;
        self.child = child;
        self.docker = docker;
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The SITL the sim started for itself, kept here so that the tests can restart it.
static SITL: std::sync::Mutex<Option<Running>> = std::sync::Mutex::new(None);

pub fn keep(running: Running) {
    *SITL.lock().unwrap() = Some(running);
}

pub fn restart() -> Result<(), String> {
    SITL.lock().unwrap().as_mut().map_or(Err("the sim did not start this ArduPilot".into()), Running::restart)
}

/// Stops it (at exit).
pub fn stop() {
    SITL.lock().unwrap().take();
}

/// Start what the plan says. SITL runs in `logs/sitl` under `root`, where it keeps its files.
pub fn start(root: &Path, plan: Plan) -> Result<Running, String> {
    let (child, docker) = spawn(root, plan.clone())?;
    Ok(Running { child, docker, root: root.to_path_buf(), plan })
}

fn spawn(root: &Path, plan: Plan) -> Result<(Child, bool), String> {
    simulation_sitl::launch::spawn(&data_dir(root).join("logs/sitl"), plan)
}

/// Reads SITL's heartbeat over MAVLink and tells the core whether the plane is armed and in which
/// mode, as the ground station does when there is one (the throttle stick and the HUD need it).
pub fn watch_heartbeat(core: Arc<Mutex<SimCore>>) {
    std::thread::Builder::new()
        .name("sitl-heartbeat".into())
        .spawn(move || loop {
            let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", MAVLINK_PORT)) else {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            };
            let mut parser = communication_telemetry::Parser::default();
            let mut buf = [0u8; 512];
            while let Ok(n) = std::io::Read::read(&mut stream, &mut buf) {
                if n == 0 {
                    break;
                }
                parser.push(&buf[..n]);
                while let Some((_, msg)) = parser.next_message() {
                    // What ArduPilot says (why it won't arm, for one) goes to the terminal.
                    if let communication_telemetry::MavMessage::STATUSTEXT(t) = &msg {
                        let text: Vec<u8> = t.text.iter().copied().take_while(|b| *b != 0).collect();
                        println!("ArduPilot: {}", String::from_utf8_lossy(&text));
                    }
                    // MAV_SYS_STATUS_PREARM_CHECK: ArduPilot's own pre-arm checks pass.
                    if let communication_telemetry::MavMessage::SYS_STATUS(st) = &msg {
                        core.lock().unwrap().ap.prearm_ok = st.onboard_control_sensors_health.bits() & 0x1000_0000 != 0;
                    }
                    if let communication_telemetry::MavMessage::HEARTBEAT(h) = msg {
                        // The autopilot's own heartbeat, not a ground station's.
                        if h.autopilot as u32 == ARDUPILOTMEGA {
                            let armed = h.base_mode.bits() & 128 != 0;
                            core.lock().unwrap().ap.gcs = Some((Instant::now(), armed, plane_mode_name(h.custom_mode)));
                        }
                    }
                }
            }
        })
        .expect("spawn heartbeat thread");
}
