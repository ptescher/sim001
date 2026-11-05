# Towers v3: what the arena says

## Step 1: on-board policies against "fly the order on" (headroom, criteria 3 and 4)

`cargo run --release -p cognition_towers --example edge 400`: each policy plays `persist` (the
default commander, every order with the standing order "fly on") on both sides of 400 seeds, 800
games. A policy decides for a drone that is out of contact, once a second, from `EdgeObs` only.
Margin is the policy's mean score over persist's (0 is level).

| Policy | Margin | Won / lost | Drones down, per game |
| --- | --- | --- | --- |
| keep the order (the baseline) | +0.000 ± 0.044 | 400 / 400 | 1.70 |
| home when 1 enemy is near | −0.992 ± 0.036 | 130 / 669 | 0.37 |
| home when 2 enemies are near | −0.428 ± 0.044 | 279 / 521 | 0.73 |
| home when 3 enemies are near | −0.204 ± 0.045 | 340 / 460 | 1.20 |
| finish the capture, then return | +0.000 ± 0.044 | 400 / 400 | 1.70 |

Reading it:

- **No headroom yet.** Every retreat policy loses to persist, and the more cautious it is, the
  more it loses. Pressing on beats leaving, also when the enemy is in sight. Leaving saves
  drones (1.70 down a game to 0.37) and loses the game.
- **"Finish, then return" never runs.** A drone is out of contact only inside the shadow of an
  enemy tower, and a tower that the drone's team holds casts no shadow. By the time its tower is
  held, the drone is in contact again, so the policy has no cut-off drone to decide for.
- **What this says about the rules**, not about the policies: a cut-off drone has one choice that
  matters (press on or leave) and pressing on wins. The on-board information that the v3 plan
  wants to matter does not yet change the right answer. The rules need a cost to pressing on that
  the camera can see (sections 5 and 6 of `plans/towers-rules-v3.md`), before a model has
  anything to learn.
- `EdgeObs` has no friends and no own class: "outnumbered" counts enemies only. A policy that
  compares the enemy's weight with its own side's in the circle needs both.
