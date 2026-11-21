//! A captain that sends each drone to a fixed post, whatever happens: a *formation*. The
//! allocation of drones to towers is the captain's main decision, and formations are the pure
//! strategies of it: playing them against each other shows which allocations beat which.
//! A drone whose battery is low flies home to recharge and goes back to its post.

use crate::{Captain, Persist, Prepared};
use recreation_towers::map::LayoutKind;
use recreation_towers::{
    Advise, Airframe, Class, Commander, Loadout, Order, Phase, Posture, Queued, Rules, Standing, Team, TeamView, Unit,
};

/// Where a drone goes. The towers are numbered Blue home, west, centre, east, Red home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Post {
    Centre,
    West,
    East,
    /// The team's own home tower.
    OwnHome,
    /// The enemy's home tower.
    EnemyHome,
    /// The outposts of the chain on the team's own side of the field, and on the enemy's.
    OwnWest,
    OwnMid,
    OwnEast,
    FarWest,
    FarMid,
    FarEast,
    /// A hunter (v4, a drone with a tagger): it circles high over the centre, and goes for the
    /// nearest enemy drone its team sees now.
    Hunt,
}

impl Post {
    fn outpost(self, team: Team, lane: usize) -> usize {
        let own = matches!(self, Post::OwnWest | Post::OwnMid | Post::OwnEast);
        let side = if (team == Team::Blue) == own { 5 } else { 8 };
        side + lane
    }

    pub fn tower(self, team: Team) -> usize {
        match (self, team) {
            (Post::West, _) => 1,
            (Post::Centre, _) => 2,
            (Post::East, _) => 3,
            (Post::OwnHome, Team::Blue) | (Post::EnemyHome, Team::Red) => 0,
            (Post::OwnHome, Team::Red) | (Post::EnemyHome, Team::Blue) => 4,
            // The chain: Blue's outposts are towers 5 to 7, Red's 8 to 10, west, mid, east.
            (Post::OwnWest | Post::FarWest, _) => Post::outpost(self, team, 0),
            (Post::OwnMid | Post::FarMid, _) => Post::outpost(self, team, 1),
            (Post::OwnEast | Post::FarEast, _) => Post::outpost(self, team, 2),
            // A hunter's tower is the one it guards.
            (Post::Hunt, _) => 2,
        }
    }
}

/// A drone to each post: the team's squad is three standard drones, a heavy one and a light one.
pub struct Formation {
    pub name: &'static str,
    pub posts: [Post; 5],
    rules: Rules,
    recharging: [bool; 5],
    /// A drone with a low battery lands for a swap; if not, it circles over its base.
    swap: bool,
    /// Every drone is quiet: it needs no picture and no telemetry to fly to its post.
    quiet: bool,
}

impl Formation {
    pub fn new(name: &'static str, posts: [Post; 5], rules: Rules) -> Formation {
        Formation { name, posts, rules, recharging: [false; 5], swap: false, quiet: false }
    }

    /// The same, landing for a battery swap (or not).
    pub fn swapping(self, swap: bool) -> Formation {
        Formation { swap, ..self }
    }

    /// The same, with every drone quiet (or loud).
    pub fn hushed(self, quiet: bool) -> Formation {
        Formation { quiet, ..self }
    }
}

/// What a hunter at `at` does: go for the nearest enemy drone its team sees now, or circle high over
/// the centre tower.
fn hunt(view: &TeamView, at: glam::Vec2) -> Order {
    let quarry = view.enemies.iter().filter(|e| e.seen).min_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at)));
    match quarry {
        Some(e) => Order::Intercept(e.k),
        None => Order::Circle(view.towers[Post::Hunt.tower(view.team)].at, 60.0),
    }
}

/// A captain that brings its own loadout to the game.
pub struct Loaded<C> {
    pub captain: C,
    pub loadout: Loadout,
}

impl<C: Captain> Captain for Loaded<C> {
    fn loadout(&mut self, _rules: &Rules) -> Loadout {
        self.loadout.clone()
    }

    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        self.captain.decide(view)
    }

    fn posture(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Posture)> {
        self.captain.posture(view, orders)
    }

    fn standing(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Standing)> {
        self.captain.standing(view, orders)
    }

    fn queue(&mut self, view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Vec<Queued>)> {
        self.captain.queue(view, orders)
    }

    fn unit(&self, team: Team, k: usize) -> Box<dyn Unit> {
        self.captain.unit(team, k)
    }

    fn advised_by(&mut self, advisor: Box<dyn Advise>, delay: f32) -> bool {
        self.captain.advised_by(advisor, delay)
    }
}

impl Captain for Formation {
    fn posture(&mut self, _view: &TeamView, orders: &[(usize, Order)]) -> Vec<(usize, Posture)> {
        let p = if self.quiet { Posture::Quiet } else { Posture::Loud };
        orders.iter().map(|&(k, _)| (k, p)).collect()
    }

    fn decide(&mut self, view: &TeamView) -> Result<Vec<(usize, Order)>, String> {
        let base = self.rules.base(view.team);
        let mut orders = vec![];
        for d in view.drones.iter().filter(|d| d.linked && d.phase != Phase::Down && d.k < 5) {
            let full = self.rules.battery(self.rules.class(d.k));
            if full > 0.0 {
                let need = self.rules.bingo + d.at.distance(base) / 16.0 + 15.0;
                self.recharging[d.k] = if self.recharging[d.k] {
                    d.battery < 0.8 * full
                } else {
                    d.phase == Phase::Flying && d.battery <= need
                };
            }
            orders.push((
                d.k,
                match (self.recharging[d.k], self.swap) {
                    (true, true) => Order::Land,
                    (true, false) => Order::Home,
                    (false, _) if self.posts[d.k] == Post::Hunt => hunt(view, d.at),
                    (false, _) => Order::Take(self.posts[d.k].tower(view.team)),
                },
            ));
        }
        Ok(orders)
    }
}

/// Makes a new captain for each game.
pub type Make = Box<dyn Fn() -> Box<dyn Captain> + Send + Sync>;

/// The ways to play `rules`' layout that the arena compares: the computer commander, and fixed
/// formations (v3 steps 1c and 2, `plans/towers-rules-v3-results.md`). Each presses on when it is
/// out of contact.
pub fn formations(rules: Rules) -> Vec<(&'static str, Make)> {
    use Post::*;
    let formation = |name: &'static str, posts: [Post; 5]| -> (&'static str, Make) {
        (name, Box::new(move || Box::new(Prepared { captain: Formation::new(name, posts, rules), autonomy: Persist })))
    };
    let commander: (&'static str, Make) =
        ("commander", Box::new(move || Box::new(Prepared { captain: Commander::new(rules), autonomy: Persist })));
    if rules.finite_airframes {
        return v4_formations(rules);
    }
    match rules.layout {
        LayoutKind::Chain => vec![
            commander,
            formation("mass centre", [Centre; 5]),
            formation("balanced", [West, East, Centre, Centre, Centre]),
            formation("own line", [OwnWest, OwnMid, OwnEast, Centre, Centre]),
            formation("far line", [FarWest, FarMid, FarEast, Centre, Centre]),
            formation("raid", [FarWest, FarEast, EnemyHome, Centre, EnemyHome]),
            formation("spread", [OwnWest, OwnEast, FarMid, Centre, West]),
            formation("snipe", [FarWest, FarMid, FarEast, FarMid, FarWest]),
            formation("no centre", [OwnWest, OwnEast, West, East, FarMid]),
        ],
        LayoutKind::Valley => vec![
            commander,
            formation("centre rush", [Centre; 5]),
            formation("wings", [West, West, East, East, Centre]),
            formation("balanced", [West, East, Centre, Centre, Centre]),
            formation("no centre", [West, West, East, East, East]),
            formation("raid", [West, East, EnemyHome, Centre, EnemyHome]),
            formation("turtle", [OwnHome, West, East, OwnHome, West]),
            formation("heavy centre", [West, East, West, Centre, East]),
        ],
    }
}

/// The ways to play by the v4 rules: today's formations, each with two light spares and landing for
/// a swap; and, against them, the same with no spares, circling instead of swapping, or with a
/// second heavy drone. The squad is three standard drones, a heavy one and a light one (15 of the
/// budget of 20), and a formation's posts are by the place in it.
fn v4_formations(rules: Rules) -> Vec<(&'static str, Make)> {
    use Post::*;
    let squad = Loadout::squad(&rules).airframes;
    let spares = Loadout { airframes: [squad.clone(), vec![Airframe::plain(Class::Light); 2]].concat() };
    let bare = Loadout { airframes: squad.clone() };
    // Two heavy drones (the third standard one is a heavy one) and one light spare: 18.
    let heavies = {
        let mut a = squad.clone();
        a[2] = Airframe::plain(Class::Heavy);
        Loadout { airframes: [a, vec![Airframe::plain(Class::Light)]].concat() }
    };
    // A relay on the second standard drone and one light spare: 18.
    let relay = {
        let mut a = squad.clone();
        a[1] = Airframe::new(Class::Standard, recreation_towers::Payload::Relay);
        Loadout { airframes: [a, vec![Airframe::plain(Class::Light)]].concat() }
    };
    // A tagger on the light drone, and two light spares: 20.
    let hunter = {
        let mut a = squad.clone();
        a[4] = Airframe::new(Class::Light, recreation_towers::Payload::Tagger);
        Loadout { airframes: [a, vec![Airframe::plain(Class::Light); 2]].concat() }
    };
    // Taggers on the first standard drone and the light one, and one light spare: 19.
    let hunters = {
        let mut a = squad.clone();
        a[0] = Airframe::new(Class::Standard, recreation_towers::Payload::Tagger);
        a[4] = Airframe::new(Class::Light, recreation_towers::Payload::Tagger);
        Loadout { airframes: [a, vec![Airframe::plain(Class::Light)]].concat() }
    };
    let entry = |name: &'static str, posts: Option<[Post; 5]>, loadout: &Loadout, swap: bool| -> (&'static str, Make) {
        // A name that says "quiet" is a quiet formation.
        let quiet = name.contains("quiet");
        let loadout = loadout.clone();
        (
            name,
            Box::new(move || -> Box<dyn Captain> {
                match posts {
                    Some(posts) => Box::new(Loaded {
                        captain: Prepared {
                            captain: Formation::new(name, posts, rules).swapping(swap).hushed(quiet),
                            autonomy: Persist,
                        },
                        loadout: loadout.clone(),
                    }),
                    None => Box::new(Loaded {
                        captain: Prepared { captain: Commander::new(rules), autonomy: Persist },
                        loadout: loadout.clone(),
                    }),
                }
            }),
        )
    };
    let mut out = vec![
        entry("commander", None, &spares, false),
        entry("commander, no spares", None, &bare, false),
        entry("commander, relay", None, &relay, false),
    ];
    let posts: Vec<(&'static str, [Post; 5])> = match rules.layout {
        LayoutKind::Chain => vec![
            ("mass centre", [Centre; 5]),
            ("balanced", [West, East, Centre, Centre, Centre]),
            ("own line", [OwnWest, OwnMid, OwnEast, Centre, Centre]),
            ("far line", [FarWest, FarMid, FarEast, Centre, Centre]),
            ("raid", [FarWest, FarEast, EnemyHome, Centre, EnemyHome]),
            ("spread", [OwnWest, OwnEast, FarMid, Centre, West]),
            ("no centre", [OwnWest, OwnEast, West, East, FarMid]),
        ],
        LayoutKind::Valley => vec![
            ("centre rush", [Centre; 5]),
            ("wings", [West, West, East, East, Centre]),
            ("balanced", [West, East, Centre, Centre, Centre]),
            ("no centre", [West, West, East, East, East]),
            ("raid", [West, East, EnemyHome, Centre, EnemyHome]),
            ("heavy centre", [West, East, West, Centre, East]),
        ],
    };
    let find = |n: &str| posts.iter().find(|(m, _)| *m == n).map(|(_, p)| *p);
    out.extend(posts.iter().map(|&(name, p)| entry(name, Some(p), &spares, true)));
    out.push(entry("balanced, no spares", find("balanced"), &bare, true));
    out.push(entry("balanced, circles", find("balanced"), &spares, false));
    out.push(entry("raid, circles", find("raid"), &spares, false));
    out.push(entry("raid, two heavies", find("raid"), &heavies, true));
    out.push(entry("balanced, two heavies", find("balanced"), &heavies, true));
    out.push(entry("raid, relay", find("raid"), &relay, true));
    out.push(entry("raid, quiet", find("raid"), &spares, true));
    out.push(entry("balanced, quiet", find("balanced"), &spares, true));
    let with_hunter = |p: Option<[Post; 5]>| {
        p.map(|mut p| {
            p[4] = Hunt;
            p
        })
    };
    out.push(entry("balanced, hunter", with_hunter(find("balanced")), &hunter, true));
    out.push(entry("raid, hunter", with_hunter(find("raid")), &hunter, true));
    out.push(entry(
        "two hunters",
        find("balanced").map(|mut p| {
            (p[0], p[4]) = (Hunt, Hunt);
            p
        }),
        &hunters,
        true,
    ));
    out
}

/// Sets one rule from `name=number` (`layout=1` is the chain): the rules a table is played by.
pub fn set_rule(rules: &mut Rules, setting: &str) -> Result<(), String> {
    let (k, v) = setting.split_once('=').ok_or(format!("{setting}: want name=number"))?;
    let v: f32 = v.parse().map_err(|_| format!("{setting}: not a number"))?;
    match k {
        "spire_score" => rules.spire_score = v,
        "tower_score" => rules.tower_score = v,
        "contest_drain" => rules.contest_drain = v,
        "spire_jam_radius" => rules.spire_jam_radius = v,
        "win_score" => rules.win_score = v,
        "layout" => rules.layout = if v >= 1.0 { LayoutKind::Chain } else { LayoutKind::Valley },
        // The v4 rules as far as they are built, on the layout set so far.
        "v4" if v >= 1.0 => *rules = Rules { layout: rules.layout, ..Rules::v4() },
        _ => return Err(format!("no rule called {k}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;
    use recreation_towers::Game;

    fn view(team: Team) -> TeamView {
        let mut g = Game::new(Rules::default());
        g.look();
        g.view(team)
    }

    #[test]
    fn the_valley_has_the_commander_and_seven_formations_and_the_chain_eight() {
        let names = |rules| formations(rules).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        let valley = names(Rules::default());
        assert_eq!(valley.len(), 8);
        assert_eq!(valley[0], "commander");
        assert!(valley.contains(&"balanced"));
        let chain = names(Rules { layout: recreation_towers::map::LayoutKind::Chain, ..Rules::default() });
        assert_eq!(chain.len(), 9);
        assert!(chain.contains(&"far line"));
        let mut captain = formations(Rules::default())[1].1();
        assert_eq!(captain.decide(&view(Team::Blue)).unwrap().len(), 5, "a formation orders each drone");
    }

    #[test]
    fn the_v4_rules_set_by_name_keep_the_layout() {
        let mut r = Rules::default();
        set_rule(&mut r, "layout=1").unwrap();
        set_rule(&mut r, "v4=1").unwrap();
        assert_eq!(r, Rules { layout: LayoutKind::Chain, ..Rules::v4() });
    }

    #[test]
    fn with_the_v4_rules_every_way_to_play_brings_a_loadout_the_rules_allow() {
        for layout in [LayoutKind::Valley, LayoutKind::Chain] {
            let rules = Rules { layout, ..Rules::v4() };
            let all = formations(rules);
            assert!(all.len() > formations(Rules { layout, ..Rules::default() }).len(), "more ways to play");
            for (name, make) in &all {
                let loadout = make().loadout(&rules);
                assert!(loadout.check(&rules).is_ok(), "{name}: {:?}", loadout.check(&rules));
            }
            let spares = |n: &str| all.iter().find(|(m, _)| *m == n).map(|(_, f)| f().loadout(&rules).airframes.len());
            assert_eq!(spares("balanced"), Some(7), "two light spares");
            assert_eq!(spares("balanced, no spares"), Some(5));
        }
    }

    #[test]
    fn a_formation_that_swaps_lands_to_recharge_and_one_that_does_not_circles_over_its_base() {
        let rules = Rules::v4();
        let mut v = view(Team::Blue);
        v.drones[0].phase = Phase::Flying;
        v.drones[0].battery = 70.0;
        let mut swaps = Formation::new("test", [Post::Centre; 5], rules).swapping(true);
        assert_eq!(swaps.decide(&v).unwrap()[0].1, Order::Land);
        let mut circles = Formation::new("test", [Post::Centre; 5], rules);
        assert_eq!(circles.decide(&v).unwrap()[0].1, Order::Home);
    }

    #[test]
    fn a_quiet_formation_sends_each_order_quiet_through_its_wrappers() {
        let rules = Rules::v4();
        let all = formations(rules);
        let (_, make) = all.iter().find(|(n, _)| *n == "raid, quiet").expect("a quiet raid");
        let mut c = make();
        let v = view(Team::Blue);
        let orders = c.decide(&v).unwrap();
        assert!(c.posture(&v, &orders).iter().all(|&(_, p)| p == Posture::Quiet));
        let (_, loud) = all.iter().find(|(n, _)| *n == "raid").unwrap();
        let mut c = loud();
        assert!(c.posture(&v, &orders).iter().all(|&(_, p)| p == Posture::Loud));
    }

    #[test]
    fn a_hunter_guards_over_the_centre_and_goes_for_the_nearest_enemy_its_team_sees() {
        let rules = Rules::v4();
        let mut f = Formation::new("test", [Post::Hunt, Post::Centre, Post::Centre, Post::Centre, Post::West], rules);
        let mut v = view(Team::Blue);
        let centre = v.towers[2].at;
        assert!(matches!(f.decide(&v).unwrap()[0].1, Order::Circle(p, _) if p == centre), "it guards the centre");
        let enemy = |k, at: Vec2, seen| recreation_towers::EnemyDrone {
            k,
            class: None,
            payload: None,
            at,
            alt: 17.0,
            vel: Vec2::ZERO,
            seen,
            age: if seen { 0.0 } else { 30.0 },
        };
        v.enemies = vec![
            enemy(1, centre + Vec2::new(300.0, 0.0), true),
            enemy(2, centre, true),
            enemy(3, v.drones[0].at, false),
        ];
        assert_eq!(f.decide(&v).unwrap()[0].1, Order::Intercept(2), "the nearest it sees now");
    }

    #[test]
    fn the_v4_population_has_hunters_with_taggers() {
        let rules = Rules::v4();
        let all = formations(rules);
        for name in ["balanced, hunter", "raid, hunter", "two hunters"] {
            let (_, make) = all.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name}"));
            let l = make().loadout(&rules);
            assert!(l.airframes.iter().take(5).any(|a| a.payload == recreation_towers::Payload::Tagger), "{name}");
        }
    }

    #[test]
    fn a_captain_with_a_loadout_brings_it() {
        let rules = Rules::v4();
        let mut loadout = recreation_towers::Loadout::squad(&rules);
        loadout.airframes.push(recreation_towers::Airframe::plain(recreation_towers::Class::Light));
        let mut c = Loaded { captain: Formation::new("test", [Post::Centre; 5], rules), loadout: loadout.clone() };
        assert_eq!(c.loadout(&rules), loadout);
    }

    #[test]
    fn a_rule_is_set_by_its_name() {
        let mut r = Rules::default();
        set_rule(&mut r, "spire_score=1").unwrap();
        set_rule(&mut r, "layout=1").unwrap();
        set_rule(&mut r, "contest_drain=3").unwrap();
        assert_eq!((r.spire_score, r.layout, r.contest_drain), (1.0, LayoutKind::Chain, 3.0));
        assert!(set_rule(&mut r, "no_such_rule=1").is_err());
        assert!(set_rule(&mut r, "spire_score=lots").is_err());
        assert!(set_rule(&mut r, "spire_score").is_err());
    }

    #[test]
    fn each_drone_goes_to_its_post() {
        let posts = [Post::Centre, Post::West, Post::East, Post::Centre, Post::OwnHome];
        let mut f = Formation::new("test", posts, Rules::default());
        let orders = f.decide(&view(Team::Blue)).unwrap();
        let towers: Vec<_> = orders.iter().map(|&(_, o)| o).collect();
        assert_eq!(towers, vec![Order::Take(2), Order::Take(1), Order::Take(3), Order::Take(2), Order::Take(0)]);
    }

    #[test]
    fn the_outposts_of_each_side_are_the_other_way_round_for_the_other_team() {
        assert_eq!(Post::OwnWest.tower(Team::Blue), 5);
        assert_eq!(Post::OwnEast.tower(Team::Blue), 7);
        assert_eq!(Post::FarMid.tower(Team::Blue), 9);
        assert_eq!(Post::OwnWest.tower(Team::Red), 8);
        assert_eq!(Post::FarEast.tower(Team::Red), 7);
    }

    #[test]
    fn red_has_its_own_and_the_enemy_home_the_other_way_round() {
        let posts = [Post::EnemyHome, Post::OwnHome, Post::West, Post::West, Post::West];
        let mut f = Formation::new("test", posts, Rules::default());
        let orders = f.decide(&view(Team::Red)).unwrap();
        assert_eq!(orders[0].1, Order::Take(0));
        assert_eq!(orders[1].1, Order::Take(4));
    }

    #[test]
    fn a_drone_with_a_low_battery_goes_home_and_comes_back_when_full() {
        let mut f = Formation::new("test", [Post::Centre; 5], Rules::default());
        let mut v = view(Team::Blue);
        v.drones[0].phase = Phase::Flying;
        v.drones[0].battery = 70.0;
        assert_eq!(f.decide(&v).unwrap()[0].1, Order::Home);
        v.drones[0].battery = 100.0;
        assert_eq!(f.decide(&v).unwrap()[0].1, Order::Home, "not full yet");
        v.drones[0].battery = 240.0;
        assert_eq!(f.decide(&v).unwrap()[0].1, Order::Take(2));
    }
}
