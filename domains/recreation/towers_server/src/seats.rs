//! The seats, over TCP (`recreation_towers::seat`). A seat that commands a team is kept for a
//! person (`--blue person`), one at a time; any number may watch a team or the whole game. Each
//! gets what it may know, from the same moment of the game, four times a second.

use crate::world::World;
use recreation_towers::seat::{self, Join, ToSeat, ToServer, Truth};
use recreation_towers::{Rules, Team};
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

impl Seats {
    /// Listens on `port`. `people[team]`: that team's seat is kept for a person.
    pub fn listen(port: u16, world: Arc<Mutex<World>>, people: [bool; 2]) -> std::io::Result<Seats> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(vec![]));
        let c2 = clients.clone();
        std::thread::Builder::new().name("seats".into()).spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let (clients, world) = (c2.clone(), world.clone());
                std::thread::spawn(move || seat(stream, clients, world, people));
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

    /// Sends every seat what it may know of the game now; drops the seats that have gone.
    pub fn send(&self, world: &World) {
        let game = &world.game;
        let views = [game.view(Team::Blue), game.view(Team::Red)].map(|v| seat::encode(&ToSeat::View(v)));
        let truth = seat::encode(&ToSeat::Truth(Truth::of(game)));
        self.clients.lock().unwrap().retain_mut(|c| {
            let line = match c.join {
                Join::Command(t) | Join::Watch(t) => &views[t.index()],
                Join::Spectate => &truth,
            };
            writeln!(c.stream, "{line}").is_ok()
        });
    }
}

/// One seat's connection: what it is, then its orders.
fn seat(stream: TcpStream, clients: Arc<Mutex<Vec<Client>>>, world: Arc<Mutex<World>>, people: [bool; 2]) {
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
        if writeln!(out, "{}", seat::encode(&ToSeat::Welcome { join, rules })).is_err() {
            return;
        }
        clients.push(Client { join, stream });
    }
    let Join::Command(team) = join else { return };
    for line in lines {
        if let Ok(ToServer::Order { drone, order }) = seat::decode::<ToServer>(&line)
            && drone < rules.drones
        {
            let mut w = world.lock().unwrap();
            let i = w.game.drone(team, drone);
            w.orders[i] = Some(order);
        }
    }
    // The commander has gone: its seat is free again.
    clients.lock().unwrap().retain(|c| c.join != join);
}
