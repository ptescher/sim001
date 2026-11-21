//! What a team brings to the game (the v4 rules, `plans/towers-rules-v4.md` section 4.2): its
//! airframes and their payloads, chosen before the game from a budget and hidden from the enemy.
//! The first [`Rules::drones`] fly at the start; the rest are spares.

use crate::{Class, Rules};
use serde::{Deserialize, Serialize};

/// What a drone carries besides its camera.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Payload {
    /// The camera only: every drone has one.
    #[default]
    Camera,
    /// Tags an enemy drone near it, and counts for nothing in a tower's circle.
    Tagger,
    /// Relays its team's radio far.
    Relay,
    /// When switched on, a radio shadow that moves with it.
    Jammer,
}

impl Payload {
    pub const ALL: [Payload; 4] = [Payload::Camera, Payload::Tagger, Payload::Relay, Payload::Jammer];

    /// What it costs from the budget, over the airframe.
    pub fn cost(self) -> f32 {
        match self {
            Payload::Camera => 0.0,
            Payload::Tagger | Payload::Relay => 1.0,
            Payload::Jammer => 2.0,
        }
    }

    /// Whether an airframe of `class` can carry it: a tagger needs a light or a standard drone (a
    /// heavy one is too slow to catch anything); a relay and a jammer need the lift of a standard or
    /// a heavy drone.
    pub fn fits(self, class: Class) -> bool {
        match self {
            Payload::Camera => true,
            Payload::Tagger => class != Class::Heavy,
            Payload::Relay | Payload::Jammer => class != Class::Light,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Payload::Camera => "camera",
            Payload::Tagger => "tagger",
            Payload::Relay => "relay",
            Payload::Jammer => "jammer",
        }
    }
}

/// One airframe of a loadout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Airframe {
    pub class: Class,
    #[serde(default)]
    pub payload: Payload,
}

impl Airframe {
    pub fn new(class: Class, payload: Payload) -> Airframe {
        Airframe { class, payload }
    }

    pub fn plain(class: Class) -> Airframe {
        Airframe::new(class, Payload::Camera)
    }

    pub fn cost(self) -> f32 {
        self.class.cost() + self.payload.cost()
    }
}

/// A team's airframes, in order: the first [`Rules::drones`] fly, the rest are spares.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Loadout {
    pub airframes: Vec<Airframe>,
}

/// The most airframes a loadout may have.
pub const MAX_AIRFRAMES: usize = 8;

impl Loadout {
    /// The rules' squad, with cameras only and no spares: today's game.
    pub fn squad(rules: &Rules) -> Loadout {
        Loadout { airframes: rules.squad.iter().map(|&c| Airframe::plain(c)).collect() }
    }

    pub fn cost(&self) -> f32 {
        self.airframes.iter().map(|a| a.cost()).sum()
    }

    /// Whether the rules allow it: enough airframes to fly, no more than [`MAX_AIRFRAMES`], each
    /// payload on an airframe that can carry it, and within the budget (if the rules have one).
    pub fn check(&self, rules: &Rules) -> Result<(), String> {
        let n = self.airframes.len();
        if n < rules.drones {
            return Err(format!("{n} airframes cannot fill {} places", rules.drones));
        }
        if n > MAX_AIRFRAMES {
            return Err(format!("{n} airframes are more than {MAX_AIRFRAMES}"));
        }
        if let Some(a) = self.airframes.iter().find(|a| !a.payload.fits(a.class)) {
            return Err(format!("a {} drone cannot carry a {}", a.class.name(), a.payload.name()));
        }
        let cost = self.cost();
        if rules.budget > 0.0 && cost > rules.budget {
            return Err(format!("the loadout costs {cost}, more than the budget of {}", rules.budget));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Class::*;

    fn of(airframes: &[(Class, Payload)]) -> Loadout {
        Loadout { airframes: airframes.iter().map(|&(c, p)| Airframe::new(c, p)).collect() }
    }

    #[test]
    fn an_airframe_costs_its_class_and_its_payload() {
        assert_eq!(Airframe::plain(Light).cost(), 2.0);
        assert_eq!(Airframe::plain(Standard).cost(), 3.0);
        assert_eq!(Airframe::plain(Heavy).cost(), 4.0);
        assert_eq!(Airframe::new(Light, Payload::Tagger).cost(), 3.0);
        assert_eq!(Airframe::new(Standard, Payload::Relay).cost(), 4.0);
        assert_eq!(Airframe::new(Heavy, Payload::Jammer).cost(), 6.0);
    }

    #[test]
    fn todays_squad_costs_15() {
        assert_eq!(Loadout::squad(&Rules::default()).cost(), 15.0);
    }

    #[test]
    fn each_payload_fits_the_airframes_that_can_carry_it() {
        for c in [Light, Standard, Heavy] {
            assert!(Payload::Camera.fits(c));
        }
        assert!(Payload::Tagger.fits(Light) && Payload::Tagger.fits(Standard) && !Payload::Tagger.fits(Heavy));
        assert!(!Payload::Relay.fits(Light) && Payload::Relay.fits(Standard) && Payload::Relay.fits(Heavy));
        assert!(!Payload::Jammer.fits(Light) && Payload::Jammer.fits(Standard) && Payload::Jammer.fits(Heavy));
    }

    #[test]
    fn a_loadout_within_the_budget_with_enough_airframes_that_fit_is_allowed() {
        let rules = Rules::v4();
        let mut squad = Loadout::squad(&rules);
        assert!(squad.check(&rules).is_ok());
        squad.airframes.extend([Airframe::plain(Light); 2]);
        assert!(squad.check(&rules).is_ok(), "19 of 20");
    }

    #[test]
    fn a_loadout_over_the_budget_too_small_too_big_or_with_a_payload_that_does_not_fit_is_refused() {
        let rules = Rules::v4();
        assert!(of(&[(Standard, Payload::Camera); 7]).check(&rules).is_err(), "21 is over 20");
        assert!(of(&[(Light, Payload::Camera); 4]).check(&rules).is_err(), "four cannot fill five places");
        assert!(of(&[(Light, Payload::Camera); 9]).check(&Rules { budget: 0.0, ..rules }).is_err(), "nine is too many");
        let heavy_tagger = [
            (Heavy, Payload::Tagger),
            (Light, Payload::Camera),
            (Light, Payload::Camera),
            (Light, Payload::Camera),
            (Light, Payload::Camera),
        ];
        assert!(of(&heavy_tagger).check(&rules).is_err());
    }

    #[test]
    fn with_no_budget_any_cost_is_allowed() {
        let rules = Rules { budget: 0.0, ..Rules::v4() };
        assert!(of(&[(Heavy, Payload::Jammer); 8]).check(&rules).is_ok());
    }
}
