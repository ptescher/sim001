//! The record of a game (`plans/game-tooling.md`): one JSON [`Line`] after another, the same from
//! the arena and from the game server, for the tools that replay, render and analyse a game. A
//! [`Recorder`] makes it from the game's state as the game goes: it is given the game after each
//! step, and it notices the changes itself (a tower taken, an enemy drone spotted or lost, a drone
//! down or up, a new entry on a team's board).

use crate::map::Map;
use crate::{Board, Entry, Game, Order, Outcome, Phase, Rules, Team};
use serde::{Deserialize, Serialize};
use std::io::Write;

/// How often a [`Line::Tick`] is written (s of game time).
pub const TICK_EVERY: f32 = 0.5;

/// One line of a record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Line {
    Header(Box<Header>),
    /// The referee's truth.
    Tick(Tick),
    /// The orders a team gave at a decision: `(drone by its number in the team, order)`.
    Orders {
        t: f32,
        team: Team,
        orders: Vec<(usize, Order)>,
    },
    /// Team `by` sees enemy drone `drone` (by its index in the game) now, and did not before.
    Spotted {
        t: f32,
        by: Team,
        drone: usize,
    },
    /// Team `by` no longer sees enemy drone `drone`.
    Lost {
        t: f32,
        by: Team,
        drone: usize,
    },
    /// Tower `tower` has a new holder (none: it is neutral again).
    Taken {
        t: f32,
        tower: usize,
        by: Option<Team>,
    },
    /// Drone `drone` is down (out of the game until it is back).
    Down {
        t: f32,
        drone: usize,
    },
    /// Drone `drone` is back at its base.
    Up {
        t: f32,
        drone: usize,
    },
    /// An entry that arrived on a team's board.
    Board {
        t: f32,
        team: Team,
        entry: Entry,
    },
    /// The commander took the proposal that is entry `entry` on its board.
    Took {
        t: f32,
        team: Team,
        entry: u64,
    },
    End(End),
}

/// What a record starts with: the rules, the seed, who plays, and the field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Header {
    pub rules: Rules,
    pub seed: u64,
    /// Who plays each side (Blue's, Red's): the commander and its advisors, in words.
    pub sides: [String; 2],
    /// Each drone, by its index in the game (Blue's first): its team and class.
    pub drones: Vec<(Team, String)>,
    pub towers: Vec<TowerAt>,
    pub field: Field,
}

/// A tower where it stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TowerAt {
    pub name: String,
    pub kind: String,
    pub at: [f32; 2],
}

/// The ground, for drawing: metres north and east of the field's centre.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub radius: f32,
    pub bases: [[f32; 2]; 2],
    /// The lake's shore, all the way round.
    pub lake: Vec<[f32; 2]>,
    /// The lanes, each a line of points, and their half width.
    pub lanes: Vec<Vec<[f32; 2]>>,
    pub lane_half_width: f32,
    /// The clearings: centre and radius.
    pub clearings: Vec<([f32; 2], f32)>,
}

/// The referee's truth at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tick {
    pub t: f32,
    pub score: [f32; 2],
    pub drones: Vec<DroneTick>,
    /// Each tower's control (-1 Red's to +1 Blue's) and holder.
    pub towers: Vec<(f32, Option<Team>)>,
}

/// One drone at a tick.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DroneTick {
    pub at: [f32; 2],
    pub alt: f32,
    pub phase: Phase,
    /// The order it flies.
    pub order: Option<Order>,
    /// Its team hears it (not in an enemy tower's radio shadow).
    pub linked: bool,
    /// Its battery left (s; 0 if the game has none).
    pub battery: f32,
}

/// How the game ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct End {
    pub t: f32,
    pub outcome: Outcome,
    pub score: [f32; 2],
}

/// Rounds to a tenth: a record need not carry more, and is the smaller for it.
fn r(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

fn p(v: glam::Vec2) -> [f32; 2] {
    [r(v.x), r(v.y)]
}

impl Header {
    /// The header of a game that starts as `game` is, on `seed`, played by `sides`.
    pub fn of(game: &Game, seed: u64, sides: [String; 2]) -> Header {
        let map: &Map = &game.map;
        Header {
            rules: game.rules,
            seed,
            sides,
            drones: game.drones.iter().map(|d| (d.team, format!("{:?}", d.class).to_lowercase())).collect(),
            towers: game
                .towers
                .iter()
                .map(|t| TowerAt { name: t.name.clone(), kind: format!("{:?}", t.kind).to_lowercase(), at: p(t.at) })
                .collect(),
            field: Field {
                radius: map.field_radius,
                bases: [p(map.bases[0]), p(map.bases[1])],
                lake: map.lake.outline(72).into_iter().map(p).collect(),
                lanes: map.lanes.iter().map(|l| l.points.iter().map(|&q| p(q)).collect()).collect(),
                lane_half_width: crate::map::LANE_HALF_WIDTH,
                clearings: map.clearings.iter().map(|c| (p(c.at), r(c.radius))).collect(),
            },
        }
    }
}

/// Writes a game's record as it goes.
pub struct Recorder {
    out: Box<dyn Write + Send>,
    next_tick: f32,
    /// What each team saw of each drone at the last step.
    seen: [Vec<bool>; 2],
    holders: Vec<Option<Team>>,
    down: Vec<bool>,
    /// How much of each team's board, and of its taken proposals, is written.
    posted: [usize; 2],
    took: [usize; 2],
    ended: bool,
    /// The first error writing, if any (the game goes on; the record is short).
    pub error: Option<String>,
}

impl Recorder {
    /// A recorder that writes to `out`, starting with `header`.
    pub fn new(out: Box<dyn Write + Send>, game: &Game, header: Header) -> Recorder {
        let n = game.drones.len();
        let mut rec = Recorder {
            out,
            next_tick: 0.0,
            seen: [vec![false; n], vec![false; n]],
            holders: game.towers.iter().map(|t| t.holder).collect(),
            down: vec![false; n],
            posted: [0; 2],
            took: [0; 2],
            ended: false,
            error: None,
        };
        rec.write(&Line::Header(Box::new(header)));
        rec
    }

    /// A recorder that writes to the file `path`.
    pub fn to_file(path: &std::path::Path, game: &Game, header: Header) -> std::io::Result<Recorder> {
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        Ok(Recorder::new(Box::new(file), game, header))
    }

    fn write(&mut self, line: &Line) {
        if self.error.is_some() {
            return;
        }
        let res = serde_json::to_string(line)
            .map_err(|e| e.to_string())
            .and_then(|s| writeln!(self.out, "{s}").map_err(|e| e.to_string()));
        if let Err(e) = res {
            self.error = Some(e);
        }
    }

    /// The orders `team` gave at the decision at game time `t`.
    pub fn orders(&mut self, t: f32, team: Team, orders: &[(usize, Order)]) {
        if !orders.is_empty() {
            self.write(&Line::Orders { t: r(t), team, orders: orders.to_vec() });
        }
    }

    /// After a step: the changes since the last one, a tick when it is time, and the teams' boards
    /// (Blue's, Red's; none for a team with no board). `flown` is the order each drone flies, and
    /// `linked` whether its team hears it.
    pub fn step(&mut self, game: &Game, flown: &[Option<Order>], boards: [Option<&Board>; 2]) {
        if self.ended {
            return;
        }
        let t = game.time;
        for (k, tower) in game.towers.iter().enumerate() {
            if tower.holder != self.holders[k] {
                self.holders[k] = tower.holder;
                self.write(&Line::Taken { t: r(t), tower: k, by: tower.holder });
            }
        }
        for team in Team::BOTH {
            let ti = team.index();
            for (i, d) in game.drones.iter().enumerate() {
                if d.team == team {
                    continue;
                }
                let now = game.maps[ti].drones.get(i).and_then(|s| *s).is_some_and(|s| s.seen);
                if now != self.seen[ti][i] {
                    self.seen[ti][i] = now;
                    let line = if now {
                        Line::Spotted { t: r(t), by: team, drone: i }
                    } else {
                        Line::Lost { t: r(t), by: team, drone: i }
                    };
                    self.write(&line);
                }
            }
        }
        for (i, d) in game.drones.iter().enumerate() {
            let down = d.phase == Phase::Down;
            if down != self.down[i] {
                self.down[i] = down;
                self.write(&if down { Line::Down { t: r(t), drone: i } } else { Line::Up { t: r(t), drone: i } });
            }
        }
        for team in Team::BOTH {
            let ti = team.index();
            let Some(board) = boards[ti] else { continue };
            let new: Vec<Entry> = board.entries()[self.posted[ti].min(board.entries().len())..].to_vec();
            self.posted[ti] = board.entries().len();
            for entry in new {
                self.write(&Line::Board { t: r(t), team, entry });
            }
            let taken: Vec<(f32, u64)> = board.taken()[self.took[ti].min(board.taken().len())..].to_vec();
            self.took[ti] = board.taken().len();
            for (at, entry) in taken {
                self.write(&Line::Took { t: r(at), team, entry });
            }
        }
        if t >= self.next_tick {
            // On the half second, not half a second after the last: the steps do not land on it.
            while self.next_tick <= t {
                self.next_tick += TICK_EVERY;
            }
            self.write(&Line::Tick(tick(game, flown)));
        }
        if let Some(outcome) = game.outcome {
            self.ended = true;
            self.write(&Line::Tick(tick(game, flown)));
            self.write(&Line::End(End { t: r(t), outcome, score: [r(game.score[0]), r(game.score[1])] }));
            if let Err(e) = self.out.flush() {
                self.error.get_or_insert(e.to_string());
            }
        }
    }
}

fn tick(game: &Game, flown: &[Option<Order>]) -> Tick {
    Tick {
        t: r(game.time),
        score: [r(game.score[0]), r(game.score[1])],
        drones: game
            .drones
            .iter()
            .enumerate()
            .map(|(i, d)| DroneTick {
                at: p(d.at),
                alt: r(d.alt),
                phase: d.phase,
                order: flown.get(i).copied().flatten(),
                linked: !game.jammed.get(i).copied().unwrap_or(false),
                battery: r(game.battery.get(i).copied().unwrap_or(0.0)),
            })
            .collect(),
        towers: game.towers.iter().map(|t| ((t.control * 100.0).round() / 100.0, t.holder)).collect(),
    }
}

/// Reads a record back, a line at a time; a line that does not read is an error with its number.
pub fn read(text: &str) -> Result<Vec<Line>, String> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", n + 1)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that keeps what it is given, for the tests.
    #[derive(Clone, Default)]
    struct Kept(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Kept {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_record_reads_back_with_its_header_ticks_and_changes() {
        let mut g = Game::new(Rules::default());
        let kept = Kept::default();
        let header = Header::of(&g, 7, ["blue side".into(), "red side".into()]);
        let mut rec = Recorder::new(Box::new(kept.clone()), &g, header);
        let flown = vec![None; g.drones.len()];
        rec.step(&g, &flown, [None, None]);
        // A Red drone where Blue's home tower sees it, and the home tower lost.
        let red = g.drone(Team::Red, 0);
        g.drones[red] = crate::Drone { at: g.towers[0].at, alt: 20.0, phase: Phase::Flying, ..g.drones[red] };
        g.step(0.1);
        rec.orders(g.time, Team::Blue, &[(0, Order::Home)]);
        rec.step(&g, &flown, [None, None]);
        g.towers[1].holder = Some(Team::Red);
        g.outcome = Some(Outcome::Draw);
        rec.step(&g, &flown, [None, None]);
        assert_eq!(rec.error, None);
        let lines = read(&String::from_utf8(kept.0.lock().unwrap().clone()).unwrap()).unwrap();
        let Line::Header(h) = &lines[0] else { panic!("{:?}", lines[0]) };
        assert_eq!((h.seed, h.towers.len(), h.drones.len()), (7, g.towers.len(), g.drones.len()));
        assert!(!h.field.lanes.is_empty() && !h.field.lake.is_empty());
        assert!(matches!(lines[1], Line::Tick(_)));
        assert!(lines.contains(&Line::Spotted { t: r(g.time), by: Team::Blue, drone: red }), "{lines:#?}");
        assert!(lines.contains(&Line::Taken { t: r(g.time), tower: 1, by: Some(Team::Red) }));
        assert!(lines.iter().any(|l| matches!(l, Line::Orders { team: Team::Blue, .. })));
        assert!(matches!(lines.last(), Some(Line::End(End { outcome: Outcome::Draw, .. }))));
        // Nothing after the end.
        let n = lines.len();
        rec.step(&g, &flown, [None, None]);
        assert_eq!(read(&String::from_utf8(kept.0.lock().unwrap().clone()).unwrap()).unwrap().len(), n);
    }

    #[test]
    fn a_board_goes_into_the_record_with_what_the_commander_took() {
        let g = Game::new(Rules::default());
        let kept = Kept::default();
        let mut rec = Recorder::new(Box::new(kept.clone()), &g, Header::of(&g, 1, ["".into(), "".into()]));
        let mut board = Board::default();
        let p = crate::Proposal {
            drone: 0,
            order: Order::Home,
            queue: vec![],
            gain: 9.0,
            made: 0.0,
            until: 10.0,
            why: "test".into(),
        };
        board.post(Entry::proposal("lookahead", p.clone()));
        board.took(0.0, &p);
        let flown = vec![None; g.drones.len()];
        rec.step(&g, &flown, [Some(&board), None]);
        rec.step(&g, &flown, [Some(&board), None]);
        let lines = read(&String::from_utf8(kept.0.lock().unwrap().clone()).unwrap()).unwrap();
        let boards: Vec<_> = lines.iter().filter(|l| matches!(l, Line::Board { .. })).collect();
        assert_eq!(boards.len(), 1, "an entry is written once");
        assert!(lines.contains(&Line::Took { t: 0.0, team: Team::Blue, entry: 1 }));
    }
}
