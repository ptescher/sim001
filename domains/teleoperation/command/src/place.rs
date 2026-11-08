//! Where the app's programs and files are: in the checkout, or in the app bundle, where the
//! launcher says (`SIM001_HOME` the resources, `SIM001_BIN` the programs, `SIM001_DATA` the
//! user's data folder, since a bundle is read-only).

use std::path::{Path, PathBuf};

/// A program's file name on this OS.
pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Where the app's programs and files are.
#[derive(Clone, Debug)]
pub struct Place {
    /// The resources: the checkout, or the bundle's.
    pub home: PathBuf,
    /// The bundle's programs, if the launcher said where.
    bin: Option<PathBuf>,
    /// Where the app may write: the user's data folder in a bundle.
    data: PathBuf,
}

impl Place {
    /// As the launcher set it up (`SIM001_HOME`, `SIM001_BIN`, `SIM001_DATA`), else the checkout.
    pub fn from_env() -> Place {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        Place::new(var("SIM001_HOME"), var("SIM001_BIN"), var("SIM001_DATA"))
    }

    pub fn new(home: Option<PathBuf>, bin: Option<PathBuf>, data: Option<PathBuf>) -> Place {
        let home = home.unwrap_or_else(checkout);
        let data = data.unwrap_or_else(|| home.clone());
        Place { home, bin, data }
    }

    /// The program `name`, and whether it must be built first: the bundle's if it has it, else
    /// the workspace's (with SIM001_BIN naming an ArduPlane folder, the programs are not there).
    pub fn program(&self, name: &str) -> (PathBuf, bool) {
        match self.bin.as_ref().map(|b| b.join(exe(name))).filter(|p| p.exists()) {
            Some(p) => (p, false),
            None => (self.home.join("target/release").join(exe(name)), true),
        }
    }

    /// Where the game server and the 3D view write their logs.
    pub fn logs(&self) -> PathBuf {
        self.data.join("logs/towers")
    }

    /// The app's assets (the map's picture of the valley), at the same place in the bundle as in
    /// the checkout.
    pub fn assets(&self) -> PathBuf {
        self.home.join("domains/teleoperation/command/assets")
    }

    /// The suite of expectations (for `--expect`).
    pub fn expectations(&self) -> PathBuf {
        self.home.join("expectations")
    }
}

/// The checkout this app was built in.
fn checkout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sim001-place-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// In the bundle the game server and the 3D view are beside the launcher, and there is
    /// nothing to build; the logs go to the user's data folder.
    #[test]
    fn in_the_bundle_the_programs_are_beside_the_launcher() {
        let (home, bin, data) = (temp("home"), temp("bin"), temp("data"));
        std::fs::write(bin.join(exe("towers_server")), "").unwrap();
        let p = Place::new(Some(home.clone()), Some(bin.clone()), Some(data.clone()));
        assert_eq!(p.program("towers_server"), (bin.join(exe("towers_server")), false));
        assert_eq!(p.logs(), data.join("logs/towers"));
        assert_eq!(p.assets(), home.join("domains/teleoperation/command/assets"));
    }

    /// From the checkout (or with SIM001_BIN naming only an ArduPlane folder) the programs are the
    /// workspace's, built first; the logs stay in the checkout.
    #[test]
    fn from_the_checkout_the_programs_are_built() {
        let (home, agent) = (temp("home2"), temp("agent"));
        std::fs::write(agent.join(exe("arduplane")), "").unwrap();
        let p = Place::new(Some(home.clone()), Some(agent), None);
        assert_eq!(p.program("towers_server"), (home.join("target/release").join(exe("towers_server")), true));
        assert_eq!(p.logs(), home.join("logs/towers"));
        let none = Place::new(None, None, None);
        assert!(none.home.join("Cargo.toml").exists(), "the checkout: {}", none.home.display());
    }
}
