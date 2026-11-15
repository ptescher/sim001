//! The seat protocol: what passes between a seat and the game server. One JSON message a line,
//! on TCP. A seat first says what it is ([`Join`]); the server answers with [`ToSeat::Welcome`]
//! and then sends what the seat may know four times a second, and, to a seat that commands, the
//! advice of its advisor when something matters. A seat that commands a team sends
//! orders for its drones.
//!
//! The fog is the server's: a seat that watches a team gets only that team's map
//! ([`ToSeat::View`]). A seat that commands is the team's ground station and makes its own map: the
//! server tells it only the umpire's word ([`ToSeat::Umpire`]) and what its towers and base see
//! ([`ToSeat::Sites`]). Only a spectator gets the whole game ([`ToSeat::Truth`]).

use crate::{
    Advice, Drone, EdgeObs, Game, Order, Outcome, Queued, Rules, Sites, Standing, Team, TeamView, Tower, Umpire,
};
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// The game server's port for seats: never a SITL port (those end in 0, 2 or 3).
pub const PORT: u16 = 5955;

/// What a seat is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Join {
    /// Commands a team, as a person at the commander app does.
    Command(Team),
    /// Commands a team, as a computer does: the same seat, with the same messages. It is a seat that
    /// gives way: a person who joins the team takes it from the computer.
    Compute(Team),
    /// Watches a team: its map, no orders.
    Watch(Team),
    /// Watches the whole game, with no fog.
    Spectate,
    /// Flies one drone of a team (its number in the team, from 0) by the sticks, as the drone's
    /// pilot: a winged drone, one pilot at a time. The server still takes the drone off and puts it
    /// on the wing; then the sticks are the pilot's.
    Unit { team: Team, drone: usize },
}

impl Join {
    /// The team the seat commands, for a person or a computer.
    pub fn commands(self) -> Option<Team> {
        match self {
            Join::Command(t) | Join::Compute(t) => Some(t),
            Join::Watch(_) | Join::Spectate | Join::Unit { .. } => None,
        }
    }

    /// The team and drone the seat flies, for a pilot.
    pub fn pilots(self) -> Option<(Team, usize)> {
        match self {
            Join::Unit { team, drone } => Some((team, drone)),
            _ => None,
        }
    }
}

/// A pilot's sticks, as a radio sends them: the server's drone takes them as its radio's channels.
/// Roll right and yaw right are positive; pitch is positive for the stick pulled back (nose up),
/// as the elevator of the radio is. Throttle is 0 (down) to 1 (up). A pilot sends them 20 times a
/// second or more: after [`STICKS_TIMEOUT`] s without them the drone is no longer flown by the pilot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sticks {
    pub roll: f32,
    pub pitch: f32,
    pub throttle: f32,
    pub yaw: f32,
}

impl Sticks {
    /// The sticks of a pilot who lets go: centred, throttle half.
    pub const CENTRED: Sticks = Sticks { roll: 0.0, pitch: 0.0, throttle: 0.5, yaw: 0.0 };

    /// The same, held to what a radio can send.
    pub fn clamped(self) -> Sticks {
        let c = |x: f32| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
        let throttle = if self.throttle.is_finite() { self.throttle.clamp(0.0, 1.0) } else { 0.0 };
        Sticks { roll: c(self.roll), pitch: c(self.pitch), throttle, yaw: c(self.yaw) }
    }
}

/// The pilot's sticks stopping this long (s of the wall's time) gives the drone back to its computer.
pub const STICKS_TIMEOUT: f32 = 2.0;

/// What a pilot is shown: the commander's instructions as a mission, and what the drone itself
/// knows. The mission (`order`, `low`, `queue`) is the commander's last word that reached the
/// drone: while the drone is out of contact it does not change, and it catches up when contact
/// returns. The rest is the drone's own: its camera, its battery and the time it has been out of
/// contact. The game does not check that the pilot flies the mission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mission {
    /// The drone's number in its team.
    pub drone: usize,
    pub order: Option<Order>,
    /// The order is flown low, along the lanes.
    pub low: bool,
    pub queue: Vec<Queued>,
    /// The drone is in contact with its team now (its radio reaches it).
    pub in_contact: bool,
    /// What the drone knows on board: where it is, what its camera sees, battery, time out of contact.
    pub obs: EdgeObs,
    /// Its velocity (m/s, north, east): the drone knows how it flies.
    pub vel: Vec2,
    /// How it sits (body to the world, a quaternion x, y, z, w, in the 3D view's frame: x east, y up,
    /// z south): the drone's own attitude, for the pilot's camera. `[0; 4]` when not known.
    #[serde(default)]
    pub attitude: [f32; 4],
    /// The towers, by their number in the game: their names and places (the field is known to all).
    pub towers: Vec<(String, Vec2)>,
    pub time: f32,
    pub time_left: f32,
    pub outcome: Option<Outcome>,
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
        /// What the drone does if it is out of contact for a time: a plan for the radio shadows.
        /// The old form of a queue of one; a queue, when there is one, wins (see [`crate::queue_of`]).
        #[serde(default)]
        standing: Option<Standing>,
        /// The queue of instructions of the drone, for the radio shadows: up to
        /// [`crate::MAX_QUEUE`], cut when longer. The commander replaces it with each order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        queue: Vec<Queued>,
    },
    /// The pilot's sticks, from a seat that is [`Join::Unit`].
    Sticks(Sticks),
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
    /// Which drones are out of contact, in the radio shadow of an enemy tower (same order).
    #[serde(default)]
    pub jammed: Vec<bool>,
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
            jammed: game.jammed.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToSeat {
    /// The seat is in. `team` is the team it commands or watches.
    Welcome {
        join: Join,
        rules: Rules,
        /// The ports of the air in front of the team's drones (drone `k` at `k`), on the server's
        /// host, for a seat that commands: the seat is the team's ground station and sends its
        /// drones MAVLink there. Empty when the server flies the team.
        #[serde(default)]
        air: Vec<u16>,
    },
    /// The seat may not have what it asked for (the team's seat is a computer's, or taken).
    Refused(String),
    /// The map of a team, for a seat that watches it.
    View(TeamView),
    /// The umpire's word (the clock, the scores, the outcome): to a seat that commands, which makes its
    /// own map from it, the reports of its sites and the telemetry of its drones.
    Umpire(Umpire),
    /// What the team's towers and base report, to a seat that commands.
    Sites(Sites),
    /// The orders the team's drones have now (by drone number in the team: the order, and whether it
    /// is flown low), once, right after the welcome of a seat that commands: a commander that takes
    /// over a game goes on from them.
    Orders(Vec<(Option<Order>, bool)>),
    Truth(Truth),
    /// The advisor of the team speaks, to a seat that commands it, when something matters.
    Advice(Advice),
    /// The drones of the team (by number in the team) that a person flies by the sticks now, to a
    /// seat that commands: the ground station leaves them alone (it does not put them in AGENT or
    /// send them plans), but their telemetry still comes down, so they stay on the map. Often.
    Piloted(Vec<usize>),
    /// To a pilot's seat: its mission, and what its drone knows, often.
    Mission(Mission),
}

/// A message as a line (without its newline).
pub fn encode<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("a seat message is always JSON")
}

pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("not a seat message ({e}): {line}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_order_with_a_queue_survives_the_wire() {
        let msg = ToServer::Order {
            drone: 1,
            order: Order::Hold,
            low: false,
            standing: None,
            queue: vec![Queued::silent(5.0, Order::Take(2)), Queued::done(Order::Home)],
        };
        assert_eq!(decode::<ToServer>(&encode(&msg)).unwrap(), msg);
    }

    #[test]
    fn a_pilots_seat_and_sticks_survive_the_wire_and_a_pilot_commands_nobody() {
        let join = Join::Unit { team: Team::Blue, drone: 1 };
        assert_eq!(decode::<ToServer>(&encode(&ToServer::Join(join))).unwrap(), ToServer::Join(join));
        assert_eq!((join.commands(), join.pilots()), (None, Some((Team::Blue, 1))));
        assert_eq!(Join::Command(Team::Red).pilots(), None);
        let sticks = ToServer::Sticks(Sticks { roll: 0.25, pitch: -0.5, throttle: 0.75, yaw: 1.0 });
        assert_eq!(decode::<ToServer>(&encode(&sticks)).unwrap(), sticks);
        let piloted = ToSeat::Piloted(vec![0, 2]);
        assert_eq!(decode::<ToSeat>(&encode(&piloted)).unwrap(), piloted);
    }

    #[test]
    fn sticks_are_held_to_what_a_radio_sends() {
        let wild = Sticks { roll: 3.0, pitch: f32::NAN, throttle: -2.0, yaw: -9.0 }.clamped();
        assert_eq!(wild, Sticks { roll: 1.0, pitch: 0.0, throttle: 0.0, yaw: -1.0 });
        assert_eq!(Sticks { throttle: 7.0, ..Sticks::CENTRED }.clamped().throttle, 1.0);
        assert_eq!(Sticks::CENTRED.clamped(), Sticks::CENTRED);
    }

    #[test]
    fn an_order_from_an_old_seat_has_no_queue_and_keeps_its_standing_order() {
        let old = r#"{"Order":{"drone":0,"order":"Hold","standing":{"after":5.0,"then":"Home"}}}"#;
        let Ok(ToServer::Order { standing, queue, .. }) = decode::<ToServer>(old) else { panic!("{old}") };
        assert!(queue.is_empty());
        assert_eq!(crate::queue_of(standing, queue), vec![Queued::silent(5.0, Order::Home)]);
    }
}
