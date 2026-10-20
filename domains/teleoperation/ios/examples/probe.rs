//! `cargo run -p teleoperation_ios --example probe [ADDRESS]`: the station on the Mac, printing
//! what it sees once a second. For telling a link problem from an app problem.
use teleoperation_ios::Station;

fn main() {
    let address = std::env::args().nth(1).unwrap_or("tcpout:127.0.0.1:5760".into());
    let s = Station::start(&address);
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let v = s.vehicle.lock().unwrap();
        println!("{} | alive {} mode {} armed {} home {}", v.link.status, v.link.alive(), v.mode(), v.armed, v.home.is_some());
    }
}
