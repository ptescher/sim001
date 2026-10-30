//! The plan in a file: the text format of Mission Planner's Save WP File and Load WP File (QGC
//! WPL 110), one item to a line, tab separated, and item 0 the home point.

use crate::vehicle::{Waypoint, WpKind};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const HEADER: &str = "QGC WPL 110";

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Keeps the plan in this file, instead of `SIM001_MISSION_FILE` or `mission.wpl` (a test's own).
pub fn use_path(path: PathBuf) {
    let _ = PATH.set(path);
}

/// The file that SAVE writes and LOAD reads.
pub fn path() -> PathBuf {
    PATH.get().cloned().unwrap_or_else(|| {
        std::env::var_os("SIM001_MISSION_FILE").map_or_else(|| PathBuf::from("mission.wpl"), PathBuf::from)
    })
}

fn command(kind: WpKind) -> u16 {
    match kind {
        WpKind::Waypoint => 16,
        WpKind::LoiterUnlim => 17,
        WpKind::LoiterTurns => 18,
        WpKind::Rtl => 20,
        WpKind::Land => 21,
        WpKind::Takeoff => 22,
        WpKind::Other(c) => c,
    }
}

fn kind(command: u16) -> WpKind {
    match command {
        16 => WpKind::Waypoint,
        17 => WpKind::LoiterUnlim,
        18 => WpKind::LoiterTurns,
        20 => WpKind::Rtl,
        21 => WpKind::Land,
        22 => WpKind::Takeoff,
        c => WpKind::Other(c),
    }
}

/// The text of a file with this plan, and `home` as its item 0.
pub fn text(plan: &[Waypoint], home: (f64, f64)) -> String {
    let mut s = format!("{HEADER}\n0\t1\t0\t16\t0\t0\t0\t0\t{:.8}\t{:.8}\t0.000000\t1\n", home.0, home.1);
    for (i, wp) in plan.iter().enumerate() {
        // Frame 3: the altitude is above home.
        let turns = if wp.kind == WpKind::LoiterTurns { 2.0 } else { 0.0 };
        s += &format!(
            "{}\t0\t3\t{}\t{turns}\t0\t0\t0\t{:.8}\t{:.8}\t{:.6}\t1\n",
            i + 1,
            command(wp.kind),
            wp.lat,
            wp.lon,
            wp.alt
        );
    }
    s
}

pub fn save(path: &Path, plan: &[Waypoint], home: (f64, f64)) -> std::io::Result<()> {
    std::fs::write(path, text(plan, home))
}

/// The plan in a file (without its item 0, the home point).
pub fn load(path: &Path) -> Result<Vec<Waypoint>, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = s.lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return Err(format!("{}: not a mission file ({HEADER})", path.display()));
    }
    let mut plan = vec![];
    for (n, l) in lines.enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let f: Vec<&str> = l.split_whitespace().collect();
        let bad = || format!("{}: line {} is not a mission item", path.display(), n + 2);
        if f.len() < 12 {
            return Err(bad());
        }
        if f[0] == "0" {
            continue;
        }
        plan.push(Waypoint {
            kind: kind(f[3].parse().map_err(|_| bad())?),
            lat: f[8].parse().map_err(|_| bad())?,
            lon: f[9].parse().map_err(|_| bad())?,
            alt: f[10].parse().map_err(|_| bad())?,
        });
    }
    Ok(plan)
}

/// Two plans are the same when every item has the same command and place (to a centimetre).
pub fn same(a: &[Waypoint], b: &[Waypoint]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            command(x.kind) == command(y.kind)
                && (x.lat - y.lat).abs() < 1e-7
                && (x.lon - y.lon).abs() < 1e-7
                && (x.alt - y.alt).abs() < 0.01
        })
}

/// How the file stands to the plan: `missing`, `up to date` or `out of date`.
pub fn state(path: &Path, plan: &[Waypoint]) -> &'static str {
    match load(path) {
        Err(_) if !path.exists() => "missing",
        Ok(saved) if same(&saved, plan) => "up to date",
        _ => "out of date",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Vec<Waypoint> {
        vec![
            Waypoint { kind: WpKind::Takeoff, lat: -35.3632, lon: 149.1652, alt: 40.0 },
            Waypoint { kind: WpKind::Waypoint, lat: -35.3641, lon: 149.1660, alt: 40.0 },
            Waypoint { kind: WpKind::Rtl, lat: 0.0, lon: 0.0, alt: 0.0 },
        ]
    }

    #[test]
    fn a_saved_plan_comes_back() {
        let dir = std::env::temp_dir().join(format!("sim001-wpfile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("circuit.wpl");
        assert_eq!(state(&file, &plan()), "missing");
        save(&file, &plan(), (-35.3632, 149.1652)).unwrap();
        assert_eq!(load(&file).unwrap().len(), 3);
        assert_eq!(state(&file, &plan()), "up to date");
        let mut changed = plan();
        changed.push(Waypoint { kind: WpKind::Waypoint, lat: -35.37, lon: 149.17, alt: 50.0 });
        assert_eq!(state(&file, &changed), "out of date");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_mission_is_refused() {
        let dir = std::env::temp_dir().join(format!("sim001-wpfile-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("other.wpl");
        std::fs::write(&file, "hello\n").unwrap();
        assert!(load(&file).is_err());
        assert_eq!(state(&file, &plan()), "out of date");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
