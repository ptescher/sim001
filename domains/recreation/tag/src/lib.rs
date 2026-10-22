//! Tag: one plane is it and chases the others. When it touches one, that plane is it. The game
//! has a clock, and the player who was it for the least time wins.
//!
//! The referee only needs where each plane is and whether it is in the air. The game is played
//! in an arena (a circle of sky over the field, under a ceiling); a plane that leaves it is it.
//! A plane that has just been tagged can't tag back the plane that tagged it straight away.

use glam::Vec3;

/// The sky the game is played in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arena {
    /// The middle of the arena, on the ground.
    pub centre: Vec3,
    /// From the middle to the edge (m).
    pub radius: f32,
    /// How high above the middle the arena goes (m).
    pub ceiling: f32,
}

impl Arena {
    /// The arena's size for the T1-S: 90 m round the middle of the field, under 60 m.
    pub fn around(centre: Vec3) -> Arena {
        Arena { centre, radius: 90.0, ceiling: 60.0 }
    }

    pub fn contains(&self, p: Vec3) -> bool {
        let d = Vec3::new(p.x - self.centre.x, 0.0, p.z - self.centre.z).length();
        d <= self.radius && p.y - self.centre.y <= self.ceiling
    }
}

/// The rules of a game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    /// Planes closer than this (between their centres, m) touch: about two wingspans of a T1-S.
    pub touch: f32,
    /// A plane that has just been tagged can't tag back the plane that tagged it for this long (s).
    pub no_tag_back: f32,
    /// How long a game lasts (s).
    pub length: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { touch: 1.5, no_tag_back: 5.0, length: 120.0 }
    }
}

/// A plane in the game, as the referee sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub pos: Vec3,
    /// In the air (not in the hand, being carried back, or crashed).
    pub flying: bool,
}

/// What the referee saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// `by` touched `who`, and `who` is it.
    Tagged { by: usize, who: usize },
    /// `who` left the arena, and is it.
    OutOfBounds { who: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// How long this player has been it (s).
    pub it_time: f32,
    /// How many planes this player tagged.
    pub tags: u32,
}

/// The referee of one game.
#[derive(Clone, Debug)]
pub struct Tag {
    pub arena: Arena,
    pub rules: Rules,
    pub players: Vec<Player>,
    /// Who is it.
    pub it: usize,
    /// The plane that last tagged the one that is it, and when: it can't be tagged back yet.
    tagged_by: Option<(usize, f32)>,
    /// The game's clock (s).
    pub time: f32,
    /// Each plane was in the arena (or on the ground) when last seen: leaving it is what counts.
    inside: Vec<bool>,
}

impl Tag {
    pub fn new(arena: Arena, rules: Rules, players: usize, first_it: usize) -> Tag {
        Tag {
            arena,
            rules,
            players: (0..players).map(|_| Player { it_time: 0.0, tags: 0 }).collect(),
            it: first_it,
            tagged_by: None,
            time: 0.0,
            inside: vec![true; players],
        }
    }

    /// Runs the game on by `dt` seconds, with every plane where it is now.
    pub fn step(&mut self, planes: &[Plane], dt: f32) -> Vec<Call> {
        if self.over() {
            return vec![];
        }
        let run = dt.min(self.rules.length - self.time);
        self.players[self.it].it_time += run;
        self.time += run;
        if self.over() {
            return vec![];
        }
        let inside: Vec<bool> = planes.iter().map(|p| !p.flying || self.arena.contains(p.pos)).collect();
        let was_inside = std::mem::replace(&mut self.inside, inside);
        let chaser = planes[self.it];
        if chaser.flying {
            let touched = (0..planes.len())
                .filter(|&i| i != self.it && planes[i].flying && self.may_tag(i))
                .map(|i| (i, planes[i].pos.distance(chaser.pos)))
                .filter(|&(_, d)| d < self.rules.touch)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((who, _)) = touched {
                let by = self.it;
                self.players[by].tags += 1;
                self.it = who;
                self.tagged_by = Some((by, self.time));
                return vec![Call::Tagged { by, who }];
            }
        }
        if let Some(who) = (0..planes.len()).find(|&i| i != self.it && was_inside[i] && !self.inside[i]) {
            self.it = who;
            self.tagged_by = None;
            return vec![Call::OutOfBounds { who }];
        }
        vec![]
    }

    /// Whether the plane that is it may tag `who` now.
    pub fn may_tag(&self, who: usize) -> bool {
        who != self.it && !self.tagged_by.is_some_and(|(by, at)| by == who && self.time - at < self.rules.no_tag_back)
    }

    /// The clock has run out.
    pub fn over(&self) -> bool {
        self.time >= self.rules.length
    }

    /// The players in the order of their places: least time as it first.
    pub fn standings(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.players.len()).collect();
        order.sort_by(|&a, &b| self.players[a].it_time.total_cmp(&self.players[b].it_time));
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena() -> Arena {
        Arena::around(Vec3::ZERO)
    }

    fn at(x: f32, y: f32, z: f32) -> Plane {
        Plane { pos: Vec3::new(x, y, z), flying: true }
    }

    #[test]
    fn the_arena_is_a_circle_under_a_ceiling() {
        let a = arena();
        assert!(a.contains(Vec3::new(89.0, 20.0, 0.0)));
        assert!(!a.contains(Vec3::new(91.0, 20.0, 0.0)));
        assert!(!a.contains(Vec3::new(0.0, 61.0, 0.0)));
    }

    #[test]
    fn a_touch_passes_it_on() {
        let mut g = Tag::new(arena(), Rules::default(), 3, 0);
        let far = [at(0.0, 20.0, 0.0), at(30.0, 20.0, 0.0), at(-30.0, 20.0, 0.0)];
        assert!(g.step(&far, 0.1).is_empty());
        let touching = [at(0.0, 20.0, 0.0), at(1.0, 20.5, 0.0), at(-30.0, 20.0, 0.0)];
        assert_eq!(g.step(&touching, 0.1), vec![Call::Tagged { by: 0, who: 1 }]);
        assert_eq!(g.it, 1);
        assert_eq!(g.players[0].tags, 1);
    }

    #[test]
    fn no_tag_backs_for_five_seconds() {
        let mut g = Tag::new(arena(), Rules::default(), 2, 0);
        let touching = [at(0.0, 20.0, 0.0), at(1.0, 20.0, 0.0)];
        g.step(&touching, 0.1);
        assert_eq!(g.it, 1);
        assert!(!g.may_tag(0));
        // Still touching a moment later: no tag back.
        assert!(g.step(&touching, 1.0).is_empty());
        assert_eq!(g.it, 1);
        for _ in 0..45 {
            g.step(&[at(0.0, 20.0, 0.0), at(50.0, 20.0, 0.0)], 0.1);
        }
        assert!(g.may_tag(0));
        assert_eq!(g.step(&touching, 0.1), vec![Call::Tagged { by: 1, who: 0 }]);
    }

    #[test]
    fn a_plane_on_the_ground_cannot_be_tagged() {
        let mut g = Tag::new(arena(), Rules::default(), 2, 0);
        let landed = Plane { pos: Vec3::new(1.0, 0.1, 0.0), flying: false };
        assert!(g.step(&[at(0.0, 1.0, 0.0), landed], 0.1).is_empty());
        assert_eq!(g.it, 0);
        // Nor can a plane that is it tag from the ground.
        let mut g = Tag::new(arena(), Rules::default(), 2, 0);
        assert!(g.step(&[Plane { pos: Vec3::new(0.0, 0.1, 0.0), flying: false }, at(1.0, 0.5, 0.0)], 0.1).is_empty());
    }

    #[test]
    fn out_of_the_arena_you_are_it() {
        let mut g = Tag::new(arena(), Rules::default(), 3, 0);
        let out = [at(0.0, 20.0, 0.0), at(95.0, 20.0, 0.0), at(10.0, 20.0, 0.0)];
        assert_eq!(g.step(&out, 0.1), vec![Call::OutOfBounds { who: 1 }]);
        assert_eq!(g.it, 1);
        // The plane that is it may go where it likes.
        assert!(g.step(&out, 0.1).is_empty());
        // The plane that was it before can be tagged straight away: nobody tagged anybody.
        assert!(g.may_tag(0));
    }

    #[test]
    fn it_goes_to_the_plane_that_flies_out_once_not_every_step_it_is_out() {
        let mut g = Tag::new(arena(), Rules::default(), 3, 0);
        let inside = [at(0.0, 20.0, 0.0), at(80.0, 20.0, 0.0), at(-80.0, 20.0, 0.0)];
        g.step(&inside, 0.1);
        // Both fly out together: the first is it, and stays it while they're both out.
        let out = [at(0.0, 20.0, 0.0), at(95.0, 20.0, 0.0), at(-95.0, 20.0, 0.0)];
        g.step(&out, 0.1);
        let first = g.it;
        assert!(first == 1 || first == 2);
        for _ in 0..10 {
            g.step(&out, 0.1);
            assert_eq!(g.it, first, "it doesn't swap between planes that are out");
        }
        // The other one comes back in and goes out again: now it is it.
        let other = 3 - first;
        let mut back = out;
        back[other] = at(0.0, 20.0, 50.0);
        g.step(&back, 0.1);
        g.step(&out, 0.1);
        assert_eq!(g.it, other);
    }

    #[test]
    fn the_clock_counts_the_time_each_player_is_it() {
        let mut g = Tag::new(arena(), Rules::default(), 3, 0);
        let apart = [at(0.0, 20.0, 0.0), at(30.0, 20.0, 0.0), at(-30.0, 20.0, 0.0)];
        for _ in 0..100 {
            g.step(&apart, 0.1);
        }
        g.step(&[at(0.0, 20.0, 0.0), at(30.0, 20.0, 0.0), at(-0.5, 20.0, 0.0)], 0.1);
        for _ in 0..299 {
            g.step(&apart, 0.1);
        }
        assert!((g.players[0].it_time - 10.1).abs() < 0.05, "{:?}", g.players);
        assert!((g.players[2].it_time - 29.9).abs() < 0.05, "{:?}", g.players);
        assert_eq!(g.players[1].it_time, 0.0);
        assert_eq!(g.standings(), vec![1, 0, 2]);
        assert!(!g.over());
        for _ in 0..900 {
            g.step(&apart, 0.1);
        }
        assert!(g.over());
        // The clock stops at the end of the game.
        let total: f32 = g.players.iter().map(|p| p.it_time).sum();
        assert!((total - 120.0).abs() < 0.05, "{total}");
    }
}
