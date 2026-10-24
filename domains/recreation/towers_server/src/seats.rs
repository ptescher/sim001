//! The seats, over TCP (`recreation_towers::seat`). A seat that commands a team is kept for a
//! person (`--blue person`), one at a time; any number may watch a team or the whole game. Each
//! gets what it may know, from the same moment of the game, four times a second.

use crate::world::World;
use recreation_towers::seat::{self, Join, ToSeat, ToServer, Truth};
use recreation_towers::{Advice, Rules, Team};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

struct Client {
    join: Join,
    stream: TcpStream,
}

pub struct Seats {
    clients: Arc<Mutex<Vec<Client>>>,
    /// The teams a person commands: their seats are not for a computer.
    people: [bool; 2],
}

/// The ports of the air in front of the drones, by drone number in the game.
pub type AirPorts = Arc<Vec<u16>>;

impl Seats {
    /// Listens on `port`. `people[team]`: that team's seat is kept for a person.
    pub fn listen(port: u16, world: Arc<Mutex<World>>, people: [bool; 2], air: AirPorts) -> std::io::Result<Seats> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(vec![]));
        let c2 = clients.clone();
        std::thread::Builder::new().name("seats".into()).spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let (clients, world, air) = (c2.clone(), world.clone(), air.clone());
                std::thread::spawn(move || seat(stream, clients, world, people, air));
            }
        })?;
        Ok(Seats { clients, people })
    }

    /// Whether every team a person commands has its person.
    pub fn people_seated(&self) -> bool {
        let clients = self.clients.lock().unwrap();
        Team::BOTH
            .iter()
            .filter(|t| self.people[t.index()])
            .all(|&t| clients.iter().any(|c| c.join == Join::Command(t)))
    }

    /// Tells the seat that commands `team` what its advisor has to say.
    pub fn advise(&self, team: Team, advice: &[Advice]) {
        let lines: Vec<String> = advice.iter().map(|a| seat::encode(&ToSeat::Advice(a.clone()))).collect();
        self.clients.lock().unwrap().retain_mut(|c| {
            c.join != Join::Command(team) || lines.iter().all(|line| writeln!(c.stream, "{line}").is_ok())
        });
    }

    /// Sends every seat what it may know of the game now; drops the seats that have gone.
    pub fn send(&self, world: &World) {
        let game = &world.game;
        // A seat that watches a team gets its whole map. One that commands is the team's ground station
        // and makes its own: it gets the umpire's word and what the team's towers and base report.
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
            Join::Command(t) => {
                writeln!(c.stream, "{umpire}").is_ok() && writeln!(c.stream, "{}", sites[t.index()]).is_ok()
            }
            Join::Watch(t) => writeln!(c.stream, "{}", views[t.index()]).is_ok(),
            Join::Spectate => writeln!(c.stream, "{truth}").is_ok(),
        });
    }
}

/// One seat's connection: what it is, then its orders.
fn seat(
    stream: TcpStream,
    clients: Arc<Mutex<Vec<Client>>>,
    world: Arc<Mutex<World>>,
    people: [bool; 2],
    air: AirPorts,
) {
    let Ok(mut out) = stream.try_clone() else { return };
    let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
    let Some(join) = lines.next().and_then(|l| seat::decode::<ToServer>(&l).ok()) else {
        let _ = writeln!(out, "{}", seat::encode(&ToSeat::Refused("say what the seat is first: Join".into())));
        return;
    };
    let ToServer::Join(join) = join else { return };
    let rules: Rules = world.lock().unwrap().game.rules;
    {
        let mut clients = clients.lock().unwrap();
        let refusal = match join {
            Join::Command(t) if !people[t.index()] => Some(format!("{t:?} is commanded by the computer")),
            Join::Command(t) if clients.iter().any(|c| c.join == join) => Some(format!("{t:?} has its commander")),
            _ => None,
        };
        if let Some(why) = refusal {
            let _ = writeln!(out, "{}", seat::encode(&ToSeat::Refused(why)));
            return;
        }
        let Ok(stream) = out.try_clone() else { return };
        // A seat that commands is its team's ground station: it gets the way to its drones.
        let ports = match join {
            Join::Command(t) => air[t.index() * rules.drones..(t.index() + 1) * rules.drones].to_vec(),
            _ => vec![],
        };
        if writeln!(out, "{}", seat::encode(&ToSeat::Welcome { join, rules, air: ports })).is_err() {
            return;
        }
        clients.push(Client { join, stream });
    }
    let Join::Command(team) = join else { return };
    for line in lines {
        if let Ok(ToServer::Order { drone, order, low, standing }) = seat::decode::<ToServer>(&line)
            && drone < rules.drones
        {
            let mut w = world.lock().unwrap();
            let i = w.game.drone(team, drone);
            // A drone out of contact does not hear it.
            w.command_standing(i, order, low, standing);
        }
    }
    // The commander has gone: its seat is free again.
    clients.lock().unwrap().retain(|c| c.join != join);
}
