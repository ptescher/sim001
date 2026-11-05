# Towers v3: scripts, layouts, kinds, a fight in the air

Status: a design, not built. Against v2 (`plans/towers-rules-v2.md`, adopted numbers in
`plans/towers-rules-v2-results.md`). Numbers are targets to check in the arena.

## 1. Why

In v2 the one real choice is "press on or leave" for a drone in a radio shadow, and the fixed
policy `persist` still wins (lead 0.11), games end at 45 % of the clock, and a drone's class is
only a weight (heavy 2, light 0.5). Nothing beats anything. A good game has several
dimensions, and in each one every option wins against one thing and loses to another.

Four changes, in this order (each is useful alone):

1. **Scripts**: a captain sends a drone a *sequence*, not one order. It is cut off from the
   radio, so it carries the answer with it.
2. **A fight in the air**: classes that beat each other.
3. **Tower kinds**: towers that ask for different things.
4. **Layouts**: maps where the same rules give different games.

## 2. Scripts (long plans)

An order is today `Take(i) | Goto(p) | Hold | Home`, plus one `Standing {after, then}` fallback
(`recreation_towers::Instruction`). A **script** replaces both:

```text
Script = [Step], at most 8 steps
Step   = { order: Order, until: Until }
Until  = Time(s)        -- fly the order for s seconds, then the next step
       | Held           -- until the tower of the order is held by the team (Take only)
       | Lost           -- until the tower is lost, or the drone is down
       | Silent(s)      -- until s seconds out of contact (the old standing order)
       | Never          -- the last step
```

Example, what the user asked for: `[ {Take(centre), Time(30)}, {Take(west), Never} ]`:
take the centre, loiter 30 s, then the west. Another: `[ {Take(west), Held}, {Home, Never} ]`.

Rules:

- A new order from the commander **replaces the whole script**, at any time, in contact. A
  captain overrides by sending one. (A drone out of contact cannot be overridden: that is the
  point of the shadow.)
- The `Silent(s)` step is the old standing order. `Instruction::give(order, standing)` stays as
  the one-step case, so v2's specs and the captains keep working.
- The failsafe is unchanged: with no script step that covers silence, link lost (45 s) or
  bingo battery flies home. A script's last step has `Never`, so it does not fly home unless the
  script says so: the commander takes the risk.
- The drone reports its step in telemetry (`step`, `of`), and the map shows the script's path.

What a captain must now decide: how far ahead to plan (a longer script is safer in a shadow
and worse when the fight changes), and which steps to trust to `Held` versus `Time`.

Build: `Instruction` becomes `{ script: Vec<Step>, at: usize, since: f32 }`; `tick` advances
it; the seat's JSON reply (`[drone, order]`) accepts an optional `script` as a list; `Prepared`
and the league's `CommanderTuning` get `script_len` and `loiter` as tunables.

## 3. A fight in the air (a cycle, not a number)

Today drones never touch. New: a drone **engages** another within **gun range 40 m**, when it
is inside the other's rear **cone** (45° half-angle from the other's heading), for 1 s, takes
**damage** (hit points: light 40, standard 60, heavy 100; a gun does 20 a second). A drone at 0
is **down** (like the battery rule: back at its base after 100 s).

The cycle (three classes, each beats one and loses to one):

| Class | Beats | Loses to | Why, in rules |
| --- | --- | --- | --- |
| **Heavy** | Standard | Light | Heavy has armour (damage taken ×0.5) and a forward gun (cone 60°), so a standard cannot get behind it in time. A light is too agile for its gun: turn rate 3×, gun cone 20° |
| **Standard** | Light | Heavy | Speed 1.5× light: it runs a light down on a straight line. A light cannot turn into it |
| **Light** | Heavy | Standard | Gets inside a heavy's turn radius and onto its rear |

A tower circle contest then asks who is in it: three lights can take down a heavy that sits on
the tower, and one standard breaks them.

Two more axes, each a smaller cycle, so no dimension is a plain "more is better":

- **Height**: a higher drone has an energy edge (a hit from above does ×1.5, being above gives
  +turn rate for a pass), and costs battery (thin air, 1.3 s a second above 80 m). A low drone is
  hidden in the trees (v1 rule) and cannot be hit from above in forest lanes.
- **Stance**: **Aggressive** (guns hot, ×1.3 drain, can be seen at 200 m), **Wary** (guns cold
  unless attacked, normal), **Dark** (radio and guns off, invisible beyond 60 m, no points from
  towers). Aggressive beats Wary in a fight, Wary beats Dark (it sees them), Dark beats
  Aggressive (an ambush before it can turn).

Order gets a stance: `Take(i)` and `Goto(p)` carry one.

What the arena must show: no class or stance wins at 3 standard errors against the other two
(the same check as v2's `persist` rule).

## 4. Tower kinds

Today: fort (home), spire (centre, 2 points), depot (flank, recharges). New kinds, each a
different price for what it gives:

| Kind | Gives | Costs |
| --- | --- | --- |
| **Fort** | Respawn point, 1 point a second | Fixed place, far from anything |
| **Spire** | 2 points a second, wide sight | Widest radio shadow, the whole game fights for it |
| **Depot** | Recharges, 1 point | Small shadow, easy to surface from |
| **Radar** (new) | Sight 400 m for its holder, **no shadow** | 0 points; an enemy near it is seen |
| **Jammer** (new) | **Shadow 220 m** | 0 points; holder cannot see its own drones in it either |
| **Flak** (new) | Hits enemy drones in its circle for 10 hp a second | A held flak tower is a trap for the attacker, and the holder is cut off from his side while taking it |
| **Hangar** (new) | A forward respawn point (back in 30 s, not 100) | A hangar lost is a respawn point for the enemy |

The kind is fixed by the map. A captain's choice is which kind to take first: radar to see,
jammer to deny, flak to stop a rush, hangar to stay in the fight.

## 5. Layouts

`Map.towers` is `[Vec2; 5]` and `Rules` has the distances as numbers. A layout is **data**: a
list of `{ name, kind, at }` and the two bases. Layouts, each with a different best strategy:

| Layout | Shape | What it favours |
| --- | --- | --- |
| **Valley** | Today's: 5 towers, three lanes | Baseline, balanced |
| **Ridge** | 7 towers in a line, a pass at each end | Flank rushes, fast hangar play |
| **Archipelago** | 9 small towers, no shadows overlap, lakes between | Light drones, radars and stance |
| **Crossroads** | A spire on a hill, 4 towers round it, one home each | Fights at the centre, the heavy |
| **Lopsided** | Blue has the depot, Red has the radar and a flak | Asymmetry: each side has a different plan |

A layout is a `.expect`-testable fixture: "On the Ridge, the first Blue drone to the nearest
tower arrives before the Red one".

## 6. Order of work (spec first, per the `tdd` skill)

1. **Scripts.** Specs in `expectations/09-towers/rules/09-standing-orders.expect` become
   `09-scripts.expect`: loiter then move, `Held` ends a step, a new order replaces the script,
   `Silent` is the old standing order. Then `Instruction`, the seat protocol, `Prepared`, the
   captain league's tunables.
2. **Layouts as data.** Required before kinds and new maps. No change in the game: the valley
   must score the same (the fast arena's 200-seed results as a regression check).
3. **Kinds.** Radar, jammer, flak, hangar, one at a time, each with its spec and a map that uses it.
4. **The fight.** Hit points, gun, cone, the class cycle. This one changes the most; measure
   the cycle with the `ruleset` example before it is adopted.
5. **Stance and height.**
6. **Captain league.** The Claude captains search the larger space (scripts, kind priority,
   class matchups); the check is that no fixed captain dominates.

Each step ends with `expectation check`, `cargo clippy --workspace --all-targets`, `cargo fmt`,
and its numbers added to `plans/towers-rules-v3-results.md`.

## 7. Open questions

- Is the fight a gun (as above), or the existing virtual touch of 8 m (as in the executor
  game)? The gun gives the cycle but means a firing model in the referee.
- How much does the script limit (8 steps) matter to the captain protocol on stdin?
- The real plane: a script is a chain of `Plan`s with a `ttl`, so it needs the C ABI `next`
  slot (`plans/edge.md`, step 3). Scripts can run on the server first.
