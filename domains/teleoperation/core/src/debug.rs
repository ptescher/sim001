//! What the ground station knows of the autopilot, as something to debug with: a log of what
//! happened (the link, every command sent and every acknowledgement, what the autopilot said,
//! what its state did, the pings), a ping of the link, and a report of the autopilot's state now.
//!
//! The model is plain data and has no screen: the console draws it in a panel, the launcher reads
//! the file the console writes it to, and a test that fails prints its tail.

use crate::vehicle::{mode_name, Vehicle};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn tag(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    fn from_tag(tag: &str) -> Option<Level> {
        [Level::Trace, Level::Debug, Level::Info, Level::Warn, Level::Error].into_iter().find(|l| l.tag() == tag)
    }

    /// From a MAVLink severity: 0 emergency to 7 debug.
    pub fn from_severity(severity: u8) -> Level {
        match severity {
            0..=3 => Level::Error,
            4 | 5 => Level::Warn,
            6 => Level::Info,
            _ => Level::Debug,
        }
    }
}

/// Where an event comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// What the station itself says: a mode change it noticed, a button it refused.
    Gcs,
    /// The connection to the autopilot: up, down, heartbeat lost and back.
    Link,
    /// What the station told the autopilot to do.
    Cmd,
    /// What the autopilot answered (COMMAND_ACK).
    Ack,
    /// What the autopilot said (STATUSTEXT).
    Fc,
    /// The ping.
    Ping,
    /// What the autopilot's state did.
    State,
    /// The autopilot's process: started, exited.
    Process,
}

impl Source {
    pub fn tag(self) -> &'static str {
        match self {
            Source::Gcs => "gcs",
            Source::Link => "link",
            Source::Cmd => "cmd",
            Source::Ack => "ack",
            Source::Fc => "fc",
            Source::Ping => "ping",
            Source::State => "state",
            Source::Process => "proc",
        }
    }

    fn from_tag(tag: &str) -> Option<Source> {
        [Source::Gcs, Source::Link, Source::Cmd, Source::Ack, Source::Fc, Source::Ping, Source::State, Source::Process].into_iter().find(|s| s.tag() == tag)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugEvent {
    /// Seconds since the log began.
    pub t: f64,
    pub level: Level,
    pub source: Source,
    pub text: String,
}

/// One line of the log: `   12.345 WARN  ack    text`.
pub fn format_line(e: &DebugEvent) -> String {
    format!("{:9.3} {:<5} {:<6} {}", e.t, e.level.tag(), e.source.tag(), e.text)
}

/// The event a line was made from; `None` for any other line (a line of some other program's log).
pub fn parse_line(line: &str) -> Option<DebugEvent> {
    // Three words, then the text (which may begin with spaces of its own).
    let mut rest = line.trim_start();
    let mut words = [""; 3];
    for w in &mut words {
        let end = rest.find(char::is_whitespace)?;
        *w = &rest[..end];
        rest = rest[end..].trim_start_matches(' ');
    }
    let t: f64 = words[0].parse().ok()?;
    Some(DebugEvent { t, level: Level::from_tag(words[1])?, source: Source::from_tag(words[2])?, text: rest.to_string() })
}

/// The log: the latest events, and a file that has them all.
pub struct DebugLog {
    started: Instant,
    events: VecDeque<DebugEvent>,
    capacity: usize,
    dropped: u64,
    file: Option<std::fs::File>,
}

impl DebugLog {
    pub fn new(started: Instant) -> Self {
        Self::with_capacity(started, 3000)
    }

    pub fn with_capacity(started: Instant, capacity: usize) -> Self {
        DebugLog { started, events: VecDeque::new(), capacity, dropped: 0, file: None }
    }

    /// Writes every event from here on to `path` (a new file; its directory is made).
    pub fn to_file(&mut self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        self.file = Some(std::fs::File::create(path)?);
        Ok(())
    }

    pub fn record(&mut self, level: Level, source: Source, text: impl Into<String>) {
        let e = DebugEvent { t: self.started.elapsed().as_secs_f64(), level, source, text: text.into() };
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{}", format_line(&e));
        }
        self.events.push_back(e);
        while self.events.len() > self.capacity {
            self.events.pop_front();
            self.dropped += 1;
        }
    }

    /// The latest `n` events, oldest first.
    pub fn tail(&self, n: usize) -> Vec<&DebugEvent> {
        let skip = self.events.len().saturating_sub(n);
        self.events.iter().skip(skip).collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// How many old events have been let go of.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// The ping: a message the autopilot answers, and how long that takes.
#[derive(Default)]
pub struct PingStats {
    waiting: HashMap<i64, Instant>,
    sent: u64,
    received: u64,
    lost: u64,
    last: Option<f32>,
    min: f32,
    max: f32,
    total: f64,
    /// Tokens given up on, so that a late reply to one is not taken for a reply.
    given_up: Vec<i64>,
}

impl PingStats {
    pub fn sent(&mut self, token: i64, at: Instant) {
        self.sent += 1;
        self.waiting.insert(token, at);
    }

    /// The round trip in milliseconds, if `token` was waiting for this reply.
    pub fn reply(&mut self, token: i64, at: Instant) -> Option<f32> {
        let sent = self.waiting.remove(&token)?;
        let ms = at.saturating_duration_since(sent).as_secs_f32() * 1000.0;
        if self.received == 0 || ms < self.min {
            self.min = ms;
        }
        if self.received == 0 || ms > self.max {
            self.max = ms;
        }
        self.received += 1;
        self.total += ms as f64;
        self.last = Some(ms);
        Some(ms)
    }

    /// Gives up on the pings that have waited longer than `timeout`; returns how many.
    pub fn expire(&mut self, now: Instant, timeout: Duration) -> u32 {
        let old: Vec<i64> = self.waiting.iter().filter(|(_, t)| now.saturating_duration_since(**t) > timeout).map(|(k, _)| *k).collect();
        for k in &old {
            self.waiting.remove(k);
            self.given_up.push(*k);
        }
        self.lost += old.len() as u64;
        if self.given_up.len() > 100 {
            self.given_up.drain(..50);
        }
        old.len() as u32
    }

    pub fn count_sent(&self) -> u64 {
        self.sent
    }

    pub fn count_received(&self) -> u64 {
        self.received
    }

    pub fn count_lost(&self) -> u64 {
        self.lost
    }

    /// How many are waiting for a reply.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    pub fn last_ms(&self) -> Option<f32> {
        self.last
    }

    /// Lost, as a share of those that have been answered or given up on.
    pub fn loss_percent(&self) -> f32 {
        let done = self.received + self.lost;
        if done == 0 { 0.0 } else { self.lost as f32 / done as f32 * 100.0 }
    }

    pub fn summary(&self) -> String {
        if self.sent == 0 {
            return "no ping yet".into();
        }
        if self.received == 0 {
            return format!("no reply to {} ping(s) ({}% lost)", self.sent, self.loss_percent().round());
        }
        format!(
            "{:.1} ms avg ({:.1} min {:.1} max, last {:.1}), {}% lost of {}",
            self.total / self.received as f64,
            self.min,
            self.max,
            self.last.unwrap_or(0.0),
            self.loss_percent().round(),
            self.received + self.lost
        )
    }
}

/// MAV_SYS_STATUS_SENSOR bits and what they are called.
const SENSORS: [(u32, &str); 24] = [
    (0x1, "gyro"),
    (0x2, "accel"),
    (0x4, "mag"),
    (0x8, "baro"),
    (0x10, "airspeed"),
    (0x20, "GPS"),
    (0x40, "optical flow"),
    (0x400, "rate control"),
    (0x800, "attitude control"),
    (0x1000, "yaw control"),
    (0x2000, "altitude control"),
    (0x4000, "position control"),
    (0x8000, "motor outputs"),
    (0x1_0000, "RC receiver"),
    (0x2_0000, "gyro 2"),
    (0x4_0000, "accel 2"),
    (0x8_0000, "mag 2"),
    (0x10_0000, "geofence"),
    (0x20_0000, "AHRS"),
    (0x40_0000, "terrain"),
    (0x100_0000, "logging"),
    (0x200_0000, "battery"),
    (0x1000_0000, "pre-arm check"),
    (0x4000_0000, "propulsion"),
];

/// The sensors that are there and enabled, but not healthy.
pub fn unhealthy_sensors(v: &Vehicle) -> Vec<&'static str> {
    SENSORS.iter().filter(|(bit, _)| v.sensor_ok(*bit) == Some(false)).map(|(_, n)| *n).collect()
}

fn fix_name(fix: u8) -> &'static str {
    match fix {
        0 => "no GPS",
        1 => "no fix",
        2 => "2D",
        3 => "3D",
        4 => "DGPS",
        5 => "RTK float",
        6 => "RTK fixed",
        _ => "?",
    }
}

fn age(t: Instant) -> String {
    format!("{:.0} s ago", t.elapsed().as_secs_f32())
}

/// What the autopilot is, now, as (what, value) lines.
pub fn state_report(v: &Vehicle) -> Vec<(String, String)> {
    let mut r: Vec<(String, String)> = vec![];
    let mut add = |k: &str, val: String| r.push((k.to_string(), val));
    add("link", format!("{} ({})", if v.link.connected { "connected" } else { "down" }, v.link.address));
    add(
        "heartbeat",
        match (v.have_heartbeat, v.link.last_heartbeat) {
            (_, None) => "none yet".into(),
            (_, Some(t)) if t.elapsed().as_secs_f32() < 3.0 => format!("{}, system {}", age(t), v.sysid),
            (_, Some(t)) => format!("LOST, last {}", age(t)),
        },
    );
    add("messages", format!("{} total, {:.0}/s", v.link.messages, v.link.rate));
    add("ping", v.ping.summary());
    add("mode", v.mode().to_string());
    add("armed", if v.armed { "YES".into() } else { "no".into() });
    add("status", format!("MAV_STATE {}", v.system_status));
    let unhealthy = unhealthy_sensors(v);
    add("sensors", if unhealthy.is_empty() { "all present sensors healthy".into() } else { format!("unhealthy: {}", unhealthy.join(", ")) });
    add(
        "ready to arm",
        match v.sensor_ok(0x1000_0000) {
            Some(true) => "yes (its pre-arm checks pass)".into(),
            Some(false) => "no (its pre-arm checks fail)".into(),
            None => "no (it does not say)".into(),
        },
    );
    if let Some((t, text)) = &v.prearm {
        add("pre-arm", format!("{text} ({})", age(*t)));
    }
    add("ekf", format!("flags 0x{:03x}, variances vel {:.2} pos {:.2} hgt {:.2} mag {:.2}", v.ekf_flags, v.ekf_vel_var, v.ekf_pos_var, v.ekf_hgt_var, v.ekf_mag_var));
    add("gps", format!("{} fix, {} satellites, hdop {:.1}", fix_name(v.gps_fix), v.sats, v.hdop));
    add("position", if v.have_position { format!("{:.6}, {:.6}, {:.1} m rel", v.lat, v.lon, v.alt_rel) } else { "none".into() });
    add("home", v.home.map_or("not set".into(), |h| format!("{:.6}, {:.6}, {:.0} m", h.0, h.1, h.2)));
    add("battery", format!("{:.2} V {:.1} A {} mAh", v.batt_v, v.batt_a, v.batt_mah));
    add("last ack", v.acks.back().map_or("none".into(), |a| format!("{}: {} ({})", a.command, a.result, age(a.at))));
    add("mission", if v.mission_status.is_empty() { "--".into() } else { v.mission_status.clone() });
    r
}

/// The last `n` lines of a file, whole lines only; none if it is not there. Reads only the end.
pub fn tail_lines(path: &Path, n: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    const WINDOW: u64 = 64 * 1024;
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let len = f.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(WINDOW);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return vec![];
    }
    let mut bytes = Vec::new();
    let _ = f.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // began in the middle of a line
    }
    let skip = lines.len().saturating_sub(n);
    lines[skip..].iter().map(|l| l.to_string()).collect()
}

/// A log on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct LogFile {
    pub name: String,
    pub path: std::path::PathBuf,
}

/// The `.log` files under `logs/`: what the console writes (`debug/console.log`) and what each
/// ArduPlane prints (`sitl/arduplane.log`, `sitl/opponent-N/arduplane.log`).
pub fn list_logs(logs: &Path) -> Vec<LogFile> {
    let mut out = vec![];
    let mut visit = |path: std::path::PathBuf| {
        if path.extension().is_none_or(|e| e != "log") {
            return;
        }
        let name = match (path.file_stem().and_then(|s| s.to_str()), path.parent().and_then(|p| p.file_name()).and_then(|p| p.to_str())) {
            (Some("arduplane"), Some(dir)) if dir.starts_with("opponent-") => format!("ArduPlane (computer pilot {})", dir.trim_start_matches("opponent-")),
            (Some("arduplane"), _) => "ArduPlane".to_string(),
            (Some(stem), _) => stem.to_string(),
            _ => return,
        };
        out.push(LogFile { name, path });
    };
    let mut dirs = vec![logs.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else {
                visit(p);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// How many acknowledgements are kept.
pub const ACKS_KEPT: usize = 12;

/// What the autopilot answered to a command.
#[derive(Clone, Debug)]
pub struct Ack {
    pub at: Instant,
    pub command: String,
    pub result: String,
}

/// The parts of the autopilot's state whose changes are worth saying.
#[derive(Clone, Debug, PartialEq)]
pub struct Digest {
    armed: bool,
    mode: &'static str,
    gps_fix: u8,
    ekf_flags: u16,
    status: u8,
    unhealthy: Vec<&'static str>,
}

impl Digest {
    pub fn of(v: &Vehicle) -> Digest {
        Digest { armed: v.armed, mode: mode_name(v.custom_mode), gps_fix: v.gps_fix, ekf_flags: v.ekf_flags, status: v.system_status, unhealthy: unhealthy_sensors(v) }
    }
}

/// What changed from `before` to `after`, one thing to a line.
pub fn changes(before: &Digest, after: &Digest) -> Vec<String> {
    let mut out = vec![];
    if before.armed != after.armed {
        out.push(if after.armed { "armed".to_string() } else { "disarmed".to_string() });
    }
    if before.mode != after.mode {
        out.push(format!("mode {} -> {}", before.mode, after.mode));
    }
    if before.gps_fix != after.gps_fix {
        out.push(format!("GPS fix {} -> {} ({})", before.gps_fix, after.gps_fix, fix_name(after.gps_fix)));
    }
    if before.ekf_flags != after.ekf_flags {
        out.push(format!("EKF flags 0x{:03x} -> 0x{:03x}", before.ekf_flags, after.ekf_flags));
    }
    if before.status != after.status {
        out.push(format!("MAV_STATE {} -> {}", before.status, after.status));
    }
    for s in &after.unhealthy {
        if !before.unhealthy.contains(s) {
            out.push(format!("{s} unhealthy"));
        }
    }
    for s in &before.unhealthy {
        if !after.unhealthy.contains(s) {
            out.push(format!("{s} healthy again"));
        }
    }
    out
}

/// An acknowledgement as a log line, and how loud it is. `progress` is a percent (0 to 100) for
/// IN_PROGRESS; `param2` is the autopilot's extra reason for a refusal.
pub fn describe_ack(command: &str, result: &str, progress: u8, param2: i32) -> (Level, String) {
    let mut text = format!("{command}: {result}");
    if result == "IN_PROGRESS" && progress > 0 {
        text.push_str(&format!(" {progress}%"));
    }
    if param2 != 0 {
        text.push_str(&format!(" (result_param2 {param2})"));
    }
    let level = match result {
        "ACCEPTED" if command == "REQUEST_MESSAGE" => Level::Trace,
        "ACCEPTED" | "IN_PROGRESS" => Level::Debug,
        _ => Level::Warn,
    };
    (level, text)
}

/// A command as it was sent: its name and its parameters up to the last that is not zero.
pub fn describe_command(name: &str, p: &[f32; 7]) -> String {
    let n = p.iter().rposition(|x| *x != 0.0).map_or(0, |i| i + 1);
    let mut s = name.to_string();
    for x in &p[..n] {
        s.push(' ');
        if x.fract() == 0.0 {
            s.push_str(&format!("{}", *x as i64));
        } else {
            s.push_str(&format!("{x}"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle::Vehicle;
    use std::time::{Duration, Instant};

    fn log() -> DebugLog {
        DebugLog::new(Instant::now())
    }

    /// Everything that happens goes in, in order and with its source, and none of it is collapsed
    /// the way the operator's message list collapses a repeated warning: a debug log that hid the
    /// fifth identical PreArm would hide how long it went on.
    #[test]
    fn every_event_is_kept_in_order_and_repeats_are_not_collapsed() {
        let mut l = log();
        l.record(Level::Info, Source::Link, "Link up");
        l.record(Level::Warn, Source::Fc, "PreArm: AHRS");
        l.record(Level::Warn, Source::Fc, "PreArm: AHRS");
        l.record(Level::Debug, Source::Ack, "ARM_DISARM: ACCEPTED");
        let all: Vec<_> = l.tail(10).iter().map(|e| (e.source, e.text.as_str())).collect();
        assert_eq!(all, vec![(Source::Link, "Link up"), (Source::Fc, "PreArm: AHRS"), (Source::Fc, "PreArm: AHRS"), (Source::Ack, "ARM_DISARM: ACCEPTED")]);
        assert_eq!(l.tail(2).len(), 2);
        assert_eq!(l.tail(2)[1].text, "ARM_DISARM: ACCEPTED", "the tail is the latest, oldest first");
    }

    #[test]
    fn the_log_keeps_the_latest_and_says_how_much_it_dropped() {
        let mut l = DebugLog::with_capacity(Instant::now(), 5);
        for i in 0..12 {
            l.record(Level::Info, Source::State, format!("event {i}"));
        }
        assert_eq!(l.len(), 5);
        assert_eq!(l.dropped(), 7);
        assert_eq!(l.tail(5)[0].text, "event 7");
        assert_eq!(l.tail(1)[0].text, "event 11");
    }

    /// One line per event, readable in a terminal and by the launcher, which reads the file
    /// the console writes: it takes a line back apart.
    #[test]
    fn a_line_reads_back_as_the_event_it_was() {
        let e = DebugEvent { t: 12.345, level: Level::Warn, source: Source::Ack, text: "ARM_DISARM: DENIED (it said: no)".into() };
        let line = format_line(&e);
        assert_eq!(line, "   12.345 WARN  ack    ARM_DISARM: DENIED (it said: no)");
        assert_eq!(parse_line(&line), Some(e));
        assert_eq!(parse_line("not a line"), None);
        assert_eq!(parse_line(""), None);
        // A line from the log of another program (ArduPlane's) is shown as it is, not parsed.
        assert_eq!(parse_line("Starting ArduPlane"), None);
    }

    #[test]
    fn levels_order_from_noise_to_trouble() {
        assert!(Level::Trace < Level::Debug && Level::Debug < Level::Info && Level::Info < Level::Warn && Level::Warn < Level::Error);
        // MAV_SEVERITY: 0 emergency to 7 debug.
        assert_eq!(Level::from_severity(2), Level::Error);
        assert_eq!(Level::from_severity(4), Level::Warn);
        assert_eq!(Level::from_severity(6), Level::Info);
        assert_eq!(Level::from_severity(7), Level::Debug);
    }

    /// With a file, each event is written as it happens, so the file is the log of a console that
    /// crashed too; the file starts anew each run.
    #[test]
    fn a_log_with_a_file_writes_each_event_as_it_happens() {
        let dir = std::env::temp_dir().join(format!("sim001-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("logs/debug/console.log");
        let mut l = log();
        l.to_file(&path).unwrap();
        l.record(Level::Info, Source::Link, "Link up: tcpout:127.0.0.1:5750");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Link up: tcpout:127.0.0.1:5750"), "{text}");
        l.record(Level::Error, Source::Fc, "Crash");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().filter(|l| parse_line(l).is_some()).count(), 2);
        let mut again = log();
        again.to_file(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().filter(|l| parse_line(l).is_some()).count(), 0, "a new run starts a new file");
    }

    // ---- ping ----

    /// A ping is answered by the autopilot echoing what it was sent: the round trip is the time
    /// between the two. One that is not answered in time is lost, and loss is a share of those
    /// that had time to be answered.
    #[test]
    fn a_ping_measures_the_round_trip_and_a_missing_reply_is_a_loss() {
        let mut p = PingStats::default();
        let t0 = Instant::now();
        p.sent(1, t0);
        assert_eq!(p.reply(1, t0 + Duration::from_millis(12)).map(|r| r.round()), Some(12.0));
        assert_eq!(p.reply(1, t0 + Duration::from_millis(13)), None, "answered twice");
        assert_eq!(p.reply(99, t0), None, "never sent");

        p.sent(2, t0 + Duration::from_secs(2));
        let lost = p.expire(t0 + Duration::from_secs(5), Duration::from_secs(2));
        assert_eq!(lost, 1);
        assert_eq!((p.count_sent(), p.count_received(), p.count_lost()), (2, 1, 1));
        assert!((p.loss_percent() - 50.0).abs() < 1e-3);
        // A reply to one that was given up on is still not a reply.
        assert_eq!(p.reply(2, t0 + Duration::from_secs(6)), None);
    }

    #[test]
    fn the_ping_summary_says_latency_and_loss() {
        let mut p = PingStats::default();
        assert_eq!(p.summary(), "no ping yet");
        let t0 = Instant::now();
        for (i, ms) in [10u64, 20, 30].into_iter().enumerate() {
            p.sent(i as i64, t0);
            p.reply(i as i64, t0 + Duration::from_millis(ms));
        }
        let s = p.summary();
        assert!(s.contains("20.0 ms avg") && s.contains("10.0") && s.contains("30.0") && s.contains("0% lost"), "{s}");
        assert_eq!(p.last_ms().map(|m| m.round()), Some(30.0));
    }

    #[test]
    fn a_ping_that_is_waiting_is_not_counted_lost_yet() {
        let mut p = PingStats::default();
        let t0 = Instant::now();
        p.sent(1, t0);
        assert_eq!(p.expire(t0 + Duration::from_millis(500), Duration::from_secs(2)), 0);
        assert_eq!(p.count_lost(), 0);
        assert_eq!(p.loss_percent(), 0.0);
        assert_eq!(p.waiting(), 1);
    }

    // ---- what the autopilot's state says ----

    fn vehicle() -> Vehicle {
        let mut v = Vehicle::new("tcpout:127.0.0.1:5750");
        v.link.connected = true;
        v.have_heartbeat = true;
        v.link.last_heartbeat = Some(Instant::now());
        v
    }

    /// The sensors that are there and enabled but not healthy, by name: the first thing to know
    /// when a plane will not arm.
    #[test]
    fn unhealthy_sensors_are_named() {
        let mut v = vehicle();
        // gyro, accel, mag, GPS, AHRS and the pre-arm check are present and enabled; the GPS and AHRS are not healthy.
        let bits = 1 | 2 | 4 | 32 | 0x20_0000 | 0x1000_0000;
        v.sensors_present = bits;
        v.sensors_enabled = bits;
        v.sensors_health = bits & !(32 | 0x20_0000);
        assert_eq!(unhealthy_sensors(&v), vec!["GPS", "AHRS"]);
        v.sensors_health = bits;
        assert!(unhealthy_sensors(&v).is_empty());
        // One that is not present is not reported, however unhealthy.
        v.sensors_present &= !4;
        v.sensors_health = 0;
        assert!(!unhealthy_sensors(&v).contains(&"mag"));
    }

    #[test]
    fn the_report_says_what_the_autopilot_is_doing() {
        let mut v = vehicle();
        v.custom_mode = 18; // QHOVER
        v.armed = false;
        v.gps_fix = 3;
        v.sats = 10;
        let report = state_report(&v);
        let get = |k: &str| report.iter().find(|(key, _)| key == k).map(|(_, val)| val.clone()).unwrap_or_else(|| panic!("no {k} in {report:?}"));
        assert!(get("link").contains("connected"), "{}", get("link"));
        assert!(get("heartbeat").contains("s ago"), "{}", get("heartbeat"));
        assert_eq!(get("mode"), "QHOVER");
        assert_eq!(get("armed"), "no");
        assert!(get("gps").contains("3D") && get("gps").contains("10"), "{}", get("gps"));
        assert!(get("ping").contains("no ping"), "{}", get("ping"));
        assert!(get("ready to arm").contains("no"), "{}", get("ready to arm"));
    }

    /// The reason it won't arm, and how long ago it said so, are in the report.
    #[test]
    fn the_report_says_why_it_will_not_arm() {
        let mut v = vehicle();
        v.prearm = Some((Instant::now() - Duration::from_secs(7), "PreArm: AHRS: not using configured AHRS type".into()));
        let report = state_report(&v);
        let why = &report.iter().find(|(k, _)| k == "pre-arm").expect("a pre-arm line").1;
        assert!(why.contains("AHRS: not using configured AHRS type") && why.contains("7 s ago"), "{why}");
    }

    #[test]
    fn a_vehicle_with_no_heartbeat_says_so() {
        let mut v = Vehicle::new("tcpout:127.0.0.1:5750");
        let report = state_report(&v);
        let get = |k: &str| report.iter().find(|(key, _)| key == k).unwrap().1.clone();
        assert!(get("heartbeat").contains("none"), "{}", get("heartbeat"));
        assert_eq!(get("mode"), "----");
        v.link.last_heartbeat = Some(Instant::now() - Duration::from_secs(9));
        v.have_heartbeat = true;
        assert!(state_report(&v).iter().find(|(k, _)| k == "heartbeat").unwrap().1.contains("LOST"));
    }

    /// What changed in the autopilot's state between two looks, as events: a sensor going
    /// unhealthy, the EKF flags, the GPS fix, the mode, arming. Nothing when nothing changed.
    #[test]
    fn what_changed_is_said_once() {
        let mut v = vehicle();
        let before = Digest::of(&v);
        assert!(changes(&before, &Digest::of(&v)).is_empty());
        v.gps_fix = 3;
        v.sats = 11;
        v.ekf_flags = 0x1ff;
        v.armed = true;
        v.custom_mode = 18;
        let after = Digest::of(&v);
        let said = changes(&before, &after).join(" | ");
        assert!(said.contains("GPS fix 0 -> 3 (3D)"), "{said}");
        assert!(said.contains("EKF flags 0x000 -> 0x1ff"), "{said}");
        assert!(said.contains("armed"), "{said}");
        assert!(said.contains("mode") && said.contains("QHOVER"), "{said}");
        // A sensor that goes unhealthy is named.
        let mut w = vehicle();
        w.sensors_present = 32;
        w.sensors_enabled = 32;
        w.sensors_health = 32;
        let healthy = Digest::of(&w);
        w.sensors_health = 0;
        assert!(changes(&healthy, &Digest::of(&w)).join("|").contains("GPS unhealthy"));
        // The satellite count wandering is not news; losing the fix is.
        let mut x = vehicle();
        x.gps_fix = 3;
        let a = Digest::of(&x);
        x.sats = 12;
        assert!(changes(&a, &Digest::of(&x)).is_empty(), "satellites come and go");
    }

    /// A command is described the way ArduPilot is told it, with its parameters, so that an
    /// acknowledgement that never comes can be matched to the command that never got one.
    #[test]
    fn a_command_is_described_with_its_parameters() {
        assert_eq!(describe_command("COMPONENT_ARM_DISARM", &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]), "COMPONENT_ARM_DISARM 1");
        assert_eq!(describe_command("DO_SET_MODE", &[1.0, 17.0, 0.0, 0.0, 0.0, 0.0, 0.0]), "DO_SET_MODE 1 17");
        assert_eq!(describe_command("NAV_TAKEOFF", &[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 20.0]), "NAV_TAKEOFF 0 0 0 0 0 0 20");
        assert_eq!(describe_command("X", &[0.0; 7]), "X");
    }

    // ---- the logs on disk ----

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sim001-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The end of a file, for a log that is too long to read whole: its last lines, and none of
    /// a line that was cut by starting in the middle of it.
    #[test]
    fn the_end_of_a_file_is_its_last_whole_lines() {
        let d = temp_dir("tail");
        let path = d.join("a.log");
        let text: String = (0..5000).map(|i| format!("line number {i}\n")).collect();
        std::fs::write(&path, &text).unwrap();
        assert_eq!(tail_lines(&path, 3), vec!["line number 4997", "line number 4998", "line number 4999"]);
        let many = tail_lines(&path, 100_000);
        assert!(many.len() < 5000, "it reads only the end ({} lines)", many.len());
        assert!(many.iter().all(|l| l.starts_with("line number ")), "a cut line: {:?}", &many[..2]);
        std::fs::write(d.join("short.log"), "one\ntwo").unwrap();
        assert_eq!(tail_lines(&d.join("short.log"), 5), vec!["one", "two"]);
        assert!(tail_lines(&d.join("missing.log"), 5).is_empty());
    }

    /// The logs a run leaves, wherever it keeps them under `logs/`: the console's, ArduPlane's
    /// and each computer pilot's, newest first, with what they are.
    #[test]
    fn the_logs_of_a_run_are_found_and_named() {
        let d = temp_dir("logs");
        for (p, text) in [("debug/console.log", "x"), ("sitl/arduplane.log", "y"), ("sitl/opponent-2/arduplane.log", "z"), ("sitl/notes.txt", "n"), ("sitl/00000001.BIN", "b")] {
            let path = d.join("logs").join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let found = list_logs(&d.join("logs"));
        let names: Vec<_> = found.iter().map(|l| l.name.as_str()).collect();
        assert!(names.contains(&"console") && names.contains(&"ArduPlane") && names.contains(&"ArduPlane (computer pilot 2)"), "{names:?}");
        assert_eq!(found.len(), 3, "only .log files: {names:?}");
        assert!(list_logs(&d.join("nowhere")).is_empty());
    }

    /// How an acknowledgement reads in the log: a refusal is a warning, a routine acceptance is noise.
    #[test]
    fn an_acknowledgement_reads_as_what_it_means() {
        let (level, text) = describe_ack("COMPONENT_ARM_DISARM", "DENIED", 0, 0);
        assert_eq!((level, text.as_str()), (Level::Warn, "COMPONENT_ARM_DISARM: DENIED"));
        assert_eq!(describe_ack("REQUEST_MESSAGE", "ACCEPTED", 0, 0).0, Level::Trace, "routine");
        assert_eq!(describe_ack("DO_SET_MODE", "ACCEPTED", 0, 0).0, Level::Debug);
        let (level, text) = describe_ack("NAV_TAKEOFF", "IN_PROGRESS", 40, 0);
        assert_eq!(level, Level::Debug);
        assert!(text.contains("IN_PROGRESS") && text.contains("40%"), "{text}");
        let (level, text) = describe_ack("NAV_TAKEOFF", "FAILED", 0, 3);
        assert_eq!(level, Level::Warn);
        assert!(text.contains("result_param2 3"), "{text}");
    }

    /// What was logged for the operator is in the debug log too, with its source.
    #[test]
    fn the_operators_messages_are_in_the_debug_log_too() {
        let mut v = vehicle();
        v.push_log("FC", 4, "PreArm: AHRS");
        v.push_log("FC", 4, "PreArm: AHRS");
        v.push_log("GCS", 6, "Mode QHOVER");
        let all: Vec<_> = v.debug.tail(10).iter().map(|e| (e.source, e.level, e.text.clone())).collect();
        assert_eq!(all.len(), 3, "the repeat is kept here: {all:?}");
        assert_eq!(all[0], (Source::Fc, Level::Warn, "PreArm: AHRS".to_string()));
        assert_eq!(all[2].0, Source::Gcs);
    }

    /// The latest acknowledgements are kept, so the report can say what the autopilot last said to
    /// what it was last told.
    #[test]
    fn the_last_acknowledgements_are_kept() {
        let mut v = vehicle();
        for i in 0..20 {
            v.record_ack("ARM_DISARM", if i == 19 { "DENIED" } else { "ACCEPTED" });
        }
        assert_eq!(v.acks.len(), ACKS_KEPT);
        let report = state_report(&v);
        let last = &report.iter().find(|(k, _)| k == "last ack").expect("last ack").1;
        assert!(last.contains("ARM_DISARM") && last.contains("DENIED"), "{last}");
    }
}
