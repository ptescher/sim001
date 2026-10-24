//! What a drone is told ahead of time: an order, and a standing order for when it is out of
//! contact. A commander cannot give a drone an order in a radio shadow, so it prepares the drone
//! for it. The server and the fast arena fly the same rule.

use crate::Order;
use serde::{Deserialize, Serialize};

/// What a drone does when it has been out of contact for `after` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Standing {
    pub after: f32,
    pub then: Order,
}

/// The order a drone flies, and the standing order that goes with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Instruction {
    pub order: Option<Order>,
    pub standing: Option<Standing>,
}

impl Instruction {
    /// A new order from the commander: it replaces the old order and its standing order.
    pub fn give(&mut self, order: Order, standing: Option<Standing>) {
        self.order = Some(order);
        self.standing = standing;
    }

    /// The drone has been out of contact for `silent` s (0 in contact), and loses its link at
    /// `timeout` s. Its standing order, when it is due, becomes its order, and stays so. With no
    /// standing order, it flies home when its link is lost, or when its `battery` (s, `None` in a
    /// game with no batteries) gets down to `bingo`: enough to fly home.
    pub fn tick(&mut self, silent: f32, timeout: f32, battery: Option<f32>, bingo: f32) {
        if silent <= 0.0 {
            return;
        }
        match self.standing {
            Some(s) if silent >= s.after => self.order = Some(s.then),
            None if silent >= timeout - crate::ENDS || battery.is_some_and(|b| b <= bingo) => {
                self.order = Some(Order::Home)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn told(standing: Option<Standing>) -> Instruction {
        Instruction { order: Some(Order::Take(2)), standing }
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
        assert_eq!((i.order, i.standing), (Some(Order::Home), None));
    }
}
