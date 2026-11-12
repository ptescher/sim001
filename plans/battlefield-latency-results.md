# The battlefield keeps up with the camera: what was measured

The plan is `plans/battlefield-latency.md`. Issue: #95.

## Step 0: the numbers before any change

Measured on an Apple M4, with `sim001_command --expect --only "keeps up with the camera"` (a
practice game: the app, `towers_server` with ten ArduPlane SITL drones at 10 times real time,
and `towers_view`). The app logs each motion of the camera
(`battlefield: the camera moved …`, `teleoperation_battlefield::lag`), and the 3D view logs its
frame rate every 5 s while an app is connected (`logs/towers/view.log`).

**While the commander pans the camera for 3 s** (one run, 2026-11-11):

| | Measured | Target (step 1 to 3) | Go/no-go |
| --- | --- | --- | --- |
| New pictures while the camera moves | 1 in 3.1 s (0.3 a second) | 60 a second | 45 |
| How far the picture on screen trails the camera | 544 ms (median), 1728 ms (90 %) | 50 ms | 80 ms |

**The 3D view itself, with the game running:**

| When | Frames a second | ms a frame |
| --- | --- | --- |
| Loading the valley | 15 to 19 | 53 to 66 |
| The valley loaded, the app connected and asking for pictures | 8 to 11 | 92 to 123 |
| Pictures it sent the app a second (Blue's slot) | 2 to 9 | |

**Loading the valley:** 35 s when `towers_view` runs alone; 127 to 136 s in a game.

## What the numbers say

1. **The gate is the largest part, as expected.** One new picture in a 3 s pan is `steady_request`
   at work: no picture goes out while the camera moves. Step 1 removes it.
2. **The 3D view is slow on its own.** At 8 to 11 frames a second, a picture is about 100 ms
   old when it is rendered, before the readback, the copy, the TCP and the upload. Even with the
   gate gone, steps 1 to 3 cannot reach 50 ms, nor 45 pictures a second, unless the view renders
   faster. **Step 5 (a lighter 3D view) moves up**, to just after step 1. What it renders at once:
   its own window at vsync, the six drones' cameras the app watches, the battlefield, on the
   realism engine, while ten SITL drones and the server share the machine.
3. **The load time** (four times longer in a game than alone) is the same contention.

## How the test runs, and what it needed

- The expectation «The picture keeps up with the camera» (`09-towers/02-battlefield.expect`) is
  `Status: pending` until it passes: it fails today, by the numbers above.
- It has an event of its own (pan for 3 s). On the event that «The keys move the camera across
  the field» uses, the generator made a test for neither.
- In a test run the system does not draw the app's window by itself, and the app asks for
  pictures as it draws. The driver draws the window 60 times a second during the pan, as a screen
  does.
- The valley takes over two minutes to load in a game: run the test with
  `SIM001_SUITE_BUDGET=600` (the default, 120 s, runs out first).

## Step 1: each picture with its own camera, and no gate

Each frame the 3D view spawns a `ReadbackOnce` whose observer keeps the request the frame was
rendered for. Bevy extracts the readback with that frame and copies the texture after the frame's
render graph (`bevy_render` 0.20, `renderer/mod.rs`), so the pairing is exact. `steady_request` is
gone; `Outbox` (`visualization_battlefield::outbox`) sends a picture if it is the size asked for,
not older than one sent, and at most 60 a second.

**Alignment while moving:** screenshots of the window in the middle of a pan show the map on the
valley (the towers' circles in their clearings, the lake's shore): what the gate was for holds
without it.

**The numbers, a 3 s pan** (several runs):

| | Step 0 | Step 1 |
| --- | --- | --- |
| New pictures a second while moving | 0.3 | 14 to 23 |
| Picture out of date by (median) | 544 ms | 420 to 630 ms |

The picture now moves with the camera, but it is still far behind. The reason is new:

**The commander app itself draws about 5 times a second while the 3D view runs.** The driver
timed each draw of the window during the pan:

| | The draw (on the UI thread) | The round trip | Draws in 3.6 s |
| --- | --- | --- | --- |
| With the 3D view running | 15 ms | 140 to 200 ms | 11 to 14 |
| With no 3D view (`--no-cameras`) | 2 ms | 6 ms | 140 |
| With the 3D view, the app not watching the drones' cameras | 15 ms | 200 ms | 12 |

The draw itself is fast (the battlefield's paint, fog included, is 7 to 27 ms). The UI thread
spends the rest of each frame waiting: GPUI's Metal renderer blocks in `next_drawable()`, and the
3D view, at about 100 ms a frame, keeps the integrated GPU busy. The drones' cameras are not the
cost. So in a game **the whole commander app runs at about 5 frames a second**, not only its
battlefield, and the motion of the camera is measured in pieces (the app asks for a new camera
only as it draws).

## What this changes in the plan

Step 5 (a lighter 3D view) is now the step that decides the go/no-go: until the 3D view leaves the
GPU free most of each frame, neither the app nor the picture can be fast. In order:

1. Cap the 3D view's frame rate, and render its own window only when someone looks at it.
2. Find what costs most in its frame (the realism passes, the trees, shadows) for the
   battlefield's camera, which looks down from far and needs little of it.
3. Measure again: the app's draws, then the picture.
