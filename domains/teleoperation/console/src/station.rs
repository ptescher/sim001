//! `--station`: bring the whole simulator station up from the ground station.
//!
//! [`prepare`] gets every prerequisite in place first, in the terminal: builds the sim, exports
//! the terrain map if missing, starts Docker Desktop, builds the ArduPlane SITL image if
//! missing and (re)starts the SITL container. Then [`start_sim`] starts sim001 as SITL's
//! physics with the sensor feed on, in its pane. The sim stops when the console closes.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use teleoperation_shared::layout::Rect;

const SITL_IMAGE: &str = "sim001-ardupilot-sitl";
const SITL_CONTAINER: &str = "sim001-sitl";

pub struct Options {
    pub raytrace: bool,
    pub feed_port: u16,
    /// Skip `cargo build` of the sim (use the existing binary).
    pub no_build: bool,
    /// Start the sim with the sticks flying (ArduPilot watching); J / D-pad down toggles.
    pub direct: bool,
    /// A real place from `maps/<name>` (None: the procedural Alpine Valley).
    pub map: Option<String>,
    /// The plane to fly (`sim001 --aircraft`); None is the sim's default.
    pub aircraft: Option<String>,
    /// The sim takes its radio from the console (`--test-radio`), for acceptance tests.
    pub test_radio: bool,
    /// The sim stands in for the TX15 on this port, and the console takes its telemetry from
    /// there (None: the console talks to ArduPilot directly).
    pub radio_port: Option<u16>,
    /// With the radio: the TX15 stand-in and ArduPilot talk through the sim's flight computer,
    /// on a UART in CRSF, and the console hears only what comes down that line.
    pub flight_computer: bool,
    /// With the flight computer, where the simulated TX15 writes its screen (for the tests).
    pub tx15_screen: Option<PathBuf>,
    /// A real TX15 on its USB cable (a serial device): the sim's flight computer is on that line,
    /// and the console's telemetry comes through the radio.
    pub tx15_serial: Option<String>,
}

/// Where the sim's TX15 stand-in listens. Below 5760: ArduPlane SITL instance N takes 5760 + 10·N (and the
/// ports after it for its other UARTs), so a computer pilot's plane would answer on 5770.
pub const RADIO_PORT: u16 = 5750;
/// Where the sim's flight computer listens for the TX15 (a virtual UART), with `--flight-computer`.
pub const UART_PORT: u16 = 5751;

/// The map pack's metadata (home position etc.), if `name` has been built.
pub fn pack_meta(name: &str) -> Option<visualization_mappack::PackMeta> {
    visualization_mappack::PackMeta::read(&repo_root().join("maps").join(name).join("build")).ok()
}

/// Repository root (the sim's crate), three levels above this crate (domains/teleoperation/console).
pub fn repo_root() -> PathBuf {
    // In an app bundle the launcher says where the resources are (SIM001_HOME).
    if let Some(home) = std::env::var_os("SIM001_HOME") {
        return PathBuf::from(home);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Where the sim's working files go (controls.cfg, flight logs): the checkout, or the user's data folder in an app bundle.
fn work_dir() -> PathBuf {
    std::env::var_os("SIM001_DATA").map(PathBuf::from).unwrap_or_else(repo_root)
}

/// Where the logs of a run go: `logs/` under the working directory (the console's own debug log,
/// each ArduPlane's output and its flight logs).
pub fn logs_dir() -> PathBuf {
    work_dir().join("logs")
}

fn sim_binary() -> PathBuf {
    if let Some(bin) = std::env::var_os("SIM001_BIN") {
        return PathBuf::from(bin).join(if cfg!(windows) { "sim001.exe" } else { "sim001" });
    }
    repo_root().join("target/release/sim001")
}

/// A native ArduPlane SITL (ardupilot/build_native.sh, or the one in an app bundle), if there is one:
/// the station then needs no Docker.
pub fn native_arduplane() -> Option<PathBuf> {
    let name = if cfg!(windows) { "arduplane.exe" } else { "arduplane" };
    let bundled = std::env::var_os("SIM001_BIN").map(|b| PathBuf::from(b).join(name));
    let built = repo_root().join(".ardupilot/build/sitl/bin").join(name);
    [bundled, Some(built)].into_iter().flatten().find(|p| p.exists())
}

/// The command line of a native ArduPlane for SITL instance `instance` (0 is yours; 1 to 3 are the
/// computer pilots', whose physics go to port 9002 + 10·N). As ardupilot/start.sh runs it in Docker.
fn native_sitl_args(instance: usize, aircraft: &str, home_loc: Option<&str>) -> Vec<String> {
    let root = repo_root();
    let parm = |name: &str| root.join("ardupilot").join(name);
    let mut defaults = vec![root.join(".ardupilot/Tools/autotest/models/plane.parm"), parm("t1s.parm")];
    if parm(&format!("{aircraft}.parm")).exists() {
        defaults.push(parm(&format!("{aircraft}.parm")));
    }
    let defaults: Vec<String> = defaults.iter().map(|p| p.display().to_string()).collect();
    let mut a: Vec<String> = [
        "--model",
        "JSON:127.0.0.1",
        "--speedup",
        "1",
        "--home",
        home_loc.unwrap_or("-35.363261,149.165230,584,353"),
        "--defaults",
    ]
    .map(String::from)
    .to_vec();
    a.push(defaults.join(","));
    // Nobody connects to a computer pilot's autopilot, so it does not wait for a ground station.
    if instance != 0 {
        a.extend(["--serial0".into(), "tcp:0".into()]);
    }
    a.push(format!("-I{instance}"));
    a
}

/// The native ArduPlanes this station started, stopped when it ends.
static NATIVE: std::sync::Mutex<Vec<Child>> = std::sync::Mutex::new(Vec::new());

/// Starts SITL instance `instance` as a native process (a fresh one: the ones of an earlier
/// station are gone, since it stopped them). Its files and flight logs go to its own folder.
fn start_native(program: &Path, instance: usize, aircraft: &str, home_loc: Option<&str>) -> Result<String, String> {
    let logs = if instance == 0 { work_dir().join("logs/sitl") } else { work_dir().join(format!("logs/sitl/opponent-{instance}")) };
    std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
    // An earlier Docker run of the same instance keeps tcp:5760 (and the JSON port) published, so the
    // native one would fail to bind and exit without a word.
    let container = if instance == 0 { SITL_CONTAINER.to_string() } else { format!("{SITL_CONTAINER}-{instance}") };
    let _ = Command::new("docker").args(["rm", "-f", &container]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    // And an ArduPlane of this instance left behind by an earlier station that did not get to stop it.
    let _ = Command::new("pkill")
        .args(["-f", &format!("arduplane --model JSON:.* -I{instance}$")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (out, err) = process_log(&logs).map_err(|e| format!("opening the log of ArduPlane {instance}: {e}"))?;
    let mut child = Command::new(program)
        .args(native_sitl_args(instance, aircraft, home_loc))
        .current_dir(&logs)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("starting {}: {e}", program.display()))?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    if let Ok(Some(status)) = child.try_wait() {
        return Err(format!("ArduPlane {instance} exited right away ({status}); is its MAVLink port already taken?"));
    }
    NATIVE.lock().unwrap().push(child);
    Ok(if instance == 0 {
        format!("started ArduPlane (MAVLink on tcp:5760, flight logs in {})", logs.display())
    } else {
        format!("started a computer pilot's ArduPlane {instance} (flight logs in {})", logs.display())
    })
}

/// Where an autopilot's output goes (stdout and stderr of the one process): `arduplane.log` in its
/// folder, after a line that says when this start was.
fn process_log(dir: &Path) -> std::io::Result<(Stdio, Stdio)> {
    use std::io::Write;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("arduplane.log"))?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    writeln!(&log, "--- started {secs} ---")?;
    let err = log.try_clone()?;
    Ok((Stdio::from(log), Stdio::from(err)))
}

fn stop_native() {
    for mut c in NATIVE.lock().unwrap().drain(..) {
        let _ = c.kill();
        let _ = c.wait();
    }
}

fn step(n: usize, total: usize, what: &str) {
    println!("\x1b[1mstation [{n}/{total}]\x1b[0m {what}");
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

/// Everything the station needs before it opens windows. Prints progress; long steps (the sim
/// build, the one-time SITL image build) stream their own output.
pub fn prepare(opts: &Options, assets: &Path) -> Result<(), String> {
    let root = repo_root();
    let total = 5;

    step(1, total, "building the sim (cargo build --release)");
    if opts.no_build {
        println!("  skipped (--no-build)");
    } else {
        // Under `cargo run`, $CARGO is the cargo running us; the repo's rust-toolchain.toml
        // pins the same toolchain for the sim.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        run(
            Command::new(cargo)
                .args(["build", "--release"])
                .current_dir(&root),
            "building the sim",
        )?;
    }
    if !sim_binary().exists() {
        return Err(format!(
            "{} missing after the build",
            sim_binary().display()
        ));
    }

    if let Some(name) = &opts.map {
        step(2, total, &format!("map pack for {name}"));
        if pack_meta(name).is_some() && assets.join("terrain_map.txt").exists() {
            println!("  maps/{name}/build (present)");
        } else {
            // Downloads USGS lidar + NAIP photos + OSM (a few minutes the first time).
            println!(
                "  building maps/{name} (USGS 3DEP lidar, NAIP photos, OpenStreetMap): a few minutes the first time"
            );
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            run(
                Command::new(cargo)
                    .args([
                        "run",
                        "--release",
                        "--manifest-path",
                        "domains/visualization/mapbuild/Cargo.toml",
                        "--",
                        name,
                    ])
                    .current_dir(&root),
                "building the map pack",
            )?;
        }
    } else {
        step(2, total, "terrain map for the tactical display");
        if assets.join("terrain_map.png").exists() && assets.join("terrain_map.txt").exists() {
            println!("  {} (present)", assets.display());
        } else {
            run(
                Command::new(sim_binary())
                    .arg("--export-map")
                    .arg(assets)
                    .current_dir(&root),
                "exporting the terrain map",
            )?;
        }
    }

    let native = native_arduplane();
    step(3, total, "Docker");
    if native.is_some() {
        println!("  not needed (native ArduPlane)");
    } else {
        ensure_docker()?;
    }

    step(4, total, "ArduPlane SITL");
    let have_image = image_id().is_some();
    if let Some(program) = &native {
        println!("  {} (native)", program.display());
    } else if have_image {
        // Cached layers make this a few seconds; it picks up changes to t1s.parm / start.sh.
        run(
            Command::new("docker")
                .args(["build", "-q", "-t", SITL_IMAGE, "ardupilot"])
                .current_dir(&root)
                .stdout(Stdio::null()),
            "updating the SITL image",
        )?;
        println!("  {SITL_IMAGE} (up to date)");
    } else {
        println!(
            "  building {SITL_IMAGE} from ardupilot/Dockerfile: first time only, about 15 minutes"
        );
        run(
            Command::new("docker")
                .args(["build", "-t", SITL_IMAGE, "ardupilot"])
                .current_dir(&root),
            "building the SITL image",
        )?;
    }

    step(
        5,
        total,
        "ArduPlane SITL: yours, and one for each computer pilot",
    );
    // ArduPilot's home must be the sim's pilot box: the map's home, else start.sh's default.
    let home = opts.map.as_deref().and_then(pack_meta).map(|m| {
        (
            format!(
                "{:.7},{:.7},{:.1},{:.0}",
                m.home_lat, m.home_lon, m.home_alt, m.launch_heading
            ),
            m.title,
        )
    });
    // The computer pilots fly planes like yours, each with an autopilot of its own, so that a
    // game is fair: they take as long to get into the air as you do.
    let aircraft = opts.aircraft.as_deref().unwrap_or(simulation_aircraft::default().id);
    if native.is_some() {
        // A leftover from an earlier station would hold the ports.
        stop_native();
    }
    for instance in 0..=recreation_games::PILOTS.len() {
        let started = match &native {
            Some(program) => start_native(program, instance, aircraft, home.as_ref().map(|h| h.0.as_str()))?,
            None => start_sitl(
                instance,
                opts.map.as_deref().unwrap_or("alpine"),
                aircraft,
                home.as_ref().map(|h| h.0.as_str()),
            )?,
        };
        println!("  {started}");
    }
    if let Some((loc, title)) = home {
        println!("  home {loc} ({title})");
    }
    Ok(())
}

fn image_id() -> Option<String> {
    let out = Command::new("docker")
        .args(["image", "inspect", "-f", "{{.Id}}", SITL_IMAGE])
        .output()
        .ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !id.is_empty()).then_some(id)
}

fn docker_up() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn ensure_docker() -> Result<(), String> {
    if Command::new("docker")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err("the docker command isn't installed: install Docker Desktop (https://www.docker.com/products/docker-desktop/)".into());
    }
    if docker_up() {
        println!("  running");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        println!("  starting Docker Desktop...");
        let _ = Command::new("open").args(["-ga", "Docker"]).status();
        let deadline = Instant::now() + Duration::from_secs(180);
        while !docker_up() {
            if Instant::now() > deadline {
                return Err("Docker Desktop didn't come up within 3 minutes".into());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        println!("  running");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("the Docker daemon isn't running: start it and try again".into())
}

/// The container of SITL instance `instance`: 0 is yours, 1 to 3 the computer pilots'.
fn sitl_container(instance: usize) -> String {
    if instance == 0 {
        SITL_CONTAINER.to_string()
    } else {
        format!("{SITL_CONTAINER}-{instance}")
    }
}

/// The `docker run` arguments for SITL instance `instance` (before the image). Yours puts its
/// MAVLink on tcp:5760 for the console; a computer pilot's has nobody to talk to, so it publishes
/// no port. Each keeps its flight logs apart.
fn sitl_run_args(
    instance: usize,
    map: &str,
    aircraft: &str,
    home_loc: Option<&str>,
    logs: &std::path::Path,
) -> Vec<String> {
    let name = sitl_container(instance);
    let mut a: Vec<String> = ["run", "-d", "--rm", "--name", &name]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if instance == 0 {
        a.extend(["-p".into(), "5760:5760".into()]);
    }
    a.extend([
        "--label".into(),
        format!("t1s.map={map}"),
        "--label".into(),
        format!("t1s.aircraft={aircraft}"),
    ]);
    a.extend([
        "-e".into(),
        format!("AIRCRAFT={aircraft}"),
        "-e".into(),
        format!("INSTANCE={instance}"),
    ]);
    if let Some(h) = home_loc {
        a.extend(["-e".into(), format!("HOME_LOC={h}")]);
    }
    a.extend(["-v".into(), format!("{}:/sitl/logs", logs.display())]);
    a
}

/// Starts SITL instance `instance`, or reboots it if it's already running: it was flying a
/// previous sim's aircraft, and ArduPilot's EKF would see the new one jump back to the pilot box.
/// A container from an older image is replaced, so parameter changes take effect.
fn start_sitl(
    instance: usize,
    map: &str,
    aircraft: &str,
    home_loc: Option<&str>,
) -> Result<String, String> {
    let name = sitl_container(instance);
    let name = name.as_str();
    let running = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^{name}$"),
            "--format",
            "{{.Image}}",
        ])
        .output()
        .map_err(|e| format!("docker ps: {e}"))?;
    if !String::from_utf8_lossy(&running.stdout).trim().is_empty() {
        let inspect = |fmt: &str| {
            Command::new("docker")
                .args(["inspect", "-f", fmt, name])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        let same_image = image_id().is_some_and(|id| id == inspect("{{.Image}}"));
        // Its home position belongs to the map it was started for.
        let same_map = inspect("{{index .Config.Labels \"t1s.map\"}}") == map;
        // Its parameters belong to the plane it was started for.
        let same_aircraft = inspect("{{index .Config.Labels \"t1s.aircraft\"}}") == aircraft;
        if same_image && same_map && same_aircraft {
            run(
                Command::new("docker")
                    .args(["restart", "-t", "1", name])
                    .stdout(Stdio::null()),
                "restarting SITL",
            )?;
            return Ok(format!("restarted {name}"));
        }
        println!(
            "  {}: replacing the container",
            if !same_image {
                "the SITL image changed"
            } else if !same_map {
                "different map"
            } else {
                "different aircraft"
            }
        );
    }
    // Gone, stopped, or from an older image.
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // ArduPilot's flight logs (.BIN) land in logs/sitl/ in the repo, so they outlive the container.
    let logs = if instance == 0 {
        work_dir().join("logs/sitl")
    } else {
        work_dir().join(format!("logs/sitl/opponent-{instance}"))
    };
    std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
    let out = Command::new("docker")
        .args(sitl_run_args(instance, map, aircraft, home_loc, &logs))
        .arg(SITL_IMAGE)
        .output()
        .map_err(|e| format!("docker run: {e}"))?;
    if out.status.success() {
        Ok(if instance == 0 {
            format!(
                "started {name} (MAVLink on tcp:5760, flight logs in {})",
                logs.display()
            )
        } else {
            format!(
                "started {name} (a computer pilot's autopilot, flight logs in {})",
                logs.display()
            )
        })
    } else {
        Err(format!(
            "docker run failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Starts the sim (release build) in its pane. Its output goes to the terminal.
pub fn start_sim(
    opts: &Options,
    rect: Option<Rect>,
    home: (f64, f64, f32),
) -> Result<Child, String> {
    let bin = sim_binary();
    let mut cmd = Command::new(&bin);
    cmd.current_dir(work_dir()).args([
        "--ardupilot",
        "--feed",
        &opts.feed_port.to_string(),
        "--exit-with-parent",
    ]);
    if opts.raytrace {
        cmd.arg("--raytrace");
    }
    if opts.direct {
        cmd.arg("--direct");
    }
    if let Some(m) = &opts.map {
        cmd.args(["--map", m]);
    }
    if let Some(a) = &opts.aircraft {
        cmd.args(["--aircraft", a]);
    }
    if opts.test_radio {
        cmd.arg("--test-radio");
    }
    // The console's telemetry comes through the sim's TX15 stand-in, which needs to know where on the earth the pilot box is.
    if let Some(port) = opts.radio_port {
        cmd.args([
            "--radio",
            &port.to_string(),
            "--home",
            &format!("{},{},{}", home.0, home.1, home.2),
        ]);
        if let Some(device) = &opts.tx15_serial {
            cmd.args(["--flight-computer", device]);
        } else if opts.flight_computer {
            if let Some(path) = &opts.tx15_screen {
                cmd.arg("--tx15-screen").arg(path);
            }
            cmd.args([
                "--flight-computer",
                &format!("tcp:{UART_PORT}"),
                "--radio-uart",
                &format!("tcp:127.0.0.1:{UART_PORT}"),
            ]);
        }
    }
    if let Some(r) = rect {
        cmd.args(["--window-rect", &r.to_arg(), "--frameless"]);
    }
    cmd.spawn()
        .map_err(|e| format!("starting {}: {e}", bin.display()))
}

/// The sim this station started, so that every way out of the station stops it.
static SIM: std::sync::Mutex<Option<Child>> = std::sync::Mutex::new(None);

/// Keep the sim that was started, to stop it when the station ends.
pub fn keep_sim(child: Child) {
    *SIM.lock().unwrap() = Some(child);
}

/// Stop the sim that this station started, if it is still there.
pub fn stop_sim() {
    stop_native();
    if let Some(mut child) = SIM.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Leave the station with this exit code, and stop the sim first: a console that quits alone
/// leaves its sim window open on a plane that nobody flies.
pub fn exit(code: i32) -> ! {
    stop_sim();
    std::process::exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Your autopilot talks to the console on tcp:5760. Each computer pilot's is an instance of
    /// its own (so its physics port is 9002 + 10·N), publishes no port, and logs on its own.
    #[test]
    fn a_native_autopilot_is_an_instance_of_its_own_too() {
        let yours = native_sitl_args(0, "t1s-vtol", None);
        assert!(yours.contains(&"-I0".to_string()) && !yours.contains(&"--serial0".to_string()), "yours waits for the console");
        let kestrel = native_sitl_args(1, "t1s-vtol", Some("1,2,3,4"));
        assert!(kestrel.windows(2).any(|w| w == ["--serial0", "tcp:0"]) && kestrel.contains(&"-I1".to_string()));
        assert!(kestrel.windows(2).any(|w| w == ["--home", "1,2,3,4"]));
    }

    /// What each autopilot prints goes to a file in its own folder (where the launcher and the
    /// console's debug panel read it), and a restart adds to it after a line that says so.
    #[test]
    fn what_an_autopilot_prints_is_kept_in_its_folder() {
        let dir = std::env::temp_dir().join(format!("sim001-station-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (out, err) = process_log(&dir).unwrap();
        let mut child = Command::new("echo").arg("ArduPlane says hello").stdout(out).stderr(err).spawn().unwrap();
        child.wait().unwrap();
        let _ = process_log(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join("arduplane.log")).unwrap();
        assert!(text.contains("ArduPlane says hello"), "{text}");
        assert_eq!(text.matches("--- started ").count(), 2, "{text}");
    }

    #[test]
    fn each_autopilot_is_a_container_of_its_own() {
        let logs = std::path::Path::new("/logs");
        let yours = sitl_run_args(0, "alpine", "t1s-vtol", None, logs);
        assert!(yours.windows(2).any(|w| w == ["--name", "sim001-sitl"]));
        assert!(yours.windows(2).any(|w| w == ["-p", "5760:5760"]));
        assert!(yours.contains(&"INSTANCE=0".to_string()));
        let kestrel = sitl_run_args(1, "alpine", "t1s-vtol", Some("1,2,3,4"), logs);
        assert!(kestrel.windows(2).any(|w| w == ["--name", "sim001-sitl-1"]));
        assert!(
            !kestrel.contains(&"-p".to_string()),
            "nobody connects to it"
        );
        assert!(
            kestrel.contains(&"INSTANCE=1".to_string())
                && kestrel.contains(&"HOME_LOC=1,2,3,4".to_string())
        );
        assert!(
            kestrel.contains(&"AIRCRAFT=t1s-vtol".to_string()),
            "a plane like yours"
        );
    }
}
