//! The ArduPilot the sim starts for itself, so that a plane with a flight controller flies with
//! no ground station: `sim001` alone, with the T1-S VTOL. A native ArduPlane SITL build (see
//! ardupilot/build_native.sh) is the light way; the Docker image the station uses is the fallback.

use crate::sim::SimCore;
pub use simulation_sitl::launch::{
    DOCKER_NAME, MAVLINK_PORT, Plan, docker_image_exists, native_binary, plan, plane_mode_name,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
}

impl Running {
    fn halt(&mut self) {
        if self.docker {
            let _ = Command::new("docker")
                .args(["rm", "-f", DOCKER_NAME])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The SITL the sim started for itself, kept here until it stops.
static SITL: std::sync::Mutex<Option<Running>> = std::sync::Mutex::new(None);

pub fn keep(running: Running) {
    *SITL.lock().unwrap() = Some(running);
}

/// Stops it (at exit).
pub fn stop() {
    SITL.lock().unwrap().take();
}

/// Start what the plan says. SITL runs in `logs/sitl` under `root`, where it keeps its files.
pub fn start(root: &Path, plan: Plan) -> Result<Running, String> {
    let (child, docker) = spawn(root, plan)?;
    Ok(Running { child, docker })
}

fn spawn(root: &Path, plan: Plan) -> Result<(Child, bool), String> {
    simulation_sitl::launch::spawn(&data_dir(root).join("logs/sitl"), plan)
}

/// Reads SITL's heartbeat over MAVLink and tells the core whether the plane is armed and in which
/// mode, as the ground station does when there is one (the throttle stick and the HUD need it).
pub fn watch_heartbeat(core: Arc<Mutex<SimCore>>) {
    std::thread::Builder::new()
        .name("sitl-heartbeat".into())
        .spawn(move || {
            loop {
                let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", MAVLINK_PORT)) else {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                };
                let mut parser = communication_telemetry::Parser::default();
                let mut buf = [0u8; 512];
                let trace = std::env::var_os("SIM001_TRACE_AP").is_some();
                let mut last_trace = Instant::now();
                // Ask for every stream at 4 Hz, so that the tests can see what ArduPilot sees.
                #[allow(deprecated)] // still the request ArduPilot answers
                let mut ask = communication_telemetry::MavMessage::REQUEST_DATA_STREAM(Default::default());
                #[allow(deprecated)]
                if let communication_telemetry::MavMessage::REQUEST_DATA_STREAM(r) = &mut ask {
                    r.req_message_rate = 4;
                    r.target_system = 1;
                    r.target_component = 1;
                    r.start_stop = 1;
                }
                let _ = std::io::Write::write_all(&mut stream, &communication_telemetry::frame(255, 190, 0, &ask));
                while let Ok(n) = std::io::Read::read(&mut stream, &mut buf) {
                    if n == 0 {
                        break;
                    }
                    parser.push(&buf[..n]);
                    while let Some((_, msg)) = parser.next_message() {
                        // What ArduPilot says (why it won't arm, for one) goes to the terminal.
                        if let communication_telemetry::MavMessage::STATUSTEXT(t) = &msg {
                            let text: Vec<u8> = t.text.iter().copied().take_while(|b| *b != 0).collect();
                            let text = String::from_utf8_lossy(&text).into_owned();
                            println!("ArduPilot: {text}");
                            core.lock().unwrap().ap.watch.say(text);
                        }
                        // SIM001_TRACE_AP=1: what ArduPilot sees, once a second, in the terminal.
                        if trace && last_trace.elapsed() > Duration::from_secs(1) {
                            last_trace = Instant::now();
                            let c = core.lock().unwrap();
                            println!(
                                "ap-trace: [t {:.0} s, sticks {:.2}, ground {}] {}",
                                c.state.time,
                                c.sticks.throttle,
                                c.state.on_ground,
                                c.ap.watch.report(c.ap.vehicle())
                            );
                        }
                        use communication_telemetry::MavMessage as M;
                        match &msg {
                            M::ESTIMATOR_STATUS(e) => core.lock().unwrap().ap.watch.ekf_flags = Some(e.flags.bits()),
                            M::RC_CHANNELS(r) => {
                                core.lock().unwrap().ap.watch.rc = Some([
                                    r.chan1_raw,
                                    r.chan2_raw,
                                    r.chan3_raw,
                                    r.chan4_raw,
                                    r.chan5_raw,
                                    r.chan6_raw,
                                    r.chan7_raw,
                                    r.chan8_raw,
                                ])
                            }
                            M::SERVO_OUTPUT_RAW(s) => {
                                core.lock().unwrap().ap.watch.servo = Some([
                                    s.servo1_raw,
                                    s.servo2_raw,
                                    s.servo3_raw,
                                    s.servo4_raw,
                                    s.servo5_raw,
                                    s.servo6_raw,
                                    s.servo7_raw,
                                    s.servo8_raw,
                                ])
                            }
                            M::VFR_HUD(h) => core.lock().unwrap().ap.watch.hud = Some((h.alt, h.climb, h.throttle)),
                            _ => {}
                        }
                        // MAV_SYS_STATUS_PREARM_CHECK: ArduPilot's own pre-arm checks pass.
                        if let communication_telemetry::MavMessage::SYS_STATUS(st) = &msg {
                            core.lock().unwrap().ap.prearm_ok =
                                st.onboard_control_sensors_health.bits() & 0x1000_0000 != 0;
                        }
                        if let communication_telemetry::MavMessage::HEARTBEAT(h) = msg {
                            // The autopilot's own heartbeat, not a ground station's.
                            if h.autopilot as u32 == ARDUPILOTMEGA {
                                let armed = h.base_mode.bits() & 128 != 0;
                                core.lock().unwrap().ap.gcs =
                                    Some((Instant::now(), armed, plane_mode_name(h.custom_mode)));
                            }
                        }
                    }
                }
            }
        })
        .expect("spawn heartbeat thread");
}
