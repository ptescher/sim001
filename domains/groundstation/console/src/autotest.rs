//! `--autotest`: drives the GCS command path end to end against SITL, the way a pilot would
//! with the buttons. It uploads a circuit, reads it back, arms, flies AUTO, sends a GUIDED
//! goto, then RTL, and prints telemetry along the way. The UI stays live throughout.
//!
//! With the t1s_sim physics the AUTO takeoff waits for a hand launch: throw it in the sim
//! (throttle up or Space) when the log says "waiting for launch".

use crate::geo::Origin;
use crate::link::{Cmd, Shared};
use crate::vehicle::{Vehicle, Waypoint, WpKind, mode_number};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub fn spawn(vehicle: Shared, tx: Sender<Cmd>, origin: Origin) {
    std::thread::spawn(move || {
        let ok = run(&vehicle, &tx, origin);
        let verdict = if ok { "AUTOTEST PASS" } else { "AUTOTEST FAIL" };
        println!("{verdict}");
        vehicle.lock().unwrap().push_log("TEST", if ok { 6 } else { 3 }, verdict);
    });
}

fn say(vehicle: &Shared, s: String) {
    println!("[autotest] {s}");
    vehicle.lock().unwrap().push_log("TEST", 5, s);
}

fn wait(vehicle: &Shared, secs: u64, mut cond: impl FnMut(&Vehicle) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if cond(&vehicle.lock().unwrap()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn report(vehicle: &Shared, secs: u64, label: &str) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        std::thread::sleep(Duration::from_secs(3));
        let s = {
            let v = vehicle.lock().unwrap();
            format!(
                "{label}: {} {} alt {:5.1} IAS {:4.1} GS {:4.1} thr {:3}% wp {} dist {:4.0} batt {:.2} V",
                v.mode(),
                if v.armed { "ARMED" } else { "disarmed" },
                v.alt_rel,
                v.airspeed,
                v.groundspeed,
                v.throttle,
                v.mission_seq,
                v.wp_dist,
                v.batt_v
            )
        };
        say(vehicle, s);
    }
}

fn run(vehicle: &Shared, tx: &Sender<Cmd>, origin: Origin) -> bool {
    say(vehicle, "waiting for heartbeat, home and EKF position".into());
    if !wait(vehicle, 180, |v| v.link.alive() && v.home.is_some() && v.ekf_flags & 16 != 0) {
        say(vehicle, "timed out waiting for the vehicle".into());
        return false;
    }

    let home = vehicle.lock().unwrap().home.unwrap();
    let (hn, he) = origin.to_ne(home.0, home.1);
    let wp = |kind, n: f32, e: f32, alt| {
        let (lat, lon) = origin.to_ll(hn + n, he + e);
        Waypoint { kind, lat, lon, alt }
    };
    let plan = vec![
        Waypoint { kind: WpKind::Takeoff, lat: home.0, lon: home.1, alt: 40.0 },
        wp(WpKind::Waypoint, 260.0, -120.0, 50.0),
        wp(WpKind::Waypoint, 260.0, 160.0, 50.0),
        wp(WpKind::Waypoint, -120.0, 160.0, 50.0),
        wp(WpKind::Waypoint, -120.0, -120.0, 50.0),
        wp(WpKind::LoiterUnlim, 80.0, 20.0, 60.0),
    ];
    let n = plan.len();
    say(vehicle, format!("uploading {n}-item circuit"));
    let _ = tx.send(Cmd::Upload(plan));
    if !wait(vehicle, 20, |v| v.mission_status.ends_with("on vehicle") && v.mission.len() == n + 1) {
        say(vehicle, format!("upload not accepted: {}", vehicle.lock().unwrap().mission_status));
        return false;
    }
    vehicle.lock().unwrap().mission.clear();
    let _ = tx.send(Cmd::Download);
    if !wait(vehicle, 20, |v| v.mission.len() == n + 1 && v.mission_status.ends_with("on vehicle")) {
        say(vehicle, format!("read-back failed: {}", vehicle.lock().unwrap().mission_status));
        return false;
    }
    say(vehicle, format!("mission read back: {} items + home", n));

    let _ = tx.send(Cmd::SetMode(mode_number("AUTO").unwrap()));
    if !wait(vehicle, 10, |v| v.mode() == "AUTO") {
        say(vehicle, "vehicle did not enter AUTO".into());
        return false;
    }
    for attempt in 1..=12 {
        let _ = tx.send(Cmd::Arm(true));
        if wait(vehicle, 5, |v| v.armed) {
            break;
        }
        let pre = vehicle.lock().unwrap().prearm.as_ref().map(|p| p.1.clone()).unwrap_or_default();
        say(vehicle, format!("arm attempt {attempt} refused {pre}"));
        if attempt == 12 {
            return false;
        }
    }
    say(vehicle, "armed in AUTO: waiting for launch / takeoff".into());
    if !wait(vehicle, 20, |v| v.alt_rel > 3.0 || v.groundspeed > 6.0) {
        // The t1s_sim hand launch needs a throw, which AUTO waits for. Throw it the way the
        // sim does for a pilot: hold throttle up in FBWA (RC override), then hand back to AUTO.
        say(vehicle, "no takeoff: launching in FBWA with RC throttle override".into());
        let _ = tx.send(Cmd::SetMode(mode_number("FBWA").unwrap()));
        let end = Instant::now() + Duration::from_secs(40);
        // Throttle full, a little up elevator; 0 leaves the other channels to the radio.
        while Instant::now() < end && vehicle.lock().unwrap().alt_rel < 25.0 {
            let _ = tx.send(Cmd::RcOverride([1500, 1650, 2000, 1500, 0, 0, 0, 0]));
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = tx.send(Cmd::RcOverride([0; 8]));
        let _ = tx.send(Cmd::SetMode(mode_number("AUTO").unwrap()));
        let _ = tx.send(Cmd::SetCurrentWp(2));
    }
    if !wait(vehicle, 120, |v| v.alt_rel > 20.0) {
        say(vehicle, "never climbed through 20 m".into());
        return false;
    }
    report(vehicle, 60, "auto");
    let reached_wp2 = vehicle.lock().unwrap().mission_seq >= 2;

    let (lat, lon) = origin.to_ll(hn + 350.0, he + 300.0);
    say(vehicle, "GUIDED goto 350 m N, 300 m E at 60 m".into());
    let _ = tx.send(Cmd::Goto { lat, lon, alt: 60.0 });
    if !wait(vehicle, 10, |v| v.mode() == "GUIDED") {
        say(vehicle, "goto did not switch to GUIDED".into());
        return false;
    }
    let target = origin.to_ne(lat, lon);
    let closest = {
        let mut best = f32::MAX;
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end {
            std::thread::sleep(Duration::from_secs(1));
            let v = vehicle.lock().unwrap();
            let (n, e) = origin.to_ne(v.lat, v.lon);
            best = best.min(((n - target.0).powi(2) + (e - target.1).powi(2)).sqrt());
        }
        best
    };
    say(vehicle, format!("closest approach to goto point {closest:.0} m"));

    let _ = tx.send(Cmd::SetMode(mode_number("RTL").unwrap()));
    report(vehicle, 30, "rtl");
    let home_dist = {
        let v = vehicle.lock().unwrap();
        let (n, e) = origin.to_ne(v.lat, v.lon);
        ((n - hn).powi(2) + (e - he).powi(2)).sqrt()
    };
    say(vehicle, format!("RTL: {home_dist:.0} m from home"));
    reached_wp2 && closest < 90.0 && home_dist < 200.0
}
