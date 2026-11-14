//! What a drone is told ahead of time: an order, and a queue of instructions for when it is out
//! of contact. A commander cannot give a drone an order in a radio shadow, so it prepares the
//! drone for it. The server and the fast arena fly the same rule.
//!
//! In contact, the commander replaces the queue with each new order. Out of contact, the drone
//! works down the queue: each [`Queued`] instruction has an order and the [`When`] that starts
//! it, and the drone takes them in turn. A [`Standing`] order is a queue of one instruction. A
//! drone with no queue flies home when its link is lost, or at a low battery: that is the failsafe.
//!
//! Only the first `Silent` instruction of a queue goes to the flight controller today: it has one
//! slot for a standing plan (`cognition_plan`). The whole queue is flown by the server for a drone
//! with no wing, and by the fast arena.

use crate::Order;
use serde::{Deserialize, Serialize};

/// The most instructions in a queue. A longer queue is cut to this many.
pub const MAX_QUEUE: usize = 4;

/// What a drone does when it has been out of contact for `after` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standing {
    pub after: f32,
    pub then: Order,
}

/// What starts an instruction of a queue.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum When {
    /// The drone has been out of contact this long (s).
    Silent(f32),
    /// The order before it is done, as the drone's own camera and position show.
    Done,
    /// The drone's battery is down to this much (s of flight).
    Battery(f32),
}

/// An instruction in a queue: the order, and what starts it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Queued {
    pub when: When,
    pub order: Order,
}

impl Queued {
    pub fn silent(after: f32, order: Order) -> Queued {
        Queued { when: When::Silent(after), order }
    }

    pub fn done(order: Order) -> Queued {
        Queued { when: When::Done, order }
    }

    pub fn battery(left: f32, order: Order) -> Queued {
        Queued { when: When::Battery(left), order }
    }
}

impl From<Standing> for Queued {
    fn from(s: Standing) -> Queued {
        Queued::silent(s.after, s.then)
    }
}

/// The queue that an order comes with: the queue itself, or, when it is empty, the old standing
/// order as a queue of one. Cut to [`MAX_QUEUE`].
pub fn queue_of(standing: Option<Standing>, mut queue: Vec<Queued>) -> Vec<Queued> {
    if queue.is_empty() {
        queue.extend(standing.map(Queued::from));
    }
    queue.truncate(MAX_QUEUE);
    queue
}

/// The order a drone flies, and the queue that goes with it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Instruction {
    pub order: Option<Order>,
    pub queue: Vec<Queued>,
    /// How many instructions of the queue the drone has started. It works on from there.
    #[serde(default)]
    pub taken: usize,
}

impl Instruction {
    /// A new order from the commander: it replaces the old order and its queue (a standing order
    /// is a queue of one).
    pub fn give(&mut self, order: Order, standing: Option<Standing>) {
        self.give_queue(order, standing.map(Queued::from).into_iter().collect());
    }

    /// The same with a queue. A queue of more than [`MAX_QUEUE`] instructions is cut.
    pub fn give_queue(&mut self, order: Order, queue: Vec<Queued>) {
        self.order = Some(order);
        self.queue = queue_of(None, queue);
        self.taken = 0;
    }

    /// The drone decides for itself, on board: its own order replaces the queue. The failsafe
    /// still holds.
    pub fn decide(&mut self, order: Order) {
        self.order = Some(order);
        self.queue.clear();
        self.taken = 0;
    }

    /// The first `Silent` instruction: what the flight controller has as its standing plan.
    pub fn standing(&self) -> Option<Standing> {
        first_silent(&self.queue)
    }

    /// The drone has been out of contact for `silent` s (0 in contact), and loses its link at
    /// `timeout` s. It starts the next instruction of its queue when that is due, and stays on its
    /// order, also when it is in contact again. With no queue, it flies home when its link is lost,
    /// or when its `battery` (s, `None` in a game with no batteries) gets down to `bingo`: enough
    /// to fly home. No order is done: see [`Instruction::tick_with`].
    pub fn tick(&mut self, silent: f32, timeout: f32, battery: Option<f32>, bingo: f32) {
        self.tick_with(silent, timeout, battery, bingo, |_| false)
    }

    /// The same, with `done`: whether an order is done, as the drone itself can tell (see
    /// `Game::order_done`). It is asked only when the next instruction waits for it.
    pub fn tick_with(
        &mut self,
        silent: f32,
        timeout: f32,
        battery: Option<f32>,
        bingo: f32,
        done: impl FnOnce(Order) -> bool,
    ) {
        if silent <= 0.0 {
            return;
        }
        if let Some(next) = self.queue.get(self.taken).copied() {
            let due = match next.when {
                When::Silent(after) => silent >= after,
                When::Done => self.order.is_some_and(done),
                When::Battery(left) => battery.is_some_and(|b| b <= left),
            };
            if due {
                self.order = Some(next.order);
                self.taken += 1;
            }
        }
        // The failsafe is for a drone with no plan for the shadow: none that waits for the
        // silence, and none started.
        let planned = self.taken > 0 || self.queue.iter().any(|q| matches!(q.when, When::Silent(_)));
        if !planned && (crate::link_lost(silent, timeout) || battery.is_some_and(|b| b <= bingo)) {
            self.order = Some(Order::Home)
        }
    }
}

fn first_silent(queue: &[Queued]) -> Option<Standing> {
    queue.iter().find_map(|q| match q.when {
        When::Silent(after) => Some(Standing { after, then: q.order }),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told(standing: Option<Standing>) -> Instruction {
        Instruction { order: Some(Order::Take(2)), queue: standing.map(Queued::from).into_iter().collect(), taken: 0 }
    }

    fn hold_after(after: f32) -> Option<Standing> {
        Some(Standing { after, then: Order::Hold })
    }

    #[test]
    fn a_drone_in_contact_keeps_its_order() {
        let mut i = told(hold_after(5.0));
        i.tick(0.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
    }

    #[test]
    fn the_standing_order_is_due_after_its_time_out_of_contact() {
        let mut i = told(hold_after(5.0));
        i.tick(4.9, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(5.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Hold));
    }

    #[test]
    fn with_no_standing_order_a_lost_link_flies_home() {
        let mut i = told(None);
        i.tick(19.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(20.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Home));
    }

    #[test]
    fn a_standing_order_replaces_the_failsafe_and_sticks_when_contact_returns() {
        let mut i = told(hold_after(5.0));
        i.tick(30.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Hold));
        i.tick(0.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Hold));
    }

    #[test]
    fn with_no_standing_order_a_drone_out_of_contact_flies_home_at_bingo() {
        let mut i = told(None);
        i.tick(5.0, 20.0, Some(61.0), 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(5.0, 20.0, Some(60.0), 60.0);
        assert_eq!(i.order, Some(Order::Home));
    }

    #[test]
    fn a_drone_in_contact_does_not_turn_back_at_bingo_and_no_battery_is_no_bingo() {
        let mut i = told(None);
        i.tick(0.0, 20.0, Some(10.0), 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(5.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Take(2)), "a game with no batteries has no bingo");
    }

    #[test]
    fn a_standing_order_replaces_bingo_too() {
        let mut i = told(hold_after(30.0));
        i.tick(10.0, 20.0, Some(5.0), 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
    }

    #[test]
    fn a_new_order_replaces_the_standing_order() {
        let mut i = told(hold_after(5.0));
        i.give(Order::Home, None);
        assert_eq!((i.order, i.standing()), (Some(Order::Home), None));
    }

    fn queue_of_three() -> Vec<Queued> {
        vec![Queued::silent(5.0, Order::Take(3)), Queued::done(Order::Take(1)), Queued::done(Order::Home)]
    }

    #[test]
    fn a_drone_works_down_its_queue_as_each_order_is_done() {
        let mut i = Instruction::default();
        i.give_queue(Order::Take(2), queue_of_three());
        i.tick_with(4.0, 20.0, None, 60.0, |_| true);
        assert_eq!(i.order, Some(Order::Take(2)), "the first is not due");
        i.tick_with(5.0, 20.0, None, 60.0, |_| false);
        assert_eq!(i.order, Some(Order::Take(3)));
        i.tick_with(6.0, 20.0, None, 60.0, |_| false);
        assert_eq!(i.order, Some(Order::Take(3)), "not done: the drone waits");
        i.tick_with(7.0, 20.0, None, 60.0, |o| o == Order::Take(3));
        assert_eq!(i.order, Some(Order::Take(1)));
        i.tick_with(8.0, 20.0, None, 60.0, |o| o == Order::Take(1));
        assert_eq!(i.order, Some(Order::Home));
    }

    #[test]
    fn a_drone_in_contact_does_not_take_an_instruction_and_keeps_its_place_in_the_queue() {
        let mut i = Instruction::default();
        i.give_queue(Order::Take(2), queue_of_three());
        i.tick_with(0.0, 20.0, None, 60.0, |_| true);
        assert_eq!((i.order, i.taken), (Some(Order::Take(2)), 0));
        i.tick(6.0, 20.0, None, 60.0);
        i.tick_with(0.0, 20.0, None, 60.0, |_| true);
        assert_eq!((i.order, i.taken), (Some(Order::Take(3)), 1));
    }

    #[test]
    fn an_instruction_can_wait_for_the_battery() {
        let mut i = Instruction::default();
        i.give_queue(Order::Take(2), vec![Queued::battery(80.0, Order::Hold)]);
        i.tick(2.0, 20.0, Some(81.0), 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(2.0, 20.0, Some(80.0), 60.0);
        assert_eq!(i.order, Some(Order::Hold));
        i.tick(2.0, 20.0, None, 60.0);
    }

    #[test]
    fn a_queue_of_more_than_four_is_cut_to_four() {
        let mut i = Instruction::default();
        i.give_queue(Order::Hold, vec![Queued::done(Order::Home); 6]);
        assert_eq!(i.queue.len(), MAX_QUEUE);
        assert_eq!(queue_of(None, vec![Queued::done(Order::Home); 9]).len(), MAX_QUEUE);
    }

    #[test]
    fn a_standing_order_is_a_queue_of_one_and_the_first_silent_instruction_is_the_standing_order() {
        let mut i = Instruction::default();
        let s = Standing { after: 5.0, then: Order::Hold };
        i.give(Order::Take(2), Some(s));
        assert_eq!(i.queue, vec![Queued::silent(5.0, Order::Hold)]);
        assert_eq!(i.standing(), Some(s));
        i.give_queue(Order::Take(2), queue_of_three());
        assert_eq!(i.standing(), Some(Standing { after: 5.0, then: Order::Take(3) }));
        assert_eq!(queue_of(Some(s), vec![]), vec![Queued::from(s)]);
        assert_eq!(queue_of(Some(s), queue_of_three()), queue_of_three(), "a queue wins over a standing order");
    }

    #[test]
    fn a_queue_with_no_instruction_for_the_silence_leaves_the_failsafe_until_one_starts() {
        let mut i = Instruction::default();
        i.give_queue(Order::Take(2), vec![Queued::done(Order::Take(1))]);
        i.tick(19.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Take(2)));
        i.tick(20.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Home), "the failsafe holds");
    }

    #[test]
    fn a_drones_own_decision_replaces_the_queue_and_the_failsafe_holds_again() {
        let mut i = Instruction::default();
        i.give_queue(Order::Take(2), queue_of_three());
        i.tick(6.0, 20.0, None, 60.0);
        i.decide(Order::Hold);
        assert_eq!((i.order, i.queue.len(), i.taken), (Some(Order::Hold), 0, 0));
        i.tick(20.0, 20.0, None, 60.0);
        assert_eq!(i.order, Some(Order::Home));
    }

    #[test]
    fn a_queue_survives_the_wire_and_an_old_standing_order_has_no_queue_there() {
        let q = queue_of_three();
        let line = serde_json::to_string(&q).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Queued>>(&line).unwrap(), q);
    }
}
