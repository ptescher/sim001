//! The TX15 on a mock line, with the flight computer at the other end.

use aviation_computer::{FlightComputer, Vehicle};
use aviation_line::{Pilot, Tx15, mock_pair};

const STEP: f64 = 0.005;

/// A flight computer on the far end of a mock line, and a radio on the near one.
struct Bench {
    radio: Tx15,
    far: aviation_line::Mock,
    fc: FlightComputer,
    now: f64,
}

impl Bench {
    fn new() -> Bench {
        let (near, far) = mock_pair();
        Bench { radio: Tx15::new(near), far, fc: FlightComputer::new(), now: 0.0 }
    }

    /// Run for `secs`, the flight computer reading what the radio sent and answering with the
    /// state of `vehicle`.
    fn run(&mut self, secs: f64, vehicle: &Vehicle) {
        let end = self.now + secs;
        while self.now < end {
            self.now += STEP;
            self.radio.step(self.now);
            let up = self.far.take();
            self.fc.from_line(&up, self.now);
            let down = self.fc.to_line(vehicle, self.now);
            if !down.is_empty() {
                use aviation_line::Wire;
                let mut far = self.far.clone();
                far.write_all(&down);
            }
        }
    }
}

fn plane() -> Vehicle {
    Vehicle { mode: "QHOVER".into(), volts: 15.2, percent: 80, ..Default::default() }
}

#[test]
fn the_sticks_and_switches_arrive_as_channels() {
    let mut bench = Bench::new();
    bench.radio.fly(Pilot { roll: 0.5, throttle: 0.25, arm: true, mode: 5, ..Pilot::AT_REST });
    bench.run(0.5, &plane());
    let ch = bench.fc.channels(bench.now);
    assert_eq!(ch[0], 1750, "roll");
    assert_eq!(ch[1], 1500, "pitch");
    assert_eq!(ch[2], 1250, "throttle");
    assert_eq!(ch[4], 1900, "arm");
    assert_eq!(ch[7], simulation_aircraft::rc_mode_pwm(5), "mode");
    assert!(bench.fc.link_up(bench.now));
}

#[test]
fn a_radio_at_rest_sends_the_sticks_centred_and_the_switches_out() {
    let ch = Pilot::AT_REST.channels();
    assert_eq!(&ch[..5], &[1500, 1500, 1000, 1500, 1100]);
    assert_eq!(ch[7], simulation_aircraft::rc_mode_pwm(0));
}

#[test]
fn an_unplugged_radio_says_nothing_and_the_flight_computer_fails_safe() {
    let mut bench = Bench::new();
    bench.run(0.5, &plane());
    assert!(bench.fc.link_up(bench.now));
    bench.radio.connect(false);
    bench.run(1.0, &plane());
    assert!(!bench.fc.link_up(bench.now));
    assert_eq!(bench.fc.channels(bench.now)[2], aviation_computer::FAILSAFE_THROTTLE);
}

#[test]
fn the_screen_shows_what_the_flight_computer_reports() {
    let mut bench = Bench::new();
    bench.run(1.5, &Vehicle { armed: true, ..plane() });
    let screen = bench.radio.screen();
    assert_eq!(screen.mode, "QHOVER");
    assert!(screen.armed);
    assert!((screen.volts - 15.2).abs() < 0.2, "{}", screen.volts);
    assert!(bench.radio.quality(bench.now) > 0);
}

#[test]
fn mavlink_goes_up_and_down() {
    let mut bench = Bench::new();
    bench.radio.send_mavlink(&[0xFD, 1, 2, 3]);
    bench.run(0.5, &plane());
    assert_eq!(bench.fc.take_for_autopilot(), vec![0xFD, 1, 2, 3]);
    // A flight computer with a heartbeat from the plane sends it down the line.
    let heartbeat = communication_free_heartbeat();
    bench.fc.from_autopilot(&heartbeat);
    bench.run(1.0, &plane());
    assert!(!bench.radio.take_mavlink().is_empty());
}

/// Bytes ArduPilot's heartbeat would be; the flight computer only forwards them.
fn communication_free_heartbeat() -> Vec<u8> {
    vec![0xFD, 0x09, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0, 0, 0, 0, 1, 3, 0, 0, 3, 0, 0]
}

#[test]
fn a_line_that_hangs_up_is_noticed() {
    let (near, far) = mock_pair();
    let mut radio = Tx15::new(near);
    radio.step(0.1);
    assert!(radio.alive());
    far.hang_up();
    radio.step(0.2);
    assert!(!radio.alive());
}
