# Towers rules, as they are now (v1)

Source of truth: `domains/recreation/towers/src/lib.rs` (`Rules`), the specs in
`expectations/09-towers/rules/`, and the valley in `domains/recreation/towers/src/map.rs`.

## The game
Two teams, Blue (south) and Red (north), of 5 drones each: three standard (T1-S VTOL), one heavy
(T2 Cruza, counts double), one light (Seeker 3 quad, counts half). The game lasts 480 s and ends
early when a team reaches 500 points. Most points wins; equal is a draw.

## Towers and points
- Five towers: a home tower for each team (held at the start), a centre tower, and a west and an
  east flank tower. Home towers are 430 m from the centre, flank towers about 360 m, bases 520 m.
- A drone counts for a tower inside its **circle**: 60 m across, up to 120 m high.
- Each tower has a **control** from -100 % (Red) to +100 % (Blue). It moves towards the team with
  more weight in the circle: 5 % a second per unit weight, so one standard drone takes a neutral
  tower in 20 s. The control moves only by the difference between the teams.
- A team **holds** a tower at its end of the bar, and keeps it until the control gets back to 0,
  so an attack on a held tower takes twice as long as on a neutral one.
- Each held tower gives 1 point a second. 500 points wins.

## Sight (fog)
- A team sees 150 m round each of its drones and its base, and 200 m round each tower it holds.
  The rest is fog. The map keeps each enemy drone and tower as last seen, with its age.
- The valley is forest (20-30 m trees) except three lanes (west past the lake, mid, east), the
  clearings round towers and bases, and the lake. A drone flying **low** (below the treetops) is
  seen only along open ground; a tower or a drone above the trees sees a low drone within 75 m.
- Tall mountains ring the field: the drones cannot fly over them (ceiling 100 m).

## Radio shadows
- A tower a team holds blocks the enemy's radio within 140 m, at all heights. A drone there is
  **out of contact**: no telemetry, no picture, no new orders, and the team's map keeps its last
  position.
- Out of contact for 20 s, the link is **lost** and the drone flies home by itself.
- A **standing order** (new) can go with an order: what the drone does after `after` s out of
  contact. With none, the 20 s failsafe stays.

## Control
Each team has a commander (a person or the computer) who gives each drone an order about once a
second: take a tower, go to a point, hold, or go home; high or low.

## What we measured (arena, 200 games)
An attacker is always inside the shadow of the tower it is taking (shadow 140 m > circle 60 m),
and the failsafe (20 s) equals one capture (20 s). Flying the order on beat the failsafe by
+0.74 +- 0.08. The drone has one decision that matters.

## Knobs that exist in `Rules`
field_radius 600, base_distance 520, home_distance 430, tower_distance 360, treetop 20, look_down 75,
drones 5, squad [Std, Std, Std, Heavy, Light], circle_radius 60, circle_ceiling 120, capture_time 20,
win_score 500, duration 480, jam_radius 140, link_timeout 20, ceiling 100, tower_vision 200,
drone_vision 150, base_vision 150.
