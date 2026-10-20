//! External things a test needs: a flight controller, a database, a container. The suite names
//! what it needs and the harness starts it, once for the run, in the order the tests ask, stops it
//! when the run is over, and starts it again if it died on the way. What cannot be had on this
//! machine is not a failure: the tests that need it are "not supported yet", with how to get it.
//!
//! A [`Service`] says whether it can run here ([`Service::unavailable`]) and how to start
//! ([`Service::start`]), which gives a [`Running`] that stops when it is dropped. There are
//! providers for a [`Command`] (a program, ready when a port answers) and a [`Docker`] container
//! (built from a directory if its image is not there), and a [`Fallback`] takes the first of
//! several that is available: a native build, else a container. A database is a container.
//!
//! No dependencies beyond the standard library.

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

/// Something that can be started for a test run.
pub trait Service: Send {
    /// What the tests call it.
    fn name(&self) -> &str;

    /// Why it cannot run on this machine, and how to get it so that it can; `None` when it can.
    fn unavailable(&self) -> Option<String> {
        None
    }

    /// Starts it and waits until it is ready.
    fn start(&self) -> Result<Box<dyn Running>, String>;
}

/// A started service. Dropping it stops it.
pub trait Running: Send {
    /// Whether it is still up.
    fn alive(&mut self) -> bool {
        true
    }

    /// Where to find it: `"port"`, `"pid"`, `"container"`, whatever the service tells.
    fn endpoint(&self, key: &str) -> Option<String> {
        let _ = key;
        None
    }
}

/// The services of a run: those that are known, and those that are up.
#[derive(Default)]
pub struct Services {
    known: BTreeMap<String, Box<dyn Service>>,
    up: Vec<(String, Box<dyn Running>)>,
}

impl Services {
    pub fn new() -> Self {
        Services::default()
    }

    /// Makes a service known (by its name), without starting it.
    pub fn add(&mut self, service: impl Service + 'static) {
        self.known.insert(service.name().to_string(), Box::new(service));
    }

    pub fn with(mut self, service: impl Service + 'static) -> Self {
        self.add(service);
        self
    }

    /// Whether anything is known.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// The reason each of `names` cannot be had here, one line each: empty when all can.
    pub fn missing(&self, names: &[&str]) -> Vec<String> {
        names
            .iter()
            .filter_map(|n| match self.known.get(*n) {
                None => Some(format!("{n}: no such service is set up for this run")),
                Some(s) => s.unavailable().map(|why| format!("{n}: {why}")),
            })
            .collect()
    }

    /// Makes sure each of `names` is up, in that order: one that is up stays, one that died is
    /// started again. The error says what cannot be had, or what failed to start.
    pub fn start(&mut self, names: &[&str]) -> Result<(), String> {
        let missing = self.missing(names);
        if !missing.is_empty() {
            return Err(missing.join("; "));
        }
        for name in names {
            if let Some(i) = self.up.iter().position(|(n, _)| n == name) {
                if self.up[i].1.alive() {
                    continue;
                }
                self.up.remove(i);
            }
            let service = &self.known[*name];
            let running = service.start().map_err(|e| format!("{name} would not start: {e}"))?;
            self.up.push((name.to_string(), running));
        }
        Ok(())
    }

    /// Starts these afresh: stopped if they are up, then started. For a service that keeps state
    /// a test must not find (a flight controller armed by the last test).
    pub fn restart(&mut self, names: &[&str]) -> Result<(), String> {
        for name in names {
            if let Some(i) = self.up.iter().position(|(n, _)| n == name) {
                self.up.remove(i);
            }
        }
        self.start(names)
    }

    /// Where a running service says it is (see [`Running::endpoint`]).
    pub fn endpoint(&self, name: &str, key: &str) -> Option<String> {
        self.up.iter().find(|(n, _)| n == name)?.1.endpoint(key)
    }

    /// Stops everything, the last started first (what depends on a database stops before it).
    pub fn stop_all(&mut self) {
        while self.up.pop().is_some() {}
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// How to tell that a started service is ready to be used.
#[derive(Clone, Debug)]
pub enum Ready {
    /// As soon as it has started.
    Started,
    /// When something accepts a connection on this port of this machine.
    Tcp(u16),
    /// After this long.
    After(Duration),
}

fn wait_until_ready(name: &str, ready: &Ready, patience: Duration, mut still_up: impl FnMut() -> bool) -> Result<(), String> {
    match ready {
        Ready::Started => Ok(()),
        Ready::After(d) => {
            std::thread::sleep(*d);
            if still_up() { Ok(()) } else { Err(format!("{name} exited while it was starting")) }
        }
        Ready::Tcp(port) => {
            let addr = SocketAddr::from(([127, 0, 0, 1], *port));
            let start = Instant::now();
            loop {
                if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
                    return Ok(());
                }
                if !still_up() {
                    return Err(format!("{name} exited before port {port} answered"));
                }
                if start.elapsed() > patience {
                    return Err(format!("{name}: nothing answered on port {port} within {} s", patience.as_secs_f32()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Whether something accepts a connection on `port` of this machine.
fn port_answers(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(200)).is_ok()
}

/// Whether `program` can be run: a path that exists, or a name found on `PATH`.
fn installed(program: &str) -> bool {
    let p = std::path::Path::new(program);
    if p.components().count() > 1 {
        return p.exists();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|d| d.join(program).is_file()))
}

/// A program run for the duration of a test run.
#[derive(Clone)]
pub struct Command {
    name: String,
    program: String,
    args: Vec<String>,
    dir: Option<std::path::PathBuf>,
    env: Vec<(String, String)>,
    ready: Ready,
    patience: Duration,
    hint: Option<String>,
}

impl Command {
    pub fn new(name: &str, program: &str) -> Self {
        Command {
            name: name.into(),
            program: program.into(),
            args: vec![],
            dir: None,
            env: vec![],
            ready: Ready::Started,
            patience: Duration::from_secs(30),
            hint: None,
        }
    }

    pub fn arg(mut self, arg: &str) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I: IntoIterator<Item = S>, S: AsRef<str>>(mut self, args: I) -> Self {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_string()));
        self
    }

    /// Runs in this directory (a program that keeps its files where it runs).
    pub fn dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn ready(mut self, ready: Ready) -> Self {
        self.ready = ready;
        self
    }

    /// How long to wait for it to be ready.
    pub fn patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// What to say when the program is not there: how to get it.
    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The program that is run.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Its arguments.
    pub fn argv(&self) -> &[String] {
        &self.args
    }

    /// The directory it runs in, if it has one of its own.
    pub fn working_dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }
}

impl Service for Command {
    fn name(&self) -> &str {
        &self.name
    }

    fn unavailable(&self) -> Option<String> {
        if installed(&self.program) {
            return None;
        }
        Some(match &self.hint {
            Some(h) => format!("{} is not installed: {h}", self.program),
            None => format!("{} is not installed", self.program),
        })
    }

    fn start(&self) -> Result<Box<dyn Running>, String> {
        if let Ready::Tcp(port) = self.ready {
            if port_answers(port) {
                return Err(format!("{}: port {port} is already in use (a leftover of an earlier run? stop it first)", self.name));
            }
        }
        let mut cmd = std::process::Command::new(&self.program);
        cmd.args(&self.args).envs(self.env.iter().map(|(k, v)| (k, v))).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(dir) = &self.dir {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn().map_err(|e| format!("cannot run {}: {e}", self.program))?;
        let name = self.name.clone();
        let port = if let Ready::Tcp(p) = self.ready { Some(p) } else { None };
        let up = |c: &mut Child| c.try_wait().map(|s| s.is_none()).unwrap_or(false);
        let waited = wait_until_ready(&self.name, &self.ready, self.patience, || up(&mut child));
        if let Err(e) = waited {
            let exited = child.try_wait().ok().flatten();
            let _ = child.kill();
            let _ = child.wait();
            return Err(match exited {
                Some(status) => format!("{name} exited ({status}) before it was ready"),
                None => e,
            }
            .replace("exited while it was starting", "exited before it was ready"));
        }
        Ok(Box::new(Process { child, port }))
    }
}

struct Process {
    child: Child,
    port: Option<u16>,
}

impl Running for Process {
    fn alive(&mut self) -> bool {
        self.child.try_wait().map(|s| s.is_none()).unwrap_or(false)
    }

    fn endpoint(&self, key: &str) -> Option<String> {
        match key {
            "pid" => Some(self.child.id().to_string()),
            "port" => self.port.map(|p| p.to_string()),
            _ => None,
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Something that must be there but is not ours to start: a build of a program, a data file. It
/// is available when it exists; starting it is nothing, and it is at its `path`.
pub struct Present {
    name: String,
    path: std::path::PathBuf,
    hint: Option<String>,
}

impl Present {
    pub fn new(name: &str, path: &std::path::Path) -> Self {
        Present { name: name.into(), path: path.to_path_buf(), hint: None }
    }

    /// How to get it.
    pub fn hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl Service for Present {
    fn name(&self) -> &str {
        &self.name
    }

    fn unavailable(&self) -> Option<String> {
        if self.path.exists() {
            return None;
        }
        Some(match &self.hint {
            Some(h) => format!("{} is not there: {h}", self.path.display()),
            None => format!("{} is not there", self.path.display()),
        })
    }

    fn start(&self) -> Result<Box<dyn Running>, String> {
        match self.unavailable() {
            Some(why) => Err(why),
            None => Ok(Box::new(Here(self.path.clone()))),
        }
    }
}

struct Here(std::path::PathBuf);

impl Running for Here {
    fn alive(&mut self) -> bool {
        self.0.exists()
    }

    fn endpoint(&self, key: &str) -> Option<String> {
        (key == "path").then(|| self.0.display().to_string())
    }
}

/// A Docker container: its image is built from a directory when it is not there, it runs
/// detached, and it is removed when it stops. A database for the tests is one of these.
#[derive(Clone)]
pub struct Docker {
    name: String,
    image: String,
    container: String,
    build_dir: Option<String>,
    ports: Vec<(u16, u16)>,
    env: Vec<(String, String)>,
    args: Vec<String>,
    ready: Ready,
    patience: Duration,
    program: String,
}

impl Docker {
    pub fn new(name: &str, image: &str) -> Self {
        Docker {
            name: name.into(),
            image: image.into(),
            container: format!("sim001-{name}"),
            build_dir: None,
            ports: vec![],
            env: vec![],
            args: vec![],
            ready: Ready::Started,
            patience: Duration::from_secs(60),
            program: "docker".into(),
        }
    }

    /// The container's name, which a stale one of the same name is removed by.
    pub fn container(mut self, name: &str) -> Self {
        self.container = name.into();
        self
    }

    /// Where the image is built from, if it is not there.
    pub fn build_from(mut self, dir: &str) -> Self {
        self.build_dir = Some(dir.into());
        self
    }

    /// Publishes the container's `inside` port as `outside` on this machine.
    pub fn port(mut self, inside: u16, outside: u16) -> Self {
        self.ports.push((inside, outside));
        self
    }

    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// An argument for the image's command, after the image.
    pub fn arg(mut self, arg: &str) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn ready(mut self, ready: Ready) -> Self {
        self.ready = ready;
        self
    }

    pub fn patience(mut self, patience: Duration) -> Self {
        self.patience = patience;
        self
    }

    /// Which program is Docker (for tests, and for `podman`).
    pub fn docker_program(mut self, program: &str) -> Self {
        self.program = program.into();
        self
    }

    /// The arguments of `docker run`.
    pub fn run_args(&self) -> Vec<String> {
        let mut a: Vec<String> = ["run", "-d", "--rm", "--name"].iter().map(|s| s.to_string()).collect();
        a.push(self.container.clone());
        for (inside, outside) in &self.ports {
            a.extend(["-p".to_string(), format!("{outside}:{inside}")]);
        }
        for (k, v) in &self.env {
            a.extend(["-e".to_string(), format!("{k}={v}")]);
        }
        a.push(self.image.clone());
        a.extend(self.args.iter().cloned());
        a
    }

    /// The arguments of `docker build`, when there is a directory to build from.
    pub fn build_args(&self) -> Option<Vec<String>> {
        let dir = self.build_dir.as_ref()?;
        Some(["build", "-q", "-t", &self.image, dir].iter().map(|s| s.to_string()).collect())
    }

    fn docker(&self, args: &[String]) -> std::io::Result<std::process::Output> {
        std::process::Command::new(&self.program).args(args).stdin(Stdio::null()).output()
    }

    fn daemon_up(&self) -> bool {
        self.docker(&["info".to_string()]).is_ok_and(|o| o.status.success())
    }

    /// Docker Desktop on a Mac does not run until it is opened: open it and wait for it.
    fn wake_daemon(&self) -> Result<(), String> {
        if self.daemon_up() {
            return Ok(());
        }
        if cfg!(target_os = "macos") {
            let _ = std::process::Command::new("open").args(["-ga", "Docker"]).status();
        }
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            if self.daemon_up() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        Err("the Docker daemon is not running".into())
    }
}

impl Service for Docker {
    fn name(&self) -> &str {
        &self.name
    }

    fn unavailable(&self) -> Option<String> {
        if installed(&self.program) {
            return None;
        }
        Some("Docker is not installed: install Docker Desktop (https://www.docker.com/products/docker-desktop/)".into())
    }

    fn start(&self) -> Result<Box<dyn Running>, String> {
        self.wake_daemon()?;
        let have_image = self.docker(&["image".into(), "inspect".into(), self.image.clone()]).is_ok_and(|o| o.status.success());
        if !have_image {
            let build = self.build_args().ok_or_else(|| format!("the image {} is not built, and there is nothing to build it from", self.image))?;
            let out = self.docker(&build).map_err(|e| format!("docker build: {e}"))?;
            if !out.status.success() {
                return Err(format!("docker build failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        // A container of the same name from an earlier run would stop this one starting.
        let _ = self.docker(&["rm".into(), "-f".into(), self.container.clone()]);
        if let Some((_, outside)) = self.ports.first().filter(|_| matches!(self.ready, Ready::Tcp(_))) {
            if port_answers(*outside) {
                return Err(format!("{}: port {outside} is already in use (a leftover of an earlier run? stop it first)", self.name));
            }
        }
        let out = self.docker(&self.run_args()).map_err(|e| format!("docker run: {e}"))?;
        if !out.status.success() {
            return Err(format!("docker run failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let container = self.container.clone();
        let program = self.program.clone();
        let running = Container { program, container: container.clone(), port: self.ports.first().map(|p| p.1) };
        let up = || running.is_up();
        wait_until_ready(&self.name, &self.ready, self.patience, up)?;
        Ok(Box::new(running))
    }
}

struct Container {
    program: String,
    container: String,
    port: Option<u16>,
}

impl Container {
    fn is_up(&self) -> bool {
        std::process::Command::new(&self.program)
            .args(["inspect", "-f", "{{.State.Running}}", &self.container])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
    }
}

impl Running for Container {
    fn alive(&mut self) -> bool {
        self.is_up()
    }

    fn endpoint(&self, key: &str) -> Option<String> {
        match key {
            "container" => Some(self.container.clone()),
            "port" => self.port.map(|p| p.to_string()),
            _ => None,
        }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.program).args(["rm", "-f", &self.container]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

/// One service, made of several ways to have it: the first that can run here runs.
pub struct Fallback {
    name: String,
    ways: Vec<Box<dyn Service>>,
}

impl Fallback {
    pub fn new(name: &str, ways: Vec<Box<dyn Service>>) -> Self {
        Fallback { name: name.into(), ways }
    }
}

impl Service for Fallback {
    fn name(&self) -> &str {
        &self.name
    }

    fn unavailable(&self) -> Option<String> {
        let reasons: Vec<String> = self.ways.iter().map(|w| w.unavailable()).collect::<Option<_>>()?;
        Some(reasons.join("; or ").to_string())
    }

    fn start(&self) -> Result<Box<dyn Running>, String> {
        self.ways.iter().find(|w| w.unavailable().is_none()).ok_or("no way to start it here")?.start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A service that counts how it is used.
    #[derive(Clone, Default)]
    struct Fake {
        name: &'static str,
        starts: Arc<Mutex<u32>>,
        stops: Arc<Mutex<u32>>,
        dead: Arc<Mutex<bool>>,
        why_not: Option<&'static str>,
    }

    struct FakeRunning(Fake);

    impl Service for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn unavailable(&self) -> Option<String> {
            self.why_not.map(String::from)
        }
        fn start(&self) -> Result<Box<dyn Running>, String> {
            *self.starts.lock().unwrap() += 1;
            *self.dead.lock().unwrap() = false;
            Ok(Box::new(FakeRunning(self.clone())))
        }
    }

    impl Running for FakeRunning {
        fn alive(&mut self) -> bool {
            !*self.0.dead.lock().unwrap()
        }
        fn endpoint(&self, key: &str) -> Option<String> {
            (key == "port").then(|| "5760".to_string())
        }
    }

    impl Drop for FakeRunning {
        fn drop(&mut self) {
            *self.0.stops.lock().unwrap() += 1;
        }
    }

    fn fake(name: &'static str) -> Fake {
        Fake { name, ..Default::default() }
    }

    /// A test that needs something that cannot be had here is not run and not failed: it says
    /// what is missing and how to get it.
    #[test]
    fn a_service_that_cannot_start_here_says_how_to_get_it() {
        let mut s = Services::new();
        s.add(Fake { why_not: Some("build it with `make sitl`"), ..fake("ardupilot") });
        s.add(fake("database"));
        assert_eq!(s.start(&["database"]), Ok(()));
        let err = s.start(&["database", "ardupilot"]).unwrap_err();
        assert!(err.contains("ardupilot") && err.contains("make sitl"), "{err}");
        let unknown = s.start(&["nothing"]).unwrap_err();
        assert!(unknown.contains("nothing") && unknown.contains("no such service"), "{unknown}");
    }

    /// Several plans want the same service: it is started once, for all of them, and stopped
    /// when the run is over.
    #[test]
    fn a_service_starts_once_and_stops_when_the_run_ends() {
        let f = fake("db");
        let (starts, stops) = (f.starts.clone(), f.stops.clone());
        {
            let mut s = Services::new();
            s.add(f);
            s.start(&["db"]).unwrap();
            s.start(&["db"]).unwrap();
            assert_eq!(*starts.lock().unwrap(), 1);
            assert_eq!(s.endpoint("db", "port"), Some("5760".to_string()));
            assert_eq!(s.endpoint("db", "nothing"), None);
            assert_eq!(*stops.lock().unwrap(), 0);
        }
        assert_eq!(*stops.lock().unwrap(), 1, "dropped, so stopped");
    }

    /// A service that died between tests (a crash it did not survive) is started again for the next.
    #[test]
    fn a_service_that_died_is_started_again() {
        let f = fake("sitl");
        let (starts, stops, dead) = (f.starts.clone(), f.stops.clone(), f.dead.clone());
        let mut s = Services::new();
        s.add(f);
        s.start(&["sitl"]).unwrap();
        *dead.lock().unwrap() = true;
        s.start(&["sitl"]).unwrap();
        assert_eq!((*starts.lock().unwrap(), *stops.lock().unwrap()), (2, 1));
    }

    /// A restart is a stop and a start, so that what the last test left in a service is gone.
    #[test]
    fn a_restart_is_a_fresh_start() {
        let f = fake("sitl");
        let (starts, stops) = (f.starts.clone(), f.stops.clone());
        let mut s = Services::new();
        s.add(f);
        s.start(&["sitl"]).unwrap();
        s.restart(&["sitl"]).unwrap();
        assert_eq!((*starts.lock().unwrap(), *stops.lock().unwrap()), (2, 1));
        // A service that is not up is just started.
        let mut other = Services::new();
        other.add(fake("db"));
        other.restart(&["db"]).unwrap();
        assert!(other.endpoint("db", "port").is_some());
    }

    #[test]
    fn services_stop_in_the_opposite_order_they_started() {
        let order = Arc::new(Mutex::new(Vec::<&str>::new()));
        struct Ordered(&'static str, Arc<Mutex<Vec<&'static str>>>);
        struct OrderedRunning(&'static str, Arc<Mutex<Vec<&'static str>>>);
        impl Service for Ordered {
            fn name(&self) -> &str {
                self.0
            }
            fn start(&self) -> Result<Box<dyn Running>, String> {
                Ok(Box::new(OrderedRunning(self.0, self.1.clone())))
            }
        }
        impl Running for OrderedRunning {}
        impl Drop for OrderedRunning {
            fn drop(&mut self) {
                self.1.lock().unwrap().push(self.0);
            }
        }
        {
            let mut s = Services::new();
            s.add(Ordered("database", order.clone()));
            s.add(Ordered("app", order.clone()));
            s.start(&["database", "app"]).unwrap();
        }
        assert_eq!(*order.lock().unwrap(), vec!["app", "database"], "what depends on the database stops first");
    }

    /// The first service of a list that can run here is the one that runs: a native build, else a container.
    #[test]
    fn a_fallback_takes_the_first_that_is_available() {
        let (native, docker) = (Fake { why_not: Some("not built"), ..fake("native") }, fake("docker"));
        let docker_starts = docker.starts.clone();
        let mut s = Services::new();
        s.add(Fallback::new("sitl", vec![Box::new(native.clone()), Box::new(docker.clone())]));
        s.start(&["sitl"]).unwrap();
        assert_eq!(*docker_starts.lock().unwrap(), 1);
        assert_eq!(*native.starts.lock().unwrap(), 0);

        let mut none = Services::new();
        none.add(Fallback::new("sitl", vec![Box::new(native.clone()), Box::new(Fake { why_not: Some("no docker"), ..fake("docker") })]));
        let err = none.start(&["sitl"]).unwrap_err();
        assert!(err.contains("not built") && err.contains("no docker"), "every way is named: {err}");
    }

    /// A command is a service when it can be run here; its readiness is a port answering.
    #[test]
    fn a_command_is_ready_when_its_port_answers() {
        // The program opens its port a moment after it starts (here, a thread does).
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let opener = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
            std::thread::sleep(std::time::Duration::from_secs(3));
            drop(listener);
        });
        let sleeper = Command::new("sleeper", "sleep").arg("30").ready(Ready::Tcp(port)).patience(std::time::Duration::from_secs(5));
        let began = std::time::Instant::now();
        let mut running = sleeper.start().expect("starts and is ready");
        assert!(began.elapsed() >= std::time::Duration::from_millis(300), "it did not wait for the port");
        assert!(running.alive());
        assert_eq!(running.endpoint("port"), Some(port.to_string()));
        opener.join().unwrap();

        // Nothing answers on a port that is closed: the wait ends, and says so.
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let slow = Command::new("slow", "sleep").arg("30").ready(Ready::Tcp(closed)).patience(std::time::Duration::from_millis(300));
        let err = slow.start().err().expect("never ready");
        assert!(err.contains("slow") && err.contains(&closed.to_string()), "{err}");
    }

    /// A port that something else already answers on is not ours to wait for: what answers there
    /// is a leftover of an earlier run, and a test that talked to it would be testing that.
    #[test]
    fn a_port_somebody_else_holds_is_not_ours_to_wait_for() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let c = Command::new("sitl", "sleep").arg("30").ready(Ready::Tcp(port));
        let err = c.start().err().expect("the port is taken");
        assert!(err.contains("sitl") && err.contains(&port.to_string()) && err.contains("already"), "{err}");
        drop(listener);
    }

    #[test]
    fn a_command_is_stopped_when_it_is_dropped() {
        let sleeper = Command::new("sleeper", "sleep").arg("30");
        let running = sleeper.start().unwrap();
        let pid = running.endpoint("pid").expect("a pid");
        let gone = |pid: &str| !std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().unwrap().success();
        assert!(!gone(&pid), "it is running");
        drop(running);
        let mut tries = 0;
        while !gone(&pid) && tries < 50 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            tries += 1;
        }
        assert!(gone(&pid), "it is still running");
    }

    #[test]
    fn a_command_that_is_not_installed_is_unavailable_with_a_hint() {
        let c = Command::new("thing", "definitely-not-a-program-sim001").hint("install it from example.org");
        let why = c.unavailable().expect("not there");
        assert!(why.contains("definitely-not-a-program-sim001") && why.contains("example.org"), "{why}");
        assert!(Command::new("sleeper", "sleep").unavailable().is_none());
    }

    /// Something that must be there but is not ours to start (a build, a file): available when
    /// it exists, and then it is simply there, at its path.
    #[test]
    fn a_thing_that_must_be_there_is_available_when_it_exists() {
        let here = std::env::current_exe().unwrap();
        let present = Present::new("tool", &here).hint("build it first");
        assert_eq!(present.unavailable(), None);
        let mut running = present.start().unwrap();
        assert!(running.alive());
        assert_eq!(running.endpoint("path"), Some(here.display().to_string()));

        let gone = Present::new("tool", std::path::Path::new("/definitely/not/here/tool")).hint("build it first");
        let why = gone.unavailable().expect("missing");
        assert!(why.contains("/definitely/not/here/tool") && why.contains("build it first"), "{why}");
        assert!(gone.start().is_err());
    }

    #[test]
    fn a_command_that_exits_at_once_is_not_ready() {
        let c = Command::new("quitter", "true").ready(Ready::Tcp(1)).patience(std::time::Duration::from_secs(2));
        let err = c.start().err().expect("it quit");
        assert!(err.contains("quitter") && err.contains("exited"), "{err}");
    }

    /// What `docker run` is told for a container: it is detached, removed when it stops, named,
    /// with its ports, environment and image.
    #[test]
    fn a_container_runs_detached_with_its_ports_and_environment() {
        let postgres = Docker::new("database", "postgres:16").container("sim001-test-db").port(5432, 55432).env("POSTGRES_PASSWORD", "test").ready(Ready::Tcp(55432));
        assert_eq!(
            postgres.run_args(),
            vec!["run", "-d", "--rm", "--name", "sim001-test-db", "-p", "55432:5432", "-e", "POSTGRES_PASSWORD=test", "postgres:16"]
        );
        let with_command = Docker::new("x", "img").container("c").arg("--flag");
        assert_eq!(with_command.run_args().last().map(String::as_str), Some("--flag"), "arguments go after the image");
    }

    /// Without Docker installed, a container service is unavailable and says where to get it.
    #[test]
    fn a_container_needs_docker() {
        let d = Docker::new("database", "postgres:16").docker_program("definitely-not-docker-sim001");
        let why = d.unavailable().expect("no docker");
        assert!(why.contains("Docker") && why.contains("docker.com"), "{why}");
    }

    /// An image that is not there is built from its directory, when it has one, and the
    /// service is not available when it has neither.
    #[test]
    fn an_image_is_built_from_its_directory() {
        let d = Docker::new("sitl", "sim001-ardupilot-sitl").build_from("ardupilot");
        assert_eq!(d.build_args().unwrap(), vec!["build", "-q", "-t", "sim001-ardupilot-sitl", "ardupilot"]);
        assert!(Docker::new("x", "img").build_args().is_none());
    }
}
