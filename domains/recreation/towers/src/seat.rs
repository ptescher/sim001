//! The seat protocol: what passes between a seat and the game server. One JSON message a line,
//! on TCP. A seat first says what it is ([`Join`]); the server answers with [`ToSeat::Welcome`]
//! and then sends what the seat may know four times a second. A seat that commands a team sends
//! orders for its drones.
//!
//! The fog is the server's: a seat that commands or watches a team gets only that team's map
//! ([`ToSeat::View`]). Only a spectator gets the whole game ([`ToSeat::Truth`]).

use crate::{Drone, Game, Order, Outcome, Rules, Team, TeamView, Tower};
use serde::{Deserialize, Serialize};

/// The game server's port for seats: never a SITL port (those end in 0, 2 or 3).
pub const PORT: u16 = 5955;

/// What a seat is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Join {
    /// Commands a team (a seat the server was told to keep for a person).
    Command(Team),
    /// Watches a team: its map, no orders.
    Watch(Team),
    /// Watches the whole game, with no fog.
    Spectate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToServer {
    Join(Join),
    /// An order for the seat's team's drone `drone` (its number in the team), flown `low` (along
    /// the lanes, below the treetops) or high (straight over the forest).
    Order {
        drone: usize,
        order: Order,
        #[serde(default)]
        low: bool,
    },
}

/// The whole game, for a spectator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Truth {
    pub time: f32,
    pub time_left: f32,
    pub score: [f32; 2],
    pub outcome: Option<Outcome>,
    pub towers: Vec<Tower>,
    /// Blue's drones, then Red's.
    pub drones: Vec<Drone>,
    /// How each drone sits (its body to the world, a quaternion x, y, z, w, in the 3D view's frame:
    /// x east, y up, z south), when the host knows. For a view of the game.
    #[serde(default)]
    pub attitudes: Vec<[f32; 4]>,
}

impl Truth {
    pub fn of(game: &Game) -> Truth {
        Truth {
            time: game.time,
            time_left: game.time_left(),
            score: game.score,
            outcome: game.outcome,
            towers: game.towers.clone(),
            drones: game.drones.clone(),
            attitudes: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToSeat {
    /// The seat is in. `team` is the team it commands or watches.
    Welcome {
        join: Join,
        rules: Rules,
    },
    /// The seat may not have what it asked for (the team's seat is a computer's, or taken).
    Refused(String),
    View(TeamView),
    Truth(Truth),
}

/// A message as a line (without its newline).
pub fn encode<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("a seat message is always JSON")
}

pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("not a seat message ({e}): {line}"))
}
