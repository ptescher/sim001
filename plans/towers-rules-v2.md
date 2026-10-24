# Towers rules, proposed (v2)

Status: a design, not built. Against v1 (`plans/towers-rules-v1.md`). Numbers are design targets
to be checked in the arena (section 7), not measurements.

## 1. Pitch and goals

v2 keeps the field, the five towers and the radio shadows, and adds one resource and four rules
that make a cut-off drone's choice depend on what only it knows. Every drone has a **battery**
that drains in flight, drains faster while enemy drones share its circle, and recharges only over
its own base or in the circle of a **depot** its team holds. The three tower kinds differ:
**forts** (homes) and the **spire** (centre, 2 points a second) cast wide shadows, **depots**
(flanks) cast small ones and recharge. Unfinished work at a tower **drifts back** when nobody
is there, so leaving a half-taken tower costs. A cut drone can **surface** through a friendly
drone in contact just outside the shadow (a one-hop **mesh**). A drone whose battery runs out is
**down** and comes back 100 s later. The commander still plays the game; the drone in the
shadow now has to choose between pressing on, waiting, surfacing, recharging and going home, and
the right answer depends on its own camera, its battery and where it is.

Goals:

- G1. **No fixed standing order dominates.** The best fixed policy in v1 (persist) beat the
  failsafe by +0.74. In v2 the best fixed policy should lead the next best by < 0.15, and a
  policy that reads on-board state should beat every fixed one (section 7).
- G2. **The on-board information is what decides.** Each rule is chosen so that a cut drone's
  best move turns on something the ground does not have:

  | Rule | Decision it creates | On-board information that settles it |
  | --- | --- | --- |
  | Contest drain | Press on or bail when a defender arrives | Own camera: enemy class and count in the circle. Own battery |
  | Drift back (decay) | Finish, wait nearby, or leave | Own camera: the bar. Time left in the game |
  | Depots recharge | Recharge at a depot or at the base | Battery, position, age of the last picture of the depot |
  | Mesh | Surface to report and get orders, or stay dark | Own camera: a friendly picket nearby; the bar (what leaving costs) |
  | Battery / down / bingo | Push past bingo or turn back | Battery (contest drain makes it unknowable on the ground) |
  | Link timeout 30 s > first half of a capture 20 s | Failsafe fires only when the attack is stalled | Own camera: is the control still moving? |

- G3. **Readable.** Five mechanics, each a sentence a commander can say. Shadows never overlap.
  The map, the towers' places and the SITL flight do not change.
- G4. **Testable.** One `Rules` field per number; each mechanic is a feature of `.expect` specs
  run by the real referee (`domains/recreation/towers/tests/acceptance.rs`).

A fact v1 hid: a held tower stops casting its shadow as soon as its control crosses 0. A lone
attacker on a held tower is cut only for the first half of the capture (20 s), so with the link
timeout at 30 s the failsafe pulls a drone off only when the attack has stalled. That makes
"is it stalled, and why" the question the drone answers, which is exactly camera information.

## 2. The ruleset

### The game
- KEEP: two teams, Blue (south) and Red (north), 5 drones each: three standard (weight 1), one
  heavy (weight 2), one light (weight 0.5).
- CHANGE: the game lasts **600 s** (v1 480) and ends early at **600 points** (v1 500). Most
  points wins; equal is a draw. Reason: room for two battery cycles per drone.

### Towers and points
- KEEP: five towers in the same places (section 4). Circle 60 m across, up to 120 m high.
- NEW: three **kinds** of tower. The kind is fixed by the map.

  | Kind | Towers | Shadow | Sight | Points/s | Recharges |
  | --- | --- | --- | --- | --- | --- |
  | Fort | Blue home, Red home | 160 m (CHANGE from 140) | 200 m (KEEP) | 1 (KEEP) | no |
  | Spire | centre | 200 m (NEW) | 250 m (NEW) | **2** (NEW) | no |
  | Depot | west, east | 120 m (NEW) | 200 m (KEEP) | 1 (KEEP) | yes, 3/s |

- KEEP: control from -100 % (Red) to +100 % (Blue), 5 % a second per unit of weight more
  (one standard takes a neutral tower in 20 s, a held one in 40 s). Equal weight with both teams
  present: contested, no movement.
- KEEP: a team holds at its end of the bar and keeps it until the control gets back to 0.
- CHANGE: **drift back.** A circle with no flying drone in it moves the control back 1 % a
  second: towards the holder's end if the tower is held, towards 0 if it is neutral. (v1: an
  empty circle keeps its control.) A contested circle does not drift.

### Battery (NEW)
- Each drone has a battery, counted in seconds of flight: light **150 s**, standard **240 s**,
  heavy **300 s**. It is full at the start and after a respawn.
- Flying uses 1 s a second. A drone on the ground uses nothing.
- **Contest drain:** in a tower's circle, a drone uses **0.5 s a second more per unit of enemy
  weight** in the same circle (a heavy enemy costs 1 s a second more, a light 0.25).
- **Recharge:** within **60 m** of its own base (the HOME orbit is inside), **10 s a second**.
  In the circle of a depot its team holds, when the circle is not contested, **3 s a second**
  (so +2 net while flying). Never at forts or the spire.
- **Down:** a drone whose battery reaches 0 is down where it is (out of every circle, sees
  nothing, relays nothing) and is back on the ground at its base, full, **100 s** later.
- The team sees a drone's battery in its telemetry, while in contact. Out of contact the map
  keeps the last reported value and its age.

### Sight (fog)
- KEEP: a team sees 150 m round each drone in contact and its base, the tower's sight round each
  tower it holds (200 m, the spire 250 m). Fog, last-seen positions with age, forest, lanes,
  low flight, look-down 75 m, mountains: all as v1.
- KEEP: a drone out of contact sends no picture. CLARIFY (not a referee change): its own
  camera still sees 150 m for itself; an on-board decision may use what it sees, including the
  control of a tower within 150 m and the class of each drone it sees.

### Radio
- CHANGE: a held tower blocks the enemy's radio within its kind's shadow (fort 160 m, spire
  200 m, depot 120 m), at all heights. No two shadows overlap (section 4).
- CHANGE: out of contact for **30 s** (v1 20), the link is lost.
- NEW: **mesh.** A drone in a shadow is in contact if a drone of its team that is flying, not
  down and **not in any enemy shadow** is within **70 m** of it. One hop only: a relayed drone
  does not relay. With orbit radius 40 m and every shadow at least 120 m, a drone circling a
  tower can never be relayed from outside the shadow (closest approach is 80 m); it must leave
  the circle to surface.
- KEEP: standing orders as v1 (`expectations/09-towers/rules/09-standing-orders.expect`).
- CHANGE, the default when there is no standing order: the drone flies home when its link is
  lost (30 s) **or** when it is out of contact and its battery falls to **bingo, 60 s**
  (enough to get home from the farthest point: 950 m at 16 m/s is 59 s), whichever is first.
  A standing order replaces both: the commander takes the risk.

### Control
- KEEP: one commander per team (person or computer), orders about once a second: take a tower,
  go to a point, hold, home; high or low; an optional standing order.
- CHANGE (computer commander): it sends a drone in contact home, or to a held depot that is
  nearer, when its battery is under bingo plus the time to get there. See section 7.

## 3. `Rules` fields

All `f32` unless noted. Energy is in seconds of flight, so drain is 1 per second and needs no
field.

| Field | v1 | v2 | Status | Reason |
| --- | --- | --- | --- | --- |
| field_radius | 600 | 600 | KEEP | SITL fence; map unchanged |
| base_distance | 520 | 520 | KEEP | |
| home_distance | 430 | 430 | KEEP | |
| tower_distance | 360 | 360 | KEEP | |
| treetop, look_down | 20, 75 | 20, 75 | KEEP | |
| drones, squad | 5, [S,S,S,H,L] | same | KEEP | |
| circle_radius, circle_ceiling | 60, 120 | 60, 120 | KEEP | orbit 40 m stays inside |
| capture_time | 20 | 20 | KEEP | first half of a held capture (20 s) < link timeout |
| win_score | 500 | 600 | CHANGE | spire scores 2; longer game |
| duration | 480 | 600 | CHANGE | two battery cycles; about 92 s wall time on SITL at 6.5x |
| jam_radius (fort) | 140 | 160 | CHANGE | room for a picket and the mesh rule; still clear of depots |
| link_timeout | 20 | 30 | CHANGE | between the first half (20 s) and the whole (40 s) of a held capture: fires only on a stall |
| ceiling | 100 | 100 | KEEP | |
| tower_vision | 200 | 200 | KEEP | forts and depots |
| drone_vision, base_vision | 150, 150 | 150, 150 | KEEP | |
| spire_jam_radius | - | 200 | NEW | the prize is hardest to attack in contact |
| depot_jam_radius | - | 120 | NEW | depots are the leaky towers, easy to surface from |
| spire_vision | - | 250 | NEW | holder sees attackers coming; gives fog value to scouting it |
| tower_score | (1, hard-coded) | 1 | NEW | points/s for a fort or depot |
| spire_score | - | 2 | NEW | breaks the "each holds two" draw; a reason to fight in the middle |
| decay_time | - | 100 | NEW | s for an empty circle to drift one unit of control (1 %/s); leaving costs |
| light_battery | - | 150 | NEW | a light cannot solo a deep attack (118 s round trip) |
| standard_battery | - | 240 | NEW | far fort and back with one 40 s capture leaves about 80 s |
| heavy_battery | - | 300 | NEW | |
| contest_drain | - | 0.5 | NEW | per unit of enemy weight in the circle; the only "combat" |
| base_recharge | - | 10 | NEW | standard fills in 24 s |
| depot_recharge | - | 3 | NEW | slower than base, but nearer the fight |
| pad_radius | - | 60 | NEW | HOME orbit (40 m) is inside |
| bingo | - | 60 | NEW | failsafe floor without a standing order |
| respawn_time | - | 100 | NEW | must exceed the trip home and refill (59 + 24 = 83 s), or dying is a free recharge |
| mesh_range | - | 70 | NEW | below 80 m: no relay into an orbit from outside any shadow |

New types outside `Rules`: `enum Kind { Fort, Spire, Depot }` on `Tower` (from the map's
index); `Game::battery: Vec<f32>`, `Game::back_at: Vec<Option<f32>>` (respawn time);
`OwnDrone::battery` and `battery_age`; `TowerView::kind`. `Instruction::tick` takes the battery
and `bingo` too.

## 4. Map

Unchanged from v1 (`map.rs`, `Map::new`); coordinates are (north, east) from the centre, m.

```text
                         N (+north)
            Red base (520, 0)
               |  90 m road
            Red home FORT (430, 0)          shadow 160
           /        |          \
  west DEPOT        |           \
  (45, -390)    centre SPIRE     east DEPOT
  shadow 120      (0, 0)         (-35, 330)
           \     shadow 200     shadow 120
            \       |           /
            Blue home FORT (-430, 0)        shadow 160
               |  90 m road
            Blue base (-520, 0)
```

Distances (m; seconds at 16 m/s):

| From \ to | Blue base | Red base | spire | west depot | east depot |
| --- | --- | --- | --- | --- | --- |
| Blue base | - | 1040 | 520 (33) | 687 (43) | 587 (37) |
| Red base | 1040 | - | 520 (33) | 615 (38) | 646 (40) |
| Blue fort | 90 (6) | 950 (59) | 430 | 615 | 515 |
| spire | 520 | 520 | - | 393 (25) | 332 (21) |

Shadow clearances: spire to east depot 332 m > 200 + 120; spire to west depot 393 m; depots to
forts 515 m and more. No point is in two shadows. Blue is nearer the east depot, Red the west:
the asymmetry of v1 stays (and is measured, section 7).

Deferred, not proposed: a 7-tower map with two relay towers. `Map::towers` is a fixed
`[Vec2; 5]` and the lanes, clearings, mountains, SVG, commander and app all index it; the
mechanics above do not need more towers.

## 5. Worked scenarios

Each scenario has a Blue drone in a shadow, with no orders. "Ground knows" is Blue's map.

### S1. Lone attacker at a depot, a defender arrives
Blue standard B1 takes the Red-held west depot (shadow 120). It entered the shadow 6 s before
reaching the circle and has been circling 12 s: control -100 % to -40 %. Battery 150. Ground
knows: B1 last seen 18 s ago, 120 m out, battery 168.

| B1's camera sees arriving | Effect if B1 stays | Right choice |
| --- | --- | --- |
| a Red light | net weight +0.5: 2.5 %/s, 0 % in 16 s, then in contact; drain 1.25/s | persist |
| a Red standard | stall at -40 %; both drain 1.5/s; Red's depot does not recharge (contested); B1 dark, failsafe due in 12 s | depends on battery race: B1 has 150; a defender fresh from base has about 200. Bail to the shadow edge or home |
| a Red heavy | control back at 5 %/s; B1 drains 2/s | leave at once; every second costs 2 s of battery and 5 % of bar |

No fixed `after`/`then` is right in all three rows. Persist wins row 1 and loses row 3; "home
after 5 s" wins row 3 and throws away row 1.

### S2. Stalled at the spire, a picket nearby
Blue standard B2 at the Red-held spire (shadow 200), control -60 %, contested by one Red
standard, battery 80, cut 15 s. B2's camera sees the Blue light B5 circling 210 m east of the
spire (outside the shadow, in contact).

- Persist: stall, battery hits bingo (60) in 13 s, then it is 33 s from base: it must leave
  then anyway, and the failsafe would fire at 30 s.
- Surface: fly east out of the circle to within 70 m of B5 (about 140 m from the spire, 6 s).
  In contact at once: the ground sees the stall, B2's battery and the defender, and can send the
  heavy. Cost: while B2 is away the Red standard pushes the control back 5 %/s (-60 % to -90 %
  in 6 s).
- Home: 33 s, safe, the attack is lost and drifts back.

Right choice: surface if the picket is there and the battery covers the round trip; home if
not. The ground cannot make this choice; it does not know about the stall.

### S3. Low battery: depot or base, from a stale picture
B3 is at the spire, still cut (stalled), battery 45, under bingo. The east depot is 332 m
(21 s) away, the base 520 m (33 s). Blue's last picture, frozen when B3 went dark 50 s ago,
shows the east depot held by Blue.

- Base: arrives with 12. Safe.
- Depot: arrives with 24 and recharges at +2/s net, back in the fight 35 s sooner. But if the
  depot fell in the last 50 s, B3 arrives inside a Red depot shadow with 24 s of battery, 587 m
  (37 s) from base: down, 100 s out.

Right choice depends on the age of the picture, whether B3's own camera saw Red drones heading
east, and on 45 versus the base trip. A model that learns "depot if picture age < 30 s or
battery > 60" is using exactly the state the ground lacks.

### S4. The clock
25 s left, Blue 590, Red 588, Blue holds 3 points/s against Red's 2. B4, battery 20, is
contesting the Red-held east depot (stalled, drain 1.5/s). Down costs nothing after the end;
leaving lets Red's defender recharge and free up. Right choice: stay to the end, below bingo,
against every battery rule. The game clock is on board.

### S5. The heavy as a battery killer
Blue's heavy H is cut at the Red fort (shadow 160) for 10 s; the Red fort was attacked first by
two Blue standards that went home. Control -30 % (drifting back 1 %/s while it was empty). H's
camera sees two Red standards in the circle: weight 2 against 2, a stall, and each Red drone
drains 1/s extra (H weighs 2) while H drains 1/s extra (2 x 0.5). H has 220, the Reds are
unknown. Staying ties two enemy drones and costs them 2 s a second each; it scores nothing but
may pull both home early, and the ground (which cannot see this) might be counting on it.
Right choice depends on the standing order's intent (pin or take) and H's battery margin to
bingo plus the 59 s trip home. Pinning is a legitimate job only the heavy can do cheaply.

## 6. Risks and degenerate strategies

| Risk | Why it might happen | What stops it |
| --- | --- | --- |
| Persist still dominates | v1's measurement | Contest drain and down make persisting against equal or heavier weight lose battery and 100 s; link timeout 30 s means failsafe only fires on a stall. Measured: G1 |
| Suicide as recharge | Down refills the battery | respawn 100 s > farthest trip home 59 s + refill 24 s; leaving at bingo is always cheaper |
| Turtling (every drone sits on own towers) | Defence is in contact, attack is dark | Forts and the spire do not recharge, so sitters must cycle; spire is worth 2, so a team that holds home + depot (2/s) loses to one that adds the spire. Measured: a turtle captain must lose to the default |
| Depot camping (sit on own depot forever, +2 net) | Depot recharge | It holds one tower with one drone, a fair trade; two attackers stall it (contested: no recharge) and drain it. Depot shadow is the smallest, so attackers surface easily |
| Jammer / picket camping | Mesh rewards a drone at the shadow edge | The picket scores nothing, uses 1 s a second and cannot relay into an orbit (70 m < 80 m). It is a commander's trade, not free |
| One drone holds everything | Held towers do not drift | Same as v1: holding needs nobody present, taking needs weight. Drift only punishes unfinished work |
| Spire snowball | 2 points + 250 m sight + 200 m shadow | Spire is 33 s from both bases, symmetrical; its holder must still cycle batteries. Measured: P(win given first spire hold) < 0.75 |
| Heavy pinning forever | Heavy drains defenders 1/s each | Heavy has 300 s and drains too; it cannot recharge at enemy towers; two standards out-weigh it at a depot |
| Map side bias | Depots are not mirror images (Blue nearer east, Red nearer west) | Each `compare` seed plays both sides; Blue-vs-Red win rate of default against default must be within 0.45-0.55 |
| SITL server breaks | Down is new from the referee side | Battery is a referee number, not the SITL battery; down reuses the server's crash path (`world.rs`, `RESPAWN` 15 s today) with `respawn_time` instead |

## 7. Test plan

### Specs (`expectations/09-towers/rules/`)
- CHANGE `02-fight.expect`: "An empty circle keeps its control" becomes "An empty circle drifts
  back 1 % each second" (neutral: towards 0; held: towards the holder); add "A contested circle
  does not drift".
- CHANGE `03-score.expect`: 600 points, 10 minutes; add "The spire gives two points each
  second".
- CHANGE `07-radio.expect`: 140 m to the shadow of each kind; 20 s to 30 s; add "A held tower
  that is taken back to 0 stops blocking at once" (true in v1 too, unspecified).
- CHANGE `09-standing-orders.expect`: lost link at 30 s; add "A drone with no standing order
  flies home at bingo" and "A standing order replaces bingo too".
- CHANGE `05-commander.expect`: add "The commander sends a drone home at bingo" and "... to a
  held depot when it is nearer".
- NEW `10-kinds.expect`: forts, spire, depot: shadow, sight, points, which recharge.
- NEW `11-battery.expect`: flying uses 1 s a second; on the ground nothing; contest drain per
  enemy weight (light 0.25, heavy 1); recharge at the base pad, at an own depot, not when
  contested, not at a fort; empty is down, out of circles, back full after 100 s; the map keeps
  a cut drone's last battery with its age.
- NEW `12-mesh.expect`: relayed through a friendly drone in contact within 70 m; not through a
  drone that is itself in a shadow; not two hops; not from a down drone.
- `section.expect`: link the three new features. The driver
  (`domains/recreation/towers/tests/acceptance.rs`) needs steps for battery, kind and mesh.

### Arena measurements (`cognition_towers::compare`, paired seeds, both sides)
`compare` must take a `Rules` (today it uses `Rules::default()`), and `TowersResult` gains
`downs: [usize; 2]` and `recharge_seconds: [f32; 2]`. A new example `ruleset` runs a round robin
(200 seeds, 400 games per pair).

Fixed standing-order policies (existing `Autonomy`): `failsafe`; `persist`; `hold after 5`;
`home after 10`; `home after 30`; `goto shadow edge after 5` (the commander precomputes the
nearest point 10 m outside the shadow: a fixed "surface").

On-board policies (the `Edge` trait of `plans/edge.md` step 2, run each second on a cut drone,
reading `EdgeObs`: own battery, own position, the control and kind of the tower it is at, enemy
and friendly drones its camera sees with class, time since contact, game clock, the frozen
picture and its age, the standing order):
- `bingo-persist`: persist until battery < bingo + time home, then home.
- `outweighed-bail`: persist while own weight > enemy weight seen in the circle, else go to the
  shadow edge; home at bingo.
- `surface-on-stall`: if the control has not moved for 5 s and a friendly drone is seen
  outside the shadow, fly to within 70 m of it; else as `outweighed-bail`.
- `full`: all of the above plus S3's depot rule and S4's clock rule.
- `blind-full`: `full` with the camera and battery replaced by the ground's last values (the
  control of the information value).

Success criteria (all at 200 seeds):
1. Persist against failsafe: margin < +0.30 (v1 +0.74).
2. No dominance among fixed policies: the best fixed policy's mean margin against the others
   leads the second best by < 0.15, and no fixed policy beats every other fixed policy at 3
   standard errors.
3. Headroom: `full` beats the best fixed policy by >= +0.20, at 3 standard errors.
4. Information value: `blind-full` keeps at most half of that headroom.
5. Health: per team per game 0.5 to 3 downs; drones dark 10 to 30 % of their flight time;
   median game lasts >= 60 % of the clock; default against default, Blue wins 45 to 55 %; a
   turtle captain loses to the default by > 0.3; P(win | first to hold the spire) < 0.75.

If 2 or 3 fails, tune in this order: contest_drain, link_timeout, mesh_range, decay_time, and
re-run. The v1 test `flying_the_order_on_beats_the_failsafe_by_a_clear_margin` becomes
`no_fixed_standing_order_wins_every_matchup` (30 seeds, a fast check of criterion 2).

### Implementation order (each step green: clippy, fmt, tests)
1. Add every new `Rules` field with values that reproduce v1 (decay off, infinite battery, mesh
   0, kinds all 140 m and 1 point); `Kind` on `Tower`. No behaviour change.
2. Tower kinds: per-kind shadow, sight and score; `10-kinds.expect`; 03 and 07 updated.
3. Drift back in `Game::capture`; 02 updated.
4. Battery: drain, base recharge, down and respawn in the referee; `11-battery.expect` (part);
   `OwnDrone::battery`.
5. Contest drain and depot recharge; rest of `11-battery.expect`.
6. Mesh in `Game::jam` (shadows first, then one relay pass); `12-mesh.expect`.
7. Bingo in `Instruction::tick`; 09 updated.
8. Arena: `Match` handles down (reset the `Flyer` at the base after `respawn_time`), result
   fields, `compare(rules, ...)`; commander battery management and a `CommanderTuning` margin;
   05 updated.
9. Switch `Rules::default()` to v2. Re-run the v1 table, then the round robin.
10. `Edge` trait, `EdgeObs`, the hand-written edge policies, criteria 3 and 4.
11. Server: referee down drives the crash-reset path; battery and kind in the seat protocol and
    the commander app. Run the SITL acceptance test.

## Unsure or not verified

- Tower coordinates are read from `Map::new` assuming `Vec2` is (north, east), as the doc
  comments say. `plans/towers.md` says "towers are 220 m apart"; with the current map they are
  330 m and more apart, so that note looks stale.
- The arena flies every class at the same cruise (16 m/s, +-5 %); only weight (and in v2,
  battery) tells classes apart there. On SITL the heavy and light fly differently; the battery
  numbers are not tied to the real planes' endurance.
- The server today makes a drone `Down` only from a SITL crash (`world.rs`). A referee-declared
  down (empty battery) needs a new path from the referee to the host. I did not check how the
  seat protocol would carry battery.
- No arena run was made for v2; every number is a starting point for step 9.
- Not checked: whether `CommanderTuning`'s current contest rule (give up after 10 s behind,
  stay away 30 s) still makes sense with contest drain.
