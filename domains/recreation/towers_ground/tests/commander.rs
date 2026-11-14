//! A computer commander is a seat like a person's: against a fake game server that speaks only the
//! seat protocol (no SITL), it takes the seat as `Join::Compute`, and from the umpire's word and
//! what its sites report it gives orders; it goes on from the orders the drones have, and it leaves
//! when it is relieved.

use recreation_towers::seat::{self, Join, ToSeat, ToServer};
use recreation_towers::{Game, Order, Rules, Team};
use recreation_towers_ground::commander::{self, Brain};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn a_computer_commander_takes_a_seat_gives_orders_from_the_sites_and_leaves_when_relieved() {
    let rules = Rules::default();
    // Air ports for the team's drones that accept and say nothing: no telemetry at all.
    let air: Vec<TcpListener> = (0..rules.drones).map(|_| TcpListener::bind("127.0.0.1:0").unwrap()).collect();
    let ports: Vec<u16> = air.iter().map(|l| l.local_addr().unwrap().port()).collect();
    let held: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
    for l in air {
        let held = held.clone();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                held.lock().unwrap().push(s);
            }
        });
    }
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = channel();
    let (a, s) = (addr.clone(), stop.clone());
    std::thread::spawn(move || {
        commander::run(&a, Team::Blue, Brain::Play, None, &s);
        done_tx.send(()).unwrap();
    });
    let (stream, _) = server.accept().unwrap();
    let mut out = stream.try_clone().unwrap();
    let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
    let ToServer::Join(join) = seat::decode(&lines.next().unwrap()).unwrap() else { panic!("not a join") };
    assert_eq!(join, Join::Compute(Team::Blue));
    let said = |out: &mut TcpStream, m: ToSeat| writeln!(out, "{}", seat::encode(&m)).unwrap();
    said(&mut out, ToSeat::Welcome { join, rules, air: ports });
    // The drones have orders already: a person was commanding them.
    said(&mut out, ToSeat::Orders(vec![(Some(Order::Hold), true); rules.drones]));
    let mut game = Game::new(rules);
    game.time = 5.0;
    let (orders_tx, orders_rx) = channel();
    let reader = std::thread::spawn(move || {
        for line in lines {
            if let Ok(ToServer::Order { drone, order, low, .. }) = seat::decode::<ToServer>(&line) {
                orders_tx.send((drone, order, low)).unwrap();
            }
        }
    });
    // Tell it the game until it has given every drone an order.
    let end = Instant::now() + Duration::from_secs(10);
    let mut given = std::collections::BTreeMap::new();
    while given.len() < rules.drones {
        assert!(Instant::now() < end, "it gave orders to {given:?}");
        game.time += 0.25;
        said(&mut out, ToSeat::Umpire(game.umpire()));
        said(&mut out, ToSeat::Sites(game.sites(Team::Blue)));
        while let Ok((drone, order, low)) = orders_rx.recv_timeout(Duration::from_millis(100)) {
            given.insert(drone, (order, low));
        }
    }
    assert!(given.keys().all(|&k| k < rules.drones));
    for (k, (_, low)) in &given {
        assert_eq!(*low, rules.class(*k) == recreation_towers::Class::Light, "drone {k} is flown low if it is light");
    }
    // Relieved by a person: it leaves, and does not come back.
    said(&mut out, ToSeat::Refused("relieved: a person took the seat".into()));
    drop(out);
    done_rx.recv_timeout(Duration::from_secs(5)).expect("the commander leaves a seat it is relieved of");
    stop.store(true, Ordering::SeqCst);
    let _ = reader;
}
