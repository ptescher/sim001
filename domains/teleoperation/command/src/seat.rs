//! The app's seat on the game server: a TCP connection on threads of its own. It keeps the last
//! map the server sent and sends the orders the commander gives. If the connection is lost, or
//! the server is not up yet, it tries again.

use recreation_towers::seat::{self, Join, ToSeat, ToServer};
use recreation_towers::{Advice, Order, Rules, Sites, TeamView, Umpire};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Connecting,
    Seated(Join),
    Refused(String),
}

#[derive(Debug)]
pub struct State {
    pub status: Status,
    pub rules: Option<Rules>,
    pub view: Option<TeamView>,
    /// What the server last said: the umpire's word, and what the team's towers and base report. The
    /// ground station makes `view` from them and from the telemetry of its drones (station.rs).
    pub umpire: Option<Umpire>,
    pub sites: Option<Sites>,
    /// How many maps of the team the server sent a seat that commands (it sends none).
    pub maps: usize,
    /// The ports of the air in front of the team's drones, on the server's host: the way to them.
    pub air: Vec<u16>,
    /// The orders given, by drone number in the team: the order and whether it is flown low.
    pub orders: Vec<(Option<Order>, bool)>,
    /// What the advisor said lately, and when (the last few).
    pub advice: Vec<(Advice, Instant)>,
}

/// How many pieces of advice the seat keeps.
const ADVICE_KEPT: usize = 3;

pub struct Seat {
    addr: String,
    join: Join,
    state: Arc<Mutex<State>>,
    orders: Mutex<Sender<ToServer>>,
    /// Each (re)connection's number: a connection that is not the newest stops.
    generation: Arc<AtomicU64>,
    /// The drones that the app's own ground has put in AGENT mode (station.rs).
    pub flown: AtomicUsize,
    /// How many position reports the app has heard from its drones.
    pub heard: AtomicUsize,
}

impl Seat {
    pub fn connect(addr: &str, join: Join) -> Arc<Seat> {
        let (tx, _) = channel();
        let seat = Arc::new(Seat {
            addr: addr.to_string(),
            join,
            state: Arc::new(Mutex::new(State {
                status: Status::Connecting,
                rules: None,
                view: None,
                umpire: None,
                sites: None,
                maps: 0,
                air: vec![],
                orders: vec![],
                advice: vec![],
            })),
            orders: Mutex::new(tx),
            generation: Arc::new(AtomicU64::new(0)),
            flown: AtomicUsize::new(0),
            heard: AtomicUsize::new(0),
        });
        seat.reconnect();
        seat
    }

    pub fn join(&self) -> Join {
        self.join
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    /// Sends an order for the team's drone `drone`, flown `low` (along the lanes) or high.
    pub fn order(&self, drone: usize, order: Order, low: bool) {
        // The app is the team's ground station: the order is its own to carry out (station.rs).
        {
            let mut s = self.state();
            if s.orders.len() <= drone {
                s.orders.resize(drone + 1, (None, false));
            }
            s.orders[drone] = (Some(order), low);
        }
        let _ = self.orders.lock().unwrap().send(ToServer::Order { drone, order, low, standing: None });
    }

    /// Stops connecting: the connection that is open ends with its server, and is not made
    /// again. (Before the server stops, so that it does not take the seat of the next game first.)
    pub fn hang_up(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Forgets the game and connects again (to a new game on the same server address).
    pub fn reconnect(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.state() = State {
            status: Status::Connecting,
            rules: None,
            view: None,
            umpire: None,
            sites: None,
            maps: 0,
            air: vec![],
            orders: vec![],
            advice: vec![],
        };
        let (tx, rx) = channel();
        *self.orders.lock().unwrap() = tx;
        let (addr, join, state, current) = (self.addr.clone(), self.join, self.state.clone(), self.generation.clone());
        std::thread::spawn(move || run(&addr, join, &state, rx, &current, generation));
    }
}

/// One connection, for as long as it is the newest: connects (again, if it is lost), takes the
/// seat, and reads the game. The orders go out on a thread of their own, to whichever stream is
/// open.
fn run(addr: &str, join: Join, state: &Mutex<State>, orders: Receiver<ToServer>, current: &AtomicU64, generation: u64) {
    let out: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
    let writer = out.clone();
    std::thread::spawn(move || {
        while let Ok(msg) = orders.recv() {
            if let Some(stream) = writer.lock().unwrap().as_mut() {
                let _ = writeln!(stream, "{}", seat::encode(&msg));
            }
        }
    });
    while current.load(Ordering::SeqCst) == generation {
        let Ok(stream) = TcpStream::connect(addr) else {
            std::thread::sleep(Duration::from_millis(300));
            continue;
        };
        let Ok(mut w) = stream.try_clone() else { continue };
        if writeln!(w, "{}", seat::encode(&ToServer::Join(join))).is_err() {
            continue;
        }
        *out.lock().unwrap() = Some(w);
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if current.load(Ordering::SeqCst) != generation {
                if let Some(s) = out.lock().unwrap().take() {
                    let _ = s.shutdown(std::net::Shutdown::Both);
                }
                return;
            }
            let Ok(msg) = seat::decode::<ToSeat>(&line) else { continue };
            let mut s = state.lock().unwrap();
            match msg {
                ToSeat::Welcome { join, rules, air } => {
                    s.status = Status::Seated(join);
                    s.rules = Some(rules);
                    s.air = air;
                }
                ToSeat::Refused(why) => s.status = Status::Refused(why),
                ToSeat::View(v) => {
                    if matches!(join, Join::Command(_)) {
                        s.maps += 1;
                    }
                    s.view = Some(v);
                }
                ToSeat::Umpire(u) => s.umpire = Some(u),
                ToSeat::Sites(sites) => s.sites = Some(sites),
                ToSeat::Truth(_) => {}
                ToSeat::Advice(a) => {
                    s.advice.push((a, Instant::now()));
                    let extra = s.advice.len().saturating_sub(ADVICE_KEPT);
                    s.advice.drain(..extra);
                }
            }
        }
        *out.lock().unwrap() = None;
        // Hung up, or a newer connection: the state is the newer one's.
        if current.load(Ordering::SeqCst) != generation {
            return;
        }
        // The seat is refused for good if the computer commands the team. If a commander holds it,
        // that is this app's own connection from before a new game: it lets go in a moment.
        let refused = match &state.lock().unwrap().status {
            Status::Refused(why) => Some(why.contains("has its commander")),
            _ => None,
        };
        match refused {
            Some(false) => return,
            Some(true) => {
                std::thread::sleep(Duration::from_millis(400));
                continue;
            }
            None => {}
        }
        state.lock().unwrap().status = Status::Connecting;
        std::thread::sleep(Duration::from_millis(300));
    }
}
