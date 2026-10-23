//! Where a station's parts talk to each other, for station number `worker`: several stations
//! (the acceptance tests run on more than one) must not share a port, a container or a folder.
//!
//! Worker 0 is the station as it always was. Each worker takes [`INSTANCES`] SITL instances, its
//! own plane and the computer pilots', so that its ports (MAVLink 5760 + 10·N, physics
//! 9002 + 10·N) lie clear of the next worker's.

/// The SITL instances a worker uses: its plane, and one for each computer pilot.
pub const INSTANCES: usize = 4;

/// The environment variable that tells the sim which worker it belongs to.
pub const ENV: &str = "SIM001_WORKER";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ports {
    pub worker: usize,
}

impl Ports {
    pub fn new(worker: usize) -> Self {
        Ports { worker }
    }

    /// The worker the environment names (`SIM001_WORKER`), else the first.
    pub fn from_env() -> Self {
        Ports::new(std::env::var(ENV).ok().and_then(|w| w.parse().ok()).unwrap_or(0))
    }

    /// The SITL instance of this worker's plane; the computer pilots follow it.
    pub fn instance(self) -> usize {
        self.worker * INSTANCES
    }

    /// Is SITL instance `instance` the plane this worker's console talks to?
    pub fn is_yours(self, instance: usize) -> bool {
        instance == self.instance()
    }

    /// Which of this worker's autopilots `instance` is: 0 is yours, 1 to 3 the computer pilots'.
    pub fn slot(self, instance: usize) -> usize {
        instance - self.instance()
    }

    /// Your autopilot's MAVLink, for the console.
    pub fn mavlink(self) -> u16 {
        crate::mavlink_port(self.instance() as u16)
    }

    /// The UDP port your autopilot sends its physics packets to (the sim's `--ap-port`).
    pub fn physics(self) -> u16 {
        crate::port_for(9002, self.instance() as u16)
    }

    /// The sim's TX15 stand-in. Below 5760, where the first SITL instance starts.
    pub fn radio(self) -> u16 {
        5750 - 2 * self.worker as u16
    }

    /// The sim's flight computer's UART, for the TX15.
    pub fn uart(self) -> u16 {
        self.radio() + 1
    }

    /// The sim's sensor feed.
    pub fn feed(self) -> u16 {
        5600 + self.worker as u16
    }

    /// The Docker container of SITL instance `instance`.
    pub fn container(self, instance: usize) -> String {
        let name = crate::launch::DOCKER_NAME;
        match (self.worker, self.slot(instance)) {
            (0, 0) => name.to_string(),
            (0, k) => format!("{name}-{k}"),
            (w, 0) => format!("{name}-w{w}"),
            (w, k) => format!("{name}-w{w}-{k}"),
        }
    }

    /// The folder under `logs/` for this worker: the first keeps `logs/` itself.
    pub fn logs(self, root: &std::path::Path) -> std::path::PathBuf {
        if self.worker == 0 { root.join("logs") } else { root.join("logs").join(format!("worker-{}", self.worker)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first worker is the station as it was: nothing a user has set up moves.
    #[test]
    fn the_first_worker_keeps_the_old_ports() {
        let p = Ports::new(0);
        assert_eq!((p.instance(), p.mavlink(), p.physics()), (0, 5760, 9002));
        assert_eq!((p.radio(), p.uart(), p.feed()), (5750, 5751, 5600));
        assert_eq!((p.container(0), p.container(2)), ("sim001-sitl".into(), "sim001-sitl-2".into()));
        assert_eq!(p.logs(std::path::Path::new("/r")), std::path::Path::new("/r/logs"));
    }

    /// Two workers share nothing: not an instance, a port, a container or a folder.
    #[test]
    fn workers_share_nothing() {
        let (a, b) = (Ports::new(0), Ports::new(1));
        assert_eq!((b.instance(), b.mavlink(), b.physics()), (4, 5800, 9042));
        assert_ne!((a.radio(), a.uart(), a.feed()), (b.radio(), b.uart(), b.feed()));
        let instances = |p: Ports| (0..INSTANCES).map(|k| p.instance() + k).collect::<Vec<_>>();
        for (x, y) in instances(a).into_iter().flat_map(|x| instances(b).into_iter().map(move |y| (x, y))) {
            assert_ne!(crate::mavlink_port(x as u16), crate::mavlink_port(y as u16));
            assert_ne!(crate::port_for(9002, x as u16), crate::port_for(9002, y as u16));
            assert_ne!(a.container(x), b.container(y));
        }
        assert_eq!(b.container(5), "sim001-sitl-w1-1");
        assert_eq!(b.slot(6), 2);
        assert!(b.is_yours(4) && !b.is_yours(0));
        assert_ne!(a.logs(std::path::Path::new("/r")), b.logs(std::path::Path::new("/r")));
    }

    /// Every worker's radio, UART and feed ports clear its neighbours' and the SITL ports.
    #[test]
    fn many_workers_keep_their_ports_apart() {
        let mut seen = std::collections::HashSet::new();
        for w in 0..8 {
            let p = Ports::new(w);
            for port in [p.radio(), p.uart(), p.feed()] {
                assert!(seen.insert(port), "worker {w}: port {port} is taken");
                assert!(port < 5760 || port == p.feed());
            }
        }
    }
}
