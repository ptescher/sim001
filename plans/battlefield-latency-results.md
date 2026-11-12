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
  `Status: pending` until step 1: it fails today, by the numbers above.
- It has an event of its own (pan for 3 s). On the event that «The keys move the camera across
  the field» uses, the generator made a test for neither.
- In a test run the system does not draw the app's window by itself, and the app asks for
  pictures as it draws. The driver draws the window 60 times a second during the pan, as a screen
  does.
- The valley takes over two minutes to load in a game: run the test with
  `SIM001_SUITE_BUDGET=600` (the default, 120 s, runs out first).
