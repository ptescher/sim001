//! The seats, over TCP (`recreation_towers::seat`). A team's seat that commands is one seat, for a
//! person (`Join::Command`) or a computer (`Join::Compute`), held by one client at a time; any number
//! may watch a team or the whole game. Each gets what it may know, from the same moment of the game,
//! four times a second.
//!
//! A seat changes hands during a game. A person who joins a team that a computer commands takes the
//! seat: the computer is told it is relieved and its connection is closed. When a person's client
//! leaves, the seat is free; if it is still free after a grace time, the server has a computer
//! commander take it (`Free::take`). A computer commander is a seat client like any other: the server
//! does not decide or send a plan for any team.

use crate::world::World;
use recreation_towers::seat::{self, Join, ToSeat, ToServer, Truth};
use recreation_towers::{Advice, Rules, Team};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Client {
    id: u64,
    join: Join,
    stream: TcpStream,
}

/// What the server does when a person's client leaves a seat.
#[derive(Clone)]
pub struct Free {
    /// How long a seat may stay free before a computer takes it (a client that comes back at once, as
    /// the app does after a lost connection, keeps it).
    pub grace: Duration,
    /// Has a computer commander take the seat of the team.
    pub take: Arc<dyn Fn(Team) + Send + Sync>,
}

pub struct Seats {
    clients: Arc<Mutex<Vec<Client>>>,
    /// The teams a person commands: GO waits for them.
    people: [bool; 2],
    port: u16,
}

/// The ports of the air in front of the drones, by drone number in the game.
pub type AirPorts = Arc<Vec<u16>>;

/// Everything a seat's thread needs.
#[derive(Clone)]
struct Shared {
    clients: Arc<Mutex<Vec<Client>>>,
    world: Arc<Mutex<World>>,
    air: AirPorts,
    free: Free,
    next_id: Arc<AtomicU64>,
}

impl Seats {
    /// Listens on `port` (0: any). `people[team]`: GO waits for a person to take that team's seat.
    pub fn listen(
        port: u16,
        world: Arc<Mutex<World>>,
        people: [bool; 2],
        air: AirPorts,
        free: Free,
    ) -> std::io::Result<Seats> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(vec![]));
        let shared = Shared { clients: clients.clone(), world, air, free, next_id: Arc::new(AtomicU64::new(0)) };
        std::thread::Builder::new().name("seats".into()).spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let shared = shared.clone();
                std::thread::spawn(move || seat(stream, shared));
            }
        })?;
        Ok(Seats { clients, people, port })
    }

    /// The port the seats are on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether every team a person commands has its person.
    pub fn people_seated(&self) -> bool {
        let clients = self.clients.lock().unwrap();
        Team::BOTH
            .iter()
            .filter(|t| self.people[t.index()])
            .all(|&t| clients.iter().any(|c| c.join == Join::Command(t)))
    }

    /// Tells the seat of a person that commands `team` what its advisor has to say.
    pub fn advise(&self, team: Team, advice: &[Advice]) {
        let lines: Vec<String> = advice.iter().map(|a| seat::encode(&ToSeat::Advice(a.clone()))).collect();
        self.clients.lock().unwrap().retain_mut(|c| {
            c.join != Join::Command(team) || lines.iter().all(|line| writeln!(c.stream, "{line}").is_ok())
        });
    }

    /// Sends every seat what it may know of the game now; drops the seats that have gone.
    pub fn send(&self, world: &World) {
        let game = &world.game;
        // A seat that watches a team gets its whole map. One that commands, a person's or a computer's,
        // is the team's ground station and makes its own: it gets the umpire's word and what the team's
        // towers and base report.
        let views = [game.view(Team::Blue), game.view(Team::Red)].map(|mut v| {
            for d in &mut v.drones {
                let i = game.drone(v.team, d.k);
                (d.order, d.low) = world.told[i];
            }
            seat::encode(&ToSeat::View(v))
        });
        let umpire = seat::encode(&ToSeat::Umpire(game.umpire()));
        let sites = [Team::Blue, Team::Red].map(|t| seat::encode(&ToSeat::Sites(game.sites(t))));
        let mut truth = Truth::of(game);
        truth.attitudes = world.planes.iter().map(|p| p.rot.to_array()).collect();
        let truth = seat::encode(&ToSeat::Truth(truth));
        self.clients.lock().unwrap().retain_mut(|c| match c.join {
            Join::Command(t) | Join::Compute(t) => {
                writeln!(c.stream, "{umpire}").is_ok() && writeln!(c.stream, "{}", sites[t.index()]).is_ok()
            }
            Join::Watch(t) => writeln!(c.stream, "{}", views[t.index()]).is_ok(),
            Join::Spectate => writeln!(c.stream, "{truth}").is_ok(),
        });
    }
}

/// One seat's connection: what it is, then its orders.
fn seat(stream: TcpStream, shared: Shared) {
    let Shared { clients, world, air, free, next_id } = shared;
    let Ok(mut out) = stream.try_clone() else { return };
    let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
    let Some(join) = lines.next().and_then(|l| seat::decode::<ToServer>(&l).ok()) else {
        let _ = writeln!(out, "{}", seat::encode(&ToSeat::Refused("say what the seat is first: Join".into())));
        return;
    };
    let ToServer::Join(join) = join else { return };
    let (rules, told): (Rules, Vec<_>) = {
        let w = world.lock().unwrap();
        (w.game.rules, w.told.clone())
    };
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    {
        let mut clients = clients.lock().unwrap();
        if let Some(team) = join.commands() {
            let holder = clients.iter().position(|c| c.join.commands() == Some(team));
            match (join, holder.map(|h| clients[h].join)) {
                // A person takes the seat from a computer: the computer is relieved.
                (Join::Command(_), Some(Join::Compute(_))) => {
                    let old = clients.remove(holder.expect("a holder"));
                    let mut s = &old.stream;
                    let _ =
                        writeln!(s, "{}", seat::encode(&ToSeat::Refused("relieved: a person took the seat".into())));
                    let _ = old.stream.shutdown(Shutdown::Both);
                }
                (_, Some(_)) => {
                    let why = format!("{team:?} has its commander");
                    let _ = writeln!(out, "{}", seat::encode(&ToSeat::Refused(why)));
                    return;
                }
                (_, None) => {}
            }
        }
        let Ok(stream) = out.try_clone() else { return };
        // A seat that commands is its team's ground station: it gets the way to its drones, and the
        // orders they have.
        let ports = match join.commands() {
            Some(t) => air[t.index() * rules.drones..(t.index() + 1) * rules.drones].to_vec(),
            None => vec![],
        };
        if writeln!(out, "{}", seat::encode(&ToSeat::Welcome { join, rules, air: ports })).is_err() {
            return;
        }
        if let Some(t) = join.commands() {
            let orders: Vec<_> = (0..rules.drones).map(|k| told[t.index() * rules.drones + k]).collect();
            if writeln!(out, "{}", seat::encode(&ToSeat::Orders(orders))).is_err() {
                return;
            }
        }
        clients.push(Client { id, join, stream });
    }
    let Some(team) = join.commands() else { return };
    for line in lines {
        if let Ok(ToServer::Order { drone, order, low, standing, queue }) = seat::decode::<ToServer>(&line)
            && drone < rules.drones
        {
            let mut w = world.lock().unwrap();
            let i = w.game.drone(team, drone);
            // A drone out of contact does not hear it.
            w.command_queue(i, order, low, recreation_towers::queue_of(standing, queue));
        }
    }
    // The commander has gone: its seat is free again, unless it was relieved (it is not ours then).
    let was_ours = {
        let mut clients = clients.lock().unwrap();
        let n = clients.len();
        clients.retain(|c| c.id != id);
        clients.len() < n
    };
    if was_ours && matches!(join, Join::Command(_)) {
        std::thread::sleep(free.grace);
        let still_free = !clients.lock().unwrap().iter().any(|c| c.join.commands() == Some(team));
        let started = world.lock().unwrap().started();
        if still_free && started {
            (free.take)(team);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::{Order, Phase};
    use recreation_towers_ground::map;
    use std::collections::BTreeSet;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    struct Client {
        got: Arc<Mutex<Vec<ToSeat>>>,
        out: TcpStream,
    }

    impl Client {
        fn join(port: u16, join: Join) -> Client {
            let out = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut w = out.try_clone().unwrap();
            writeln!(w, "{}", seat::encode(&ToServer::Join(join))).unwrap();
            let got = Arc::new(Mutex::new(vec![]));
            let g = got.clone();
            let r = out.try_clone().unwrap();
            std::thread::spawn(move || {
                for line in BufReader::new(r).lines().map_while(Result::ok) {
                    if let Ok(m) = seat::decode::<ToSeat>(&line) {
                        g.lock().unwrap().push(m);
                    }
                }
            });
            Client { got, out }
        }

        fn wait(&self, what: &str, f: impl Fn(&[ToSeat]) -> bool) {
            let end = Instant::now() + Duration::from_secs(5);
            while !f(&self.got.lock().unwrap()) {
                assert!(Instant::now() < end, "timed out waiting for {what}: {:?}", self.got.lock().unwrap());
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        fn kinds(&self) -> BTreeSet<&'static str> {
            self.got
                .lock()
                .unwrap()
                .iter()
                .map(|m| match m {
                    ToSeat::Welcome { .. } => "Welcome",
                    ToSeat::Refused(_) => "Refused",
                    ToSeat::View(_) => "View",
                    ToSeat::Umpire(_) => "Umpire",
                    ToSeat::Sites(_) => "Sites",
                    ToSeat::Orders(_) => "Orders",
                    ToSeat::Truth(_) => "Truth",
                    ToSeat::Advice(_) => "Advice",
                })
                .collect()
        }

        fn leave(self) {
            let _ = self.out.shutdown(Shutdown::Both);
        }
    }

    fn server(people: [bool; 2], grace_ms: u64) -> (Seats, Arc<Mutex<World>>, Receiver<Team>) {
        let rules = Rules::default();
        // GO is now: the game has started.
        let world = Arc::new(Mutex::new(World::new(rules, 0.0)));
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let free = Free {
            grace: Duration::from_millis(grace_ms),
            take: Arc::new(move |t| {
                let _ = tx.lock().unwrap().send(t);
            }),
        };
        let air = Arc::new((0..2 * rules.drones as u16).map(|i| 7000 + i).collect());
        let seats = Seats::listen(0, world.clone(), people, air, free).unwrap();
        (seats, world, rx)
    }

    fn tick(seats: &Seats, world: &Arc<Mutex<World>>) {
        let mut w = world.lock().unwrap();
        w.game.look();
        seats.send(&w);
    }

    #[test]
    fn a_computers_seat_gets_the_same_kind_of_messages_as_a_persons_and_a_map_from_telemetry_only() {
        let (seats, world, _) = server([true, false], 50);
        let rules = world.lock().unwrap().game.rules;
        let n = rules.drones;
        // Red's scout is out in the middle of the field, where Blue sees nothing.
        {
            let mut w = world.lock().unwrap();
            let d = &mut w.game.drones[n];
            (d.at, d.alt, d.phase) = (Vec2::new(0.0, 0.0), 60.0, Phase::Flying);
            w.game.look();
        }
        let person = Client::join(seats.port(), Join::Command(Team::Blue));
        let computer = Client::join(seats.port(), Join::Compute(Team::Red));
        let spectator = Client::join(seats.port(), Join::Spectate);
        let blue_watcher = Client::join(seats.port(), Join::Watch(Team::Blue));
        for c in [&person, &computer, &spectator, &blue_watcher] {
            c.wait("a welcome", |m| m.iter().any(|m| matches!(m, ToSeat::Welcome { .. })));
        }
        tick(&seats, &world);
        for c in [&person, &computer] {
            c.wait("the umpire and the sites", |m| {
                m.iter().any(|m| matches!(m, ToSeat::Umpire(_))) && m.iter().any(|m| matches!(m, ToSeat::Sites(_)))
            });
        }
        spectator.wait("the truth", |m| m.iter().any(|m| matches!(m, ToSeat::Truth(_))));
        assert_eq!(person.kinds(), computer.kinds(), "the same kind of messages");
        assert_eq!(person.kinds(), ["Orders", "Sites", "Umpire", "Welcome"].into());
        // The map of Blue's seat, made from what it got and no telemetry, has no enemy in it.
        let (umpire, sites) = {
            let got = person.got.lock().unwrap();
            (
                got.iter().find_map(|m| if let ToSeat::Umpire(u) = m { Some(*u) } else { None }).unwrap(),
                got.iter().find_map(|m| if let ToSeat::Sites(s) = m { Some(s.clone()) } else { None }).unwrap(),
            )
        };
        let view = map::build(&umpire, &sites, &vec![None; n], &[], &[], &rules);
        let truth = spectator.got.lock().unwrap().iter().rev().find_map(|m| match m {
            ToSeat::Truth(t) => Some(t.clone()),
            _ => None,
        });
        let truth = truth.unwrap();
        assert_eq!(truth.drones[n].phase, Phase::Flying, "the spectator sees Red's scout in the open");
        assert!(view.enemies.iter().all(|e| !e.seen), "the seat's map does not have it: {:?}", view.enemies);
        assert!(!blue_watcher.kinds().contains("Truth"));
    }

    #[test]
    fn a_person_takes_the_seat_of_a_computer_and_goes_on_from_its_orders() {
        let (seats, world, _) = server([false, false], 50);
        let computer = Client::join(seats.port(), Join::Compute(Team::Blue));
        computer.wait("a welcome", |m| matches!(m.first(), Some(ToSeat::Welcome { .. })));
        let mut told = world.lock().unwrap().told.clone();
        told[1] = (Some(Order::Hold), true);
        world.lock().unwrap().told = told;
        let person = Client::join(seats.port(), Join::Command(Team::Blue));
        person.wait("the orders", |m| m.iter().any(|m| matches!(m, ToSeat::Orders(_))));
        let orders = person
            .got
            .lock()
            .unwrap()
            .iter()
            .find_map(|m| if let ToSeat::Orders(o) = m { Some(o.clone()) } else { None });
        assert_eq!(orders.unwrap()[1], (Some(Order::Hold), true));
        computer
            .wait("to be relieved", |m| m.iter().any(|m| matches!(m, ToSeat::Refused(w) if w.contains("relieved"))));
        // The person has the seat now: another person is refused, and a computer waits.
        let second = Client::join(seats.port(), Join::Command(Team::Blue));
        second.wait("a refusal", |m| {
            m.iter().any(|m| matches!(m, ToSeat::Refused(w) if w.contains("has its commander")))
        });
        let late = Client::join(seats.port(), Join::Compute(Team::Blue));
        late.wait("a refusal", |m| {
            m.iter().any(|m| matches!(m, ToSeat::Refused(w) if w.contains("has its commander")))
        });
    }

    #[test]
    fn the_computer_takes_a_seat_a_person_leaves_unless_the_person_comes_back_at_once() {
        let (seats, _world, rx) = server([true, false], 150);
        let person = Client::join(seats.port(), Join::Command(Team::Blue));
        person.wait("a welcome", |m| matches!(m.first(), Some(ToSeat::Welcome { .. })));
        assert!(seats.people_seated());
        person.leave();
        // Back inside the grace time: the seat is not given to the computer.
        std::thread::sleep(Duration::from_millis(30));
        let again = Client::join(seats.port(), Join::Command(Team::Blue));
        again.wait("a welcome", |m| matches!(m.first(), Some(ToSeat::Welcome { .. })));
        assert!(rx.recv_timeout(Duration::from_millis(400)).is_err(), "the person came back");
        again.leave();
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)), Ok(Team::Blue), "the seat is free: a computer takes it");
        assert!(!seats.people_seated());
    }

    #[test]
    fn two_people_one_for_each_team_can_join_the_same_server() {
        let (seats, world, _) = server([true, true], 50);
        let rules = world.lock().unwrap().game.rules;
        let blue = Client::join(seats.port(), Join::Command(Team::Blue));
        let red = Client::join(seats.port(), Join::Command(Team::Red));
        for c in [&blue, &red] {
            c.wait("a welcome", |m| m.iter().any(|m| matches!(m, ToSeat::Welcome { .. })));
        }
        let air_of = |c: &Client| {
            c.got
                .lock()
                .unwrap()
                .iter()
                .find_map(|m| if let ToSeat::Welcome { air, .. } = m { Some(air.clone()) } else { None })
                .unwrap()
        };
        assert_eq!(air_of(&blue), (7000..7000 + rules.drones as u16).collect::<Vec<_>>());
        assert_eq!(air_of(&red), (7000 + rules.drones as u16..7000 + 2 * rules.drones as u16).collect::<Vec<_>>());
        tick(&seats, &world);
        blue.wait("its sites", |m| m.iter().any(|m| matches!(m, ToSeat::Sites(s) if s.team == Team::Blue)));
        red.wait("its sites", |m| m.iter().any(|m| matches!(m, ToSeat::Sites(s) if s.team == Team::Red)));
        assert!(seats.people_seated());
    }
}
