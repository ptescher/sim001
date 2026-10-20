//! ArduPlane SITL as a service of a test run (see `expectation_service`): a native build if there
//! is one, else the Docker image, built from `ardupilot/` the first time. The tests that need a
//! flight controller declare it; the harness starts it once for the run and stops it after, and
//! when neither a build nor Docker is there those tests are "not supported yet", with how to get one.

use crate::launch::{native_args, native_binary, DOCKER_IMAGE, DOCKER_NAME, MAVLINK_PORT};
use expectation_service::{Command, Docker, Fallback, Ready};
use std::path::Path;
use std::time::Duration;

/// The name tests give it.
pub const NAME: &str = "ardupilot";

/// A native ArduPlane for `aircraft`, run in `work` (SITL keeps its files where it runs).
pub fn native(root: &Path, aircraft: &str, work: &Path) -> Command {
    Command::new(NAME, &native_binary(root).display().to_string())
        .args(native_args(root, aircraft))
        .dir(work)
        .ready(Ready::Tcp(MAVLINK_PORT))
        .patience(Duration::from_secs(60))
        .hint("build ArduPilot once with `ardupilot/build_native.sh`")
}

/// The same in Docker: the image is built from `ardupilot/` if it is not there.
pub fn docker(root: &Path, aircraft: &str) -> Docker {
    Docker::new(NAME, DOCKER_IMAGE)
        .container(DOCKER_NAME)
        .build_from(&root.join("ardupilot").display().to_string())
        .port(MAVLINK_PORT, MAVLINK_PORT)
        .env("AIRCRAFT", aircraft)
        .env("NOWAIT", "1")
        .ready(Ready::Tcp(MAVLINK_PORT))
        .patience(Duration::from_secs(120))
}

/// Native if it is built, else Docker.
pub fn ardupilot(root: &Path, aircraft: &str, work: &Path) -> Fallback {
    Fallback::new(NAME, vec![Box::new(native(root, aircraft, work)), Box::new(docker(root, aircraft))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use expectation_service::Service;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    /// The native run is the sim's own: JSON physics from this machine, the plane's parameters,
    /// no wait for a ground station.
    #[test]
    fn the_native_run_flies_the_plane_with_its_parameters() {
        let c = native(&root(), "t1s-vtol", Path::new("/tmp/sitl-work"));
        assert!(c.program().ends_with("arduplane"), "{}", c.program());
        let a = c.argv().join(" ");
        assert!(a.contains("--model JSON:127.0.0.1"), "{a}");
        assert!(a.contains("t1s.parm") && a.contains("t1s-vtol.parm"), "{a}");
        assert!(a.contains("--serial0 tcp:0"), "no waiting for a ground station: {a}");
        assert_eq!(c.working_dir(), Some(Path::new("/tmp/sitl-work")));
    }

    /// The container is the station's: the same image and name, MAVLink on the same port, told
    /// which plane to fly and not to wait.
    #[test]
    fn the_container_is_the_stations() {
        let d = docker(&root(), "t1s-vtol");
        let args = d.run_args().join(" ");
        assert!(args.contains(&format!("--name {DOCKER_NAME}")), "{args}");
        assert!(args.contains(&format!("-p {MAVLINK_PORT}:{MAVLINK_PORT}")), "{args}");
        assert!(args.contains("-e AIRCRAFT=t1s-vtol") && args.contains("-e NOWAIT=1"), "{args}");
        assert!(args.ends_with(DOCKER_IMAGE), "{args}");
        assert!(d.build_args().unwrap().last().unwrap().ends_with("ardupilot"));
    }

    /// With neither a build nor Docker, both ways are named in what it says.
    #[test]
    fn with_no_way_to_have_it_it_says_every_way() {
        let nowhere = Path::new("/definitely/not/a/checkout");
        let f = Fallback::new(
            NAME,
            vec![Box::new(native(nowhere, "t1s-vtol", Path::new("/tmp/x"))), Box::new(docker(nowhere, "t1s-vtol").docker_program("definitely-not-docker-sim001"))],
        );
        let why = f.unavailable().expect("neither is there");
        assert!(why.contains("build_native.sh") && why.contains("Docker"), "{why}");
    }
}
