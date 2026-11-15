//! The two roles that play Towers, each its own trait so that each can be a person or a computer
//! (plans/actors.md): the [`Command`], one for a team, and the [`Unit`], one for a drone. Neither
//! does I/O: the arena, the game server and a seat in another process all drive the same traits.
//!
//! A command sees what its seat gets, the team's map ([`TeamView`]), and answers with a
//! [`Directive`] for each drone it wants to change: an order and the queue that goes with it.
//!
//! A unit sees what its drone has on board ([`OnBoard`]): its own camera and battery, whether it
//! is in contact, and the [`Instruction`] it last received. It runs on every step of the drone's
//! loop, in contact or not. What it returns is the order the drone flies. A unit can read
//! nothing else: [`OnBoard`] has no way to another drone, or to the team's map.

use crate::{EdgeObs, Game, Instruction, Order, Phase, Queued, Team, TeamView, edge};
use std::cell::OnceCell;

/// What a command gives a drone: the order, and the queue for when it is out of contact.
#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    pub drone: usize,
    pub order: Order,
    pub queue: Vec<Queued>,
}

impl Directive {
    /// An order with no queue: out of contact, the drone flies home when its link is lost.
    pub fn new(drone: usize, order: Order) -> Directive {
        Directive { drone, order, queue: vec![] }
    }

    pub fn with_queue(drone: usize, order: Order, queue: Vec<Queued>) -> Directive {
        Directive { drone, order, queue }
    }
}

/// A team's commander: looks at the team's map and gives orders. One for each team.
pub trait Command {
    /// The directives for the drones whose order it changes. A drone it does not name goes on
    /// with what it was told.
    fn decide(&mut self, view: &TeamView) -> Result<Vec<Directive>, String>;
}

impl<C: Command + ?Sized> Command for &mut C {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<Directive>, String> {
        (**self).decide(view)
    }
}

impl<C: Command + ?Sized> Command for Box<C> {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<Directive>, String> {
        (**self).decide(view)
    }
}

impl Command for crate::Commander {
    fn decide(&mut self, view: &TeamView) -> Result<Vec<Directive>, String> {
        Ok(crate::Commander::decide(self, view).into_iter().map(|(k, o)| Directive::new(k, o)).collect())
    }
}

/// What a drone knows on board, and no more. The fields are private, so a unit cannot get at the
/// referee's game, the other drones or the team's map.
pub struct OnBoard<'a> {
    game: &'a Game,
    i: usize,
    obs: OnceCell<EdgeObs>,
}

impl<'a> OnBoard<'a> {
    /// What drone `i` has on board in `game`.
    pub fn new(game: &'a Game, i: usize) -> OnBoard<'a> {
        OnBoard { game, i, obs: OnceCell::new() }
    }

    pub fn team(&self) -> Team {
        self.game.drones[self.i].team
    }

    /// Which of the team's drones this is, from 0.
    pub fn number(&self) -> usize {
        self.i % self.game.rules.drones
    }

    /// The drone is in the air (not on the ground, not down).
    pub fn flying(&self) -> bool {
        self.game.drones[self.i].phase == Phase::Flying
    }

    /// The drone's own clock (s since the game began).
    pub fn time(&self) -> f32 {
        self.game.time
    }

    /// How long it has been out of contact (s); 0 while it is in contact.
    pub fn silent(&self) -> f32 {
        self.game.silent[self.i]
    }

    pub fn in_contact(&self) -> bool {
        self.silent() <= 0.0
    }

    /// What is left of the battery (s); `None` in a game with no batteries.
    pub fn battery(&self) -> Option<f32> {
        self.game.charge(self.i)
    }

    /// How long out of contact before the link counts as lost (s).
    pub fn link_timeout(&self) -> f32 {
        self.game.rules.link_timeout
    }

    /// The battery (s) at which a drone with no plan flies home.
    pub fn bingo(&self) -> f32 {
        self.game.rules.bingo
    }

    /// Its camera, its position and the rest of [`EdgeObs`]. Worked out when first asked.
    pub fn obs(&self) -> &EdgeObs {
        self.obs.get_or_init(|| self.game.edge_obs(self.i))
    }

    /// Whether `order` is done, as the drone can tell from its own camera and position.
    pub fn done(&self, order: Order) -> bool {
        edge::order_done(self.obs(), self.team(), order)
    }
}

/// A drone's own brain. It runs on each step of the drone's loop (ten a second in the arena),
/// whether the drone is in contact or not, and keeps what state it likes.
pub trait Unit {
    /// One step. `told` is what the drone last received from its commander, and how far down the
    /// queue it is; the unit may change it (the drone's memory is its own). Returns the order the
    /// drone flies now: `None` while it has none.
    fn fly(&mut self, on: &OnBoard, told: &mut Instruction) -> Option<Order>;

    /// The drone is back on the ground after it went down: it forgets what it was doing.
    fn restart(&mut self) {}
}

impl<U: Unit + ?Sized> Unit for Box<U> {
    fn fly(&mut self, on: &OnBoard, told: &mut Instruction) -> Option<Order> {
        (**self).fly(on, told)
    }

    fn restart(&mut self) {
        (**self).restart()
    }
}

/// The default unit: flies the order it has, works down the queue when it is out of contact, and
/// flies home when its link is lost or its battery is low and the queue has nothing for it. This is
/// [`Instruction::tick_with`], the rule the flight controller and the server fly.
#[derive(Clone, Copy, Debug, Default)]
pub struct Autopilot;

impl Unit for Autopilot {
    fn fly(&mut self, on: &OnBoard, told: &mut Instruction) -> Option<Order> {
        told.tick_with(on.silent(), on.link_timeout(), on.battery(), on.bingo(), |o| on.done(o));
        told.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rules, Standing};
    use glam::Vec2;

    fn game() -> Game {
        let mut g = Game::new(Rules::default());
        g.look();
        g
    }

    #[test]
    fn the_autopilot_flies_the_order_it_was_told_in_contact() {
        let g = game();
        let mut told = Instruction::default();
        told.give(Order::Hold, None);
        assert_eq!(Autopilot.fly(&OnBoard::new(&g, 0), &mut told), Some(Order::Hold));
    }

    #[test]
    fn the_autopilot_works_down_the_queue_out_of_contact_and_then_flies_home() {
        let mut g = game();
        let mut told = Instruction::default();
        told.give(Order::Hold, Some(Standing { after: 5.0, then: Order::Goto(Vec2::ZERO) }));
        g.silent[0] = 4.0;
        assert_eq!(Autopilot.fly(&OnBoard::new(&g, 0), &mut told), Some(Order::Hold));
        g.silent[0] = 5.0;
        assert_eq!(Autopilot.fly(&OnBoard::new(&g, 0), &mut told), Some(Order::Goto(Vec2::ZERO)));
        // With nothing queued, a lost link is the failsafe.
        let mut bare = Instruction::default();
        bare.give(Order::Hold, None);
        g.silent[0] = g.rules.link_timeout;
        assert_eq!(Autopilot.fly(&OnBoard::new(&g, 0), &mut bare), Some(Order::Home));
    }

    #[test]
    fn on_board_is_what_the_drone_knows() {
        let mut g = game();
        g.silent[3] = 7.0;
        let on = OnBoard::new(&g, 3);
        assert_eq!((on.team(), on.number()), (Team::Blue, 3 % g.rules.drones));
        assert!(!on.in_contact());
        assert_eq!(on.obs().silent, 7.0);
        assert_eq!(on.battery(), g.charge(3));
        let ground = OnBoard::new(&g, 0);
        assert!(ground.in_contact() && !ground.flying());
    }

    #[test]
    fn a_command_is_a_commander_that_answers_with_directives() {
        let g = game();
        let mut c = crate::Commander::new(Rules::default());
        let ds = Command::decide(&mut c, &g.view(Team::Blue)).unwrap();
        assert!(!ds.is_empty() && ds.iter().all(|d| d.queue.is_empty()));
    }
}
