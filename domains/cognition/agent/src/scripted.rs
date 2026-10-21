//! The computer pilots' strategy (`recreation_games`: Kestrel, Merlin and Swift) as plans.
//!
//! The plane that is it pursues the plane it can catch soonest: near, slow and ahead of it, and it
//! sticks with the one it chose unless another is clearly quicker to catch (the games crate's
//! `choose_target`). The others evade the chaser when it comes within the distance their skill
//! lets them see it at (`awareness`), and otherwise hold a height band of their own over the
//! arena, each band at least 15 m from the next so that they don't meet by chance; one heading
//! out of the arena is sent back to its middle first.

use crate::{Brain, Craft, Order, Picture};
use cognition_plan::{Primitive, WirePlan};
use glam::{Vec2, Vec3};

/// About as fast as a T1-S flies flat out (m/s): what a chaser has to catch with
/// (`recreation_games`' `TOP_SPEED`).
pub const TOP_SPEED: f32 = 26.0;
/// How long each plan is good for (s): a plane whose ground goes quiet for this long loiters.
pub const TTL: f32 = 2.5;
/// The height bands (lowest, thickness; m above home) given to the planes in the roster's order:
/// 15 m apart, between the executor's floor (12 m) and the arena's ceiling (60 m).
pub const BANDS: [(f32, f32); 3] = [(20.0, 10.0), (35.0, 10.0), (50.0, 8.0)];

/// How a computer pilot plays: the parts of `recreation_pilot::Skill` that are strategy, not
/// stick work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AgentSkill {
    pub name: &'static str,
    /// How far away it sees a chaser coming (m): `Skill::awareness`.
    pub awareness: f32,
    /// It keeps chasing its target unless another is this many times quicker to catch.
    pub stick: f32,
    /// How far ahead it aims at its target (s of the target's velocity).
    pub lead: f32,
    /// Sent back to the middle when this far out (a fraction of the arena's radius) and heading out.
    pub edge: f32,
}

pub const ACE: AgentSkill = AgentSkill { name: "ace", awareness: 80.0, stick: 1.5, lead: 1.5, edge: 0.75 };
pub const CLUB: AgentSkill = AgentSkill { name: "club", awareness: 60.0, stick: 1.5, lead: 1.0, edge: 0.8 };
pub const ROOKIE: AgentSkill = AgentSkill { name: "rookie", awareness: 40.0, stick: 2.0, lead: 0.0, edge: 0.85 };

/// How one plane plays, as numbers that can be tuned: its [`AgentSkill`], its height band and
/// how long each plan is good for. Each number has bounds ([`Tactics::BOUNDS`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tactics {
    pub awareness: f32,
    pub stick: f32,
    pub lead: f32,
    pub edge: f32,
    /// Lowest height (m above home) and thickness of its band.
    pub band_low: f32,
    pub band_thickness: f32,
    /// How long each plan is good for (s).
    pub ttl: f32,
}

impl Tactics {
    pub const LEN: usize = 7;
    pub const NAMES: [&'static str; Tactics::LEN] =
        ["awareness", "stick", "lead", "edge", "band_low", "band_thickness", "ttl"];
    /// Least and most of each. The band stays above the executor's floor (12 m) and under the
    /// arena's ceiling (60 m); a plan is good for at least the second until the next one.
    pub const BOUNDS: [(f32, f32); Tactics::LEN] =
        [(10.0, 150.0), (1.0, 4.0), (0.0, 4.0), (0.4, 0.95), (15.0, 55.0), (2.0, 30.0), (1.5, 6.0)];

    /// A computer pilot's skill, in the first band, with the plans' usual ttl.
    pub fn of(skill: &AgentSkill) -> Tactics {
        Tactics {
            awareness: skill.awareness,
            stick: skill.stick,
            lead: skill.lead,
            edge: skill.edge,
            band_low: BANDS[0].0,
            band_thickness: BANDS[0].1,
            ttl: TTL,
        }
    }

    pub fn to_array(&self) -> [f32; Tactics::LEN] {
        [self.awareness, self.stick, self.lead, self.edge, self.band_low, self.band_thickness, self.ttl]
    }

    /// `None` unless each number is finite and within its bounds.
    pub fn from_array(a: &[f32; Tactics::LEN]) -> Option<Tactics> {
        let ok = a.iter().zip(Tactics::BOUNDS).all(|(v, (lo, hi))| v.is_finite() && (lo..=hi).contains(v));
        let [awareness, stick, lead, edge, band_low, band_thickness, ttl] = *a;
        ok.then_some(Tactics { awareness, stick, lead, edge, band_low, band_thickness, ttl })
    }

    fn skill(&self) -> AgentSkill {
        AgentSkill { name: "tactics", awareness: self.awareness, stick: self.stick, lead: self.lead, edge: self.edge }
    }
}

/// One of the brain's planes.
#[derive(Clone, Debug)]
struct Player {
    id: u8,
    skill: AgentSkill,
    /// Lowest height (m above home) and thickness of its band.
    band: (f32, f32),
    /// Who it chases, while it is it.
    target: Option<u8>,
    /// The id of its last plan.
    plan_id: u16,
    /// How long each of its plans is good for (s).
    ttl: f32,
}

/// The scripted ground agent.
#[derive(Clone, Debug)]
pub struct ScriptedBrain {
    players: Vec<Player>,
    /// For planes not in the roster.
    default_skill: AgentSkill,
}

impl ScriptedBrain {
    /// `roster`: each plane's id and skill. Each gets the next of [`BANDS`] (past the third, the
    /// bands start again).
    pub fn new(roster: &[(u8, AgentSkill)]) -> ScriptedBrain {
        let players = roster
            .iter()
            .enumerate()
            .map(|(k, &(id, skill))| Player {
                id,
                skill,
                band: BANDS[k % BANDS.len()],
                target: None,
                plan_id: 0,
                ttl: TTL,
            })
            .collect();
        ScriptedBrain { players, default_skill: CLUB }
    }

    /// One plane, `id`, that plays by `tactics`.
    pub fn with_tactics(id: u8, tactics: &Tactics) -> ScriptedBrain {
        let player = Player {
            id,
            skill: tactics.skill(),
            band: (tactics.band_low, tactics.band_thickness),
            target: None,
            plan_id: 0,
            ttl: tactics.ttl,
        };
        ScriptedBrain { players: vec![player], default_skill: CLUB }
    }

    /// The computer pilots of `recreation_games::PILOTS` with ids 1, 2 and 3: the ace, the club
    /// pilot and the rookie.
    pub fn computer_pilots() -> ScriptedBrain {
        ScriptedBrain::new(&[(1, ACE), (2, CLUB), (3, ROOKIE)])
    }

    /// The band (lowest, thickness) plane `id` holds.
    pub fn band(&self, id: u8) -> Option<(f32, f32)> {
        self.players.iter().find(|p| p.id == id).map(|p| p.band)
    }

    /// Who plane `id` chases, if it is it and has chosen.
    pub fn target(&self, id: u8) -> Option<u8> {
        self.players.iter().find(|p| p.id == id).and_then(|p| p.target)
    }

    fn player(&mut self, id: u8) -> &mut Player {
        if let Some(k) = self.players.iter().position(|p| p.id == id) {
            return &mut self.players[k];
        }
        let band = BANDS[self.players.len() % BANDS.len()];
        self.players.push(Player { id, skill: self.default_skill, band, target: None, plan_id: 0, ttl: TTL });
        self.players.last_mut().unwrap()
    }
}

fn horiz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// Seconds the chaser `me` needs to catch `t`: the distance over the speed it has on it (a slow
/// plane is easy), and 3 s more if it has to turn round first.
pub fn time_to_catch(me: &Craft, t: &Craft) -> f32 {
    let to = t.pos - me.pos;
    let ahead = horiz(me.vel).normalize_or_zero();
    let behind = if horiz(to).normalize_or_zero().dot(ahead) < 0.0 { 3.0 } else { 0.0 };
    to.length() / (TOP_SPEED - t.vel.length()).max(3.0) + behind
}

/// Who the plane that is it should chase: the one it can catch soonest, unless the one it chases
/// now (`now`) is less than `stick` times slower to catch. Planes on the ground and the one it
/// may not tag back are not candidates.
pub fn choose_target(picture: &Picture, me: &Craft, now: Option<u8>, stick: f32) -> Option<u8> {
    let can = |c: &&Craft| c.id != me.id && c.flying && c.id != picture.game.no_tag_back;
    let best =
        picture.planes.iter().filter(can).min_by(|a, b| time_to_catch(me, a).total_cmp(&time_to_catch(me, b)))?;
    match now.and_then(|id| picture.plane(id)).filter(|c| can(c)) {
        Some(c) if time_to_catch(me, c) < stick * time_to_catch(me, best) => Some(c.id),
        _ => Some(best.id),
    }
}

/// Where to break to, away from `threat`: to the side, towards the middle of the arena, the more
/// so the nearer the edge (the games crate's `evade`). Horizontal NED, 60 m out.
pub fn break_point(me: &Craft, threat: &Craft, centre: Vec2, radius: f32) -> Vec2 {
    let p = horiz(me.pos);
    let away = (p - horiz(threat.pos)).normalize_or_zero();
    let to_centre = centre - p;
    let side = Vec2::new(-away.y, away.x);
    let side = if side.dot(to_centre) >= 0.0 { side } else { -side };
    let edge = (to_centre.length() / radius - 0.6).clamp(0.0, 0.4) / 0.4;
    let dir = (away * 0.5 + side * 0.9 + to_centre.normalize_or_zero() * 2.0 * edge).normalize_or_zero();
    p + dir * 60.0
}

impl Brain for ScriptedBrain {
    fn decide(&mut self, picture: &Picture) -> Vec<Order> {
        let g = picture.game;
        let centre = Vec2::from(g.centre);
        let mut orders = vec![];
        for me in picture.planes.iter().filter(|c| c.flying) {
            let chaser = picture.plane(g.it).filter(|c| c.flying && c.id != me.id).copied();
            let p = self.player(me.id);
            let (low, thick) = p.band;
            let (low, thick) = (low.max(g.floor), thick.min(g.ceiling - low.max(g.floor)).max(0.0));
            let mid = low + thick / 2.0;
            // (primitive, target, point north-east, alt, radius)
            let mut plan = (Primitive::Band, 0u8, centre, low, thick);
            if g.time_left <= 0.0 {
                // The game is over: circle over the arena until put away.
                p.target = None;
                plan = (Primitive::Orbit, 0, centre, mid, (g.radius * 0.6).round());
            } else if g.it == me.id {
                p.target = choose_target(picture, me, p.target, p.skill.stick);
                if let Some(t) = p.target.and_then(|id| picture.plane(id)) {
                    let aim = t.pos + t.vel * p.skill.lead;
                    plan = (Primitive::Pursue, t.id, horiz(aim), -aim.z, 0.0);
                }
            } else {
                p.target = None;
                if let Some(c) = chaser.filter(|c| c.pos.distance(me.pos) < p.skill.awareness) {
                    plan = (Primitive::Evade, c.id, break_point(me, &c, centre, g.radius), low, thick);
                }
            }
            if plan.0 == Primitive::Band {
                let from = horiz(me.pos) - centre;
                let going_out = horiz(me.vel).dot(from) > 0.0;
                if from.length() > p.skill.edge * g.radius && going_out {
                    plan = (Primitive::Goto, 0, centre, mid, 0.0);
                }
            }
            p.plan_id = p.plan_id.wrapping_add(1).max(1);
            let (primitive, target, point, alt, radius) = plan;
            let (lat, lon) = picture.global(point.x, point.y);
            let alt = alt.clamp(g.floor, g.ceiling);
            orders.push(Order {
                plane: me.id,
                plan: WirePlan { id: p.plan_id, primitive, target, lat, lon, alt, radius, ttl: p.ttl },
            });
        }
        orders
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameView;
    use cognition_plan::to_local;

    const HOME: (f64, f64) = (-35.363261, 149.16523);

    /// A plane at (north, east, height) flying north at `speed`.
    fn craft(id: u8, n: f32, e: f32, h: f32, speed: f32) -> Craft {
        Craft { id, pos: Vec3::new(n, e, -h), vel: Vec3::new(speed, 0.0, 0.0), flying: true }
    }

    fn picture(it: u8, planes: Vec<Craft>) -> Picture {
        Picture { time: 10.0, home: HOME, planes, game: GameView { it, time_left: 100.0, ..Default::default() } }
    }

    fn order(orders: &[Order], id: u8) -> WirePlan {
        orders.iter().find(|o| o.plane == id).unwrap_or_else(|| panic!("no plan for {id}")).plan
    }

    fn local(p: &WirePlan) -> Vec2 {
        let (n, e) = to_local(HOME.0, HOME.1, p.lat, p.lon);
        Vec2::new(n, e)
    }

    #[test]
    fn the_chaser_pursues_the_plane_it_can_catch_soonest() {
        let mut brain = ScriptedBrain::computer_pilots();
        // 1 is it, at the middle flying north. 2 is 40 m ahead and slow; 3 is 30 m behind it.
        let pic = picture(
            1,
            vec![craft(1, 70.0, 0.0, 25.0, 18.0), craft(2, 110.0, 0.0, 40.0, 12.0), craft(3, 40.0, 0.0, 52.0, 18.0)],
        );
        let orders = brain.decide(&pic);
        let p = order(&orders, 1);
        assert_eq!((p.primitive, p.target), (Primitive::Pursue, 2), "ahead and slow beats nearer but behind");
        assert_eq!(p.ttl, TTL);
        // Aimed where the target will be (the ace leads by 1.5 s), at its height.
        assert!((local(&p) - Vec2::new(110.0 + 12.0 * 1.5, 0.0)).length() < 0.5, "{:?}", local(&p));
        assert_eq!(p.alt, 40.0);
        assert_eq!(brain.target(1), Some(2));
    }

    #[test]
    fn the_chaser_sticks_with_its_target_unless_another_is_clearly_quicker() {
        let mut brain = ScriptedBrain::computer_pilots();
        let mut pic = picture(
            1,
            vec![craft(1, 70.0, 0.0, 25.0, 18.0), craft(2, 110.0, 0.0, 25.0, 15.0), craft(3, 120.0, 0.0, 25.0, 15.0)],
        );
        assert_eq!(order(&brain.decide(&pic), 1).target, 2);
        // 3 is now a little nearer: not enough to switch.
        pic.planes[1].pos.x = 125.0;
        assert_eq!(order(&brain.decide(&pic), 1).target, 2);
        // 2 has gone far away: switch.
        pic.planes[1].pos.x = 250.0;
        assert_eq!(order(&brain.decide(&pic), 1).target, 3);
    }

    #[test]
    fn the_chaser_does_not_go_for_a_tag_back_or_a_plane_on_the_ground() {
        let mut brain = ScriptedBrain::computer_pilots();
        let mut pic = picture(
            1,
            vec![craft(1, 70.0, 0.0, 25.0, 18.0), craft(2, 90.0, 0.0, 25.0, 15.0), craft(3, 200.0, 0.0, 25.0, 15.0)],
        );
        pic.game.no_tag_back = 2;
        assert_eq!(order(&brain.decide(&pic), 1).target, 3);
        pic.game.no_tag_back = 0;
        pic.planes[1].flying = false;
        pic.planes[2].flying = false;
        let orders = brain.decide(&pic);
        assert_eq!(order(&orders, 1).primitive, Primitive::Band, "nobody to chase: hold the band");
        assert_eq!(orders.len(), 1, "no plans for planes on the ground");
    }

    #[test]
    fn a_runner_evades_a_chaser_it_can_see_and_how_far_it_sees_is_its_skill() {
        let mut brain = ScriptedBrain::computer_pilots();
        // 3 (the rookie, sees 40 m) and 2 (club, 60 m) both 50 m from the chaser.
        let pic = picture(
            1,
            vec![craft(1, 70.0, 0.0, 30.0, 18.0), craft(2, 70.0, 50.0, 30.0, 18.0), craft(3, 70.0, -50.0, 30.0, 18.0)],
        );
        let orders = brain.decide(&pic);
        let club = order(&orders, 2);
        assert_eq!((club.primitive, club.target), (Primitive::Evade, 1));
        // It breaks away from the chaser (to the east, it is east of it) and keeps its band.
        let at = local(&club);
        assert!(at.y > 50.0 - 1.0 && (at - Vec2::new(70.0, 50.0)).length() > 50.0, "{at:?}");
        assert_eq!((club.alt, club.radius), (35.0, 10.0));
        let rookie = order(&orders, 3);
        assert_eq!(rookie.primitive, Primitive::Band, "the rookie hasn't seen it");
    }

    #[test]
    fn each_plane_holds_its_own_band_over_the_arena() {
        let mut brain = ScriptedBrain::computer_pilots();
        let pic = picture(
            0,
            vec![craft(1, 70.0, 0.0, 30.0, 18.0), craft(2, 60.0, 10.0, 30.0, 18.0), craft(3, 80.0, -10.0, 30.0, 18.0)],
        );
        let orders = brain.decide(&pic);
        let bands: Vec<(f32, f32)> = (1..=3).map(|id| (order(&orders, id).alt, order(&orders, id).radius)).collect();
        assert_eq!(bands, vec![(20.0, 10.0), (35.0, 10.0), (50.0, 8.0)]);
        for id in 1..=3 {
            let p = order(&orders, id);
            assert_eq!(p.primitive, Primitive::Band);
            assert!((local(&p) - Vec2::new(70.0, 0.0)).length() < 0.5, "centred on the arena");
        }
        // At least 15 m between bands, inside the floor and the ceiling.
        for w in BANDS.windows(2) {
            assert!(w[1].0 - w[0].0 >= 15.0);
        }
        assert!(BANDS.iter().all(|&(lo, th)| lo >= 12.0 && lo + th <= 60.0));
    }

    #[test]
    fn a_plane_heading_out_of_the_arena_is_sent_back_to_the_middle() {
        let mut brain = ScriptedBrain::computer_pilots();
        // 75 m north of the middle (radius 90), going north.
        let pic = picture(0, vec![craft(1, 145.0, 0.0, 25.0, 18.0)]);
        let p = order(&brain.decide(&pic), 1);
        assert_eq!(p.primitive, Primitive::Goto);
        assert!((local(&p) - Vec2::new(70.0, 0.0)).length() < 0.5);
        assert_eq!(p.alt, 25.0, "the middle of its band");
        // Coming back in: band again.
        let mut back = pic.clone();
        back.planes[0].vel = Vec3::new(-18.0, 0.0, 0.0);
        assert_eq!(order(&brain.decide(&back), 1).primitive, Primitive::Band);
    }

    #[test]
    fn plan_ids_increase_with_each_plan() {
        let mut brain = ScriptedBrain::computer_pilots();
        let pic = picture(0, vec![craft(1, 70.0, 0.0, 25.0, 18.0)]);
        let a = order(&brain.decide(&pic), 1).id;
        let b = order(&brain.decide(&pic), 1).id;
        assert!(a >= 1 && b == a + 1);
    }

    #[test]
    fn after_the_game_they_circle_over_the_arena() {
        let mut brain = ScriptedBrain::computer_pilots();
        let mut pic = picture(1, vec![craft(1, 70.0, 0.0, 25.0, 18.0), craft(2, 90.0, 0.0, 25.0, 15.0)]);
        pic.game.time_left = 0.0;
        let orders = brain.decide(&pic);
        assert!(orders.iter().all(|o| o.plan.primitive == Primitive::Orbit && o.plan.radius == 54.0));
    }

    #[test]
    fn its_plans_survive_the_wire() {
        let mut brain = ScriptedBrain::computer_pilots();
        let pic = picture(
            1,
            vec![craft(1, 70.0, 0.0, 30.0, 18.0), craft(2, 70.0, 50.0, 30.0, 18.0), craft(3, 150.0, 0.0, 30.0, 18.0)],
        );
        for o in brain.decide(&pic) {
            let back = WirePlan::from_command(&o.plan.to_command()).unwrap();
            assert_eq!(
                (back.id, back.primitive, back.target, back.ttl),
                (o.plan.id, o.plan.primitive, o.plan.target, o.plan.ttl)
            );
            assert!(
                (back.alt - o.plan.alt).abs() <= 0.125 && (back.radius - o.plan.radius).abs() <= 0.5,
                "{back:?} {o:?}"
            );
        }
    }

    #[test]
    fn tactics_set_one_plane_s_skill_band_and_ttl() {
        let t = Tactics { awareness: 25.0, band_low: 40.0, band_thickness: 6.0, ttl: 4.0, ..Tactics::of(&ACE) };
        let mut brain = ScriptedBrain::with_tactics(1, &t);
        assert_eq!(brain.band(1), Some((40.0, 6.0)));
        // The chaser 30 m away: beyond 25 m, so the plane holds its band instead of evading.
        let pic = picture(2, vec![craft(1, 70.0, 0.0, 42.0, 18.0), craft(2, 70.0, 30.0, 42.0, 18.0)]);
        let p = order(&brain.decide(&pic), 1);
        assert_eq!((p.primitive, p.ttl), (Primitive::Band, 4.0));
        assert_eq!(Tactics::of(&ACE).to_array().len(), Tactics::LEN);
        assert_eq!(Tactics::from_array(&Tactics::of(&ACE).to_array()), Some(Tactics::of(&ACE)));
        let mut wild = t.to_array();
        wild[0] = 1.0e6;
        assert_eq!(Tactics::from_array(&wild), None);
    }
}
