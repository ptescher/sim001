# Towers v3: information, roles, scripts

Status: a design, not built. Against v2 (`plans/towers-rules-v2.md`; adopted numbers in
`plans/towers-rules-v2-results.md`). Revised after an independent review (PR 65). Numbers are
targets to check in the arena.

## 1. Where v2 stands, and the goal

Adopted v2 meets its fixed-policy targets: no policy beats every other at 3 standard errors,
`persist` leads the failsafe by +0.11 ± 0.06. Two things are open:

- **Games end at 45 % of the clock** (target 60 %).
- **Criteria 3 and 4 were never measured**: the headroom of a policy that reads on-board
  state over the best fixed one, and the value of that information. `EdgeObs` and `Edge`
  (`plans/edge.md`) are not built, so v3 starts there.

Goal: a game that is **complex and fun and physically believable**. Complex and fun means
several dimensions, in each of which every option beats one thing and loses to another, and
no option is a plain "more is better". Believable means every rule is something a real small
drone fleet has (radio, GPS, battery, terrain, direction finding, jamming, interceptors), the
differences between airframes come out of SITL, and a script maps onto ArduPilot mission
items so it can fly on a real plane.

## 2. What we cut from the first draft, and why

| Cut | Reason |
| --- | --- |
| Guns, hit points, rear cones | A captain never flies a dogfight (orders are Take/Goto/Hold/Home): who is behind whom is decided by orbit phase, not choice. Not realistic for small drones either |
| Class cycle by turn rate and speed multipliers | These come from the airframes in SITL, not from the referee; the arena flies every class at one speed and could not measure it |
| Stance Aggressive/Wary/Dark | Dark with guns off cannot ambush; Aggressive dominates Wary |
| Hangar, a radar nothing can cut, flak by the spire | Snowball: the side ahead gets further ahead |
| `[Take(x), Never]` as a script | It is `persist` with the failsafe off |
| Layouts Ridge, Archipelago, Crossroads | Tower indices are in about 150 places and the terrain, trees, lanes, assets and SITL fence are all the valley's. Kept: Valley, and Lopsided (kinds swapped on the same geometry) |

## 3. Order of work (spec first, per the `tdd` skill)

1. **`EdgeObs` and a hand-written `Edge`** (`plans/edge.md` steps 1 and 2), and game length
   (target 60 %). This is the vocabulary for everything below, and it answers criteria 3 and 4
   before anything else is built.
2. **Scripts** (section 4).
3. **Emission control and direction finding** (section 5): the information dimension.
4. **The role cycle** (section 6), after SITL has measured the airframes (section 7).
5. **Sites**: the counter-drone jammer, the battery-swap pad (section 8).
6. **Lopsided layout** (kinds swapped on the valley), then the captain league searches the space.

Each step ends with `expectation check`, `cargo clippy --workspace --all-targets`,
`cargo fmt`, and its numbers in `plans/towers-rules-v3-results.md`.

## 4. Scripts: steps and interrupts

A script is a short list of **steps**, with **interrupts** that stand over the steps.

```text
Script    = { steps: [Step] (at most 8), interrupts: [Interrupt], version }
Step      = { order: Order, until: Until, timeout: s }        timeout is required
Until     = Time(s) | Arrived | Battery(<x s) | ContactRegained | EnemySeen | Outweighed
          | Never                                             (the last step only)
Interrupt = { when: Silent(s) | Bingo | Outweighed | EnemySeen, then: Order | JumpTo(step) }
```

- **Only conditions the drone can see on board** (its camera of 150 m, its battery, its
  clock, its position, its link). `Held` and `Lost` are not allowed: a drone in a shadow does
  not know them. Everything is the same vocabulary as `EdgeObs`.
- **Interrupts are not steps.** A drone that goes dark during step 1 still has its `Silent`
  handler. The old `Standing {after, then}` is the interrupt `Silent(after) -> then`, so v2's
  specs still hold.
- **Bingo is a hard failsafe.** An interrupt may change what the drone does at bingo (go to a
  depot instead of home), but the drone is never without one. (v2 lets a standing order
  replace bingo; that changes.)
- **Override and resume.** A new order from the commander replaces the script, in contact. It
  may say `resume`: fly the new order, then continue the old script at its step.
- **A loop** (`JumpTo`) is allowed, for a patrol.
- **An upload is acknowledged and versioned.** The old script stays until the drone confirms
  the new one. A short **recall** (home, hold) gets through jamming closer to a jammer than a
  full script does (it burns through): the trade is a long script, planned ahead, against a
  short command that reaches farther.
- **Ground picture.** The ground predicts a silent drone's path from its script, and shows it
  as last seen with an age. Telemetry (`step`, `of`) is only sent in contact.
- **On the plane.** A step maps onto an ArduPilot mission item (`NAV_LOITER_TIME`,
  `NAV_WAYPOINT`, conditions, `DO_JUMP`); interrupts onto `FS_LONG_ACTN` and `BATT_FS_*`. The
  two-slot `next`/`arm_after` of the C ABI in `plans/edge.md` is not enough for 8 steps, so
  scripts run on the server and the arena first, and the plane gets a mission upload later.

## 5. Emission control: the information dimension

A drone either **emits** (radio on: it can be overridden and it sends a picture) or is
**silent**. Direction finding makes the choice real.

| Posture | Gives | Costs |
| --- | --- | --- |
| **Emit** | Orders, telemetry, a mesh hop | A direction finder bears on it at long range |
| **Silent** | Seen only optically, within 150 m | Runs its script alone; cannot be overridden or recalled; sends no picture |
| **Decoy** | A cheap drone that emits and counts for no weight | Draws the enemy's attention and range, flies a tower circle for nothing |

Cycle: Emit beats Silent (it can be coordinated and reports); Silent beats the DF
(invisible to it); Decoy beats the DF and Emit's attention (it looks like a drone to the
finder); a close look (optical, 150 m) beats Decoy. The commander's choice, per drone, is how
much to say.

**DF tower** (replaces the radar): detects *emitting* drones within 400 m, and bears on them
(a direction and an age, not a position); blind to silent ones. It has a terrain and canopy
shadow like every sensor: a mountain or forest masks it, and a low drone below the treetops
is not found. It has a radio shadow of its own like any tower (it is a kind, with a price in
points).

## 6. The role cycle (interception, not guns)

The fight is a **tag at 8 m** as in the executor game (plans/league.md), by an interceptor,
not a gun. Three roles, as a cycle that comes from airframe performance (section 7):

| Role | Beats | Loses to | Why |
| --- | --- | --- | --- |
| **Holder** (heavy, capture weight 2) | EW escort | Interceptor | Takes a tower and sits on it; the escort cannot dislodge it. The interceptor tags it: it is slow and cannot turn out |
| **EW escort** (standard, jams the interceptor's link and GPS in 80 m) | Interceptor | Holder | An interceptor that loses its link flies its failsafe home and loses its GPS fix; a holder does not care, it is already at its post |
| **Interceptor** (light, fast, short battery) | Holder | EW escort | Gets inside the holder's turn radius and tags it; the cost is its battery in the chase |

A tag knocks a drone down for a time (a fall to the ground, back after a battery swap, section
8): it is not killed. The check is the same as every cycle: no role dominates at 3 standard
errors, and the game still ends at about 60 % of the clock.

Layers that keep it from being one cycle only:

- **Height**: high gives a better link and sight, and is seen and jammed from farther; low
  gives masking, a poor link, and is slow in the trees. Wind during the game hurts the quad most.
- **Weather and wind** change during the game: a rule the real airframes already follow in SITL.

## 7. SITL first: where the class differences come from

Before the cycle is written, measure each airframe in SITL: cruise and top speed, turn radius,
climb, endurance, and its radio. The class differences are then **derived** from the
measurements (a table in `plans/towers-rules-v3-results.md`), and the fast arena is given per-class
speed and bank limits (today it flies all classes at one speed). A number in the cycle that
does not come from a measurement is not allowed.

## 8. Sites: costs, not snowballs

| Site | Gives | Costs, and why it does not snowball |
| --- | --- | --- |
| **Counter-drone jammer** | Cuts the link and GPS in 220 m, as SITL GPS denial (`SIM_GPS*_ENABLE`) plus link loss: the drone drifts, the EKF degrades, the failsafe flies it home | Scores no points; its holder cannot talk to its own drones in the circle |
| **Battery-swap pad** (forward) | A drone on the ground 30 s gets a full battery | The drone is on the ground and can be tagged; a **finite pool of spare airframes** per team, so a loss costs something |

A drone down is back at its base after a swap, not at a respawn timer, and the spare pool is
shared: the team that loses many drones runs out.

## 9. Arena checks (for every section above)

1. No option (script, role, posture) dominates at 3 standard errors.
2. P(win | first to hold the spire) below 0.75.
3. A long-script captain does not beat a short-order captain by more than 0.15.
4. The game lasts at least 60 % of the clock.
5. Headroom of an `Edge` policy over the best fixed one (criteria 3 and 4).
6. A script's drone is not stranded: with any script a drone is home or flying at bingo.

## 10. Open questions

- Is the interception a tag at 8 m (as in the executor game, so the two games can share code)
  or a net capture with a cost to the interceptor?
- How much of the EW (GPS, link) can SITL already model? `SIM_GPS*` yes; the jammer's link effect
  needs a switch in the server.
- Does a decoy cost a real drone (a cheap emitter on a quad)? It needs a price in the game's
  budget, not just a rule.
