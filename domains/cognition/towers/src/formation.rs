//! A captain that sends each drone to a fixed post, whatever happens: a *formation*. The
//! allocation of drones to towers is the captain's main decision, and formations are the pure
//! strategies of it: playing them against each other shows which allocations beat which.
//! A drone whose battery is low flies home to recharge and goes back to its post.

use crate::Captain;
use recreation_towers::{Order, Phase, Rules, Team, TeamView};

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
        }
    }
}

/// A drone to each post: the team's squad is three standard drones, a heavy one and a light one.
pub struct Formation {
    pub name: &'static str,
    pub posts: [Post; 5],
    rules: Rules,
    recharging: [bool; 5],
}

impl Formation {
    pub fn new(name: &'static str, posts: [Post; 5], rules: Rules) -> Formation {
        Formation { name, posts, rules, recharging: [false; 5] }
    }
}

impl Captain for Formation {
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
                if self.recharging[d.k] { Order::Home } else { Order::Take(self.posts[d.k].tower(view.team)) },
            ));
        }
        Ok(orders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_towers::Game;

    fn view(team: Team) -> TeamView {
        let mut g = Game::new(Rules::default());
        g.look();
        g.view(team)
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
