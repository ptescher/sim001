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

## Step 5, first part: a lighter 3D view

Measured first with `towers_view` alone (no game) and a small client that pans the battlefield at
1400 by 630 like the app (60 requests a second). Then in a game, in the app.

### What each camera costs

| Off (an experiment each) | 3D view | Pictures a second | Request to arrival |
| --- | --- | --- | --- |
| Nothing | 185 ms a frame | 4.8 | 936 ms |
| Its main camera | 172 ms | 5.1 | 1039 ms |
| The six drones' cameras, idle at 64 by 36 | 72 ms | 12.9 | 725 ms |

Each camera costs its own shadows, sky and exposure whatever its size, so an idle camera cost as
much as a full one. **Kept: a camera that nobody watches does not render** (`Camera::is_active`):
a drone's camera without a viewer, and a battlefield that no app asks for.

### The GPU was 13 frames behind

With the idle cameras off: 54 ms a frame, but a picture still arrived 730 ms after its request.
Timing inside the view: a request was already 723 ms old when its frame's readback completed, also
with only one readback in flight. Bevy presents a window only when a camera draws into it, and the
present is what holds the CPU back to the GPU; `towers_view` draws into images, so nothing held it:
it queued frames while the valley loaded, and the GPU stayed 13 frames behind. **Kept: frame
pacing** (`visualization_battlefield::pacing`): after each frame the render thread waits for the
GPU to finish the frame two before (at most 500 ms).

| | 3D view | Pictures a second | Request to arrival | Valley loaded in |
| --- | --- | --- | --- | --- |
| Idle cameras off | 54 ms | 18 | 730 ms | 35 s |
| And frame pacing | 53 ms | 18.6 | 224 ms | 25 s |

### What the rest of the frame is

Each an experiment, from 54 ms a frame with pacing: without the clouds, the exposure, bloom or fog,
no change; without the atmosphere 46 ms; without TAA 50 ms; a quarter of the pixels 50 ms; the
sun's shadows off 40 ms; two shadow cascades instead of four, no change. The view uses about half
a CPU core. So the frame is geometry (the forest and the terrain, for each camera, and once more for
each shadow cascade), not pixels or the CPU. Turning off the view's main camera crashes Bevy's
atmosphere (`bevy_pbr` `atmosphere/environment.rs:124`).

### In a game, in the app

| | The app's draws in a 3.6 s pan | Picture behind (median) | Pictures a second |
| --- | --- | --- | --- |
| Before step 5 | 12 (about 190 ms each) | 420 to 630 ms | 14 to 23 |
| Step 5 so far | 11 (about 190 ms each) | 417 ms | 28 |

In a game the app watches all five drones' cameras, so they render, and the 3D view is at about
80 ms a frame. The app still draws only about 5 times a second. Experiments to find why:

| The 3D view runs, and | The app's draws (median round trip) |
| --- | --- |
| is capped at 20 or 12 frames a second | about 200 ms (it already ran at 11 to 12) |
| the app takes no battlefield pictures | about 200 ms |
| the app takes nothing from it | about 190 ms |
| does not run at all (step 1) | 6 ms |
| **its window is hidden** | **32 ms**, but about half the draws still took 215 ms |

So the app is not slowed by the pictures it takes, nor by how hard the view works. It is slowed by
the 3D view's window, which opens on top of the app's (macOS throttles the drawing of a covered
window), and by something else, not found yet, that stops its UI thread about every other frame.

## Where this leaves the go/no-go

- The picture is 417 ms behind in a game, against 80 ms. About 4 frames of that is the view at
  80 ms a frame; the rest is the app drawing in bursts.
- The decision now rests on two things not done yet:
  1. hide the 3D view's window when an app serves as its screen, and find the other stall of the
     app's UI thread;
  2. a lighter look for the battlefield (no shadows on it: 14 ms), the drones' cameras smaller or
     slower, and the view's main camera off. These change how the game looks, so they are the
     user's call.

## Step 5, second part: the 3D view's window hidden, and a correction

- `towers_view --hidden` keeps its window off screen, and the commander app starts it so.
- The app logs how many times a second it draws (`battlefield: the app draws N times a second`).

**A correction to the app's numbers above.** The app's draws "about 5 times a second" were
measured by the test driver forcing draws of the app's window during a pan. Started from a test
or from a shell session, that window is not on screen, and macOS does not draw it: in a 4-minute
practice game started the same way, the app never once logged a draw rate. So those numbers
describe a window that is off screen, not what a person at the app sees, and the conclusions drawn
from them (the 3D view's window throttling the app; an unknown stall) are not supported. Runs of the
same case gave a median draw of 32 ms once and 207 ms another time.

The numbers of the 3D view itself (frame times, readback delay, request to arrival with the test
client) do not depend on the app's window, and stand.

**Next:** measure the app on a screen, with a person at it: start a practice game, pan the camera,
and read `battlefield: the app draws …` and `battlefield: the camera moved …` from the app's output.

## Step 5, third part: what a review found, and what it changed

A review of the code (2026-11-13) found two mistakes in the conclusions above.

1. **The lag figure ramped.** It counted from the first camera after the one on screen, so at the
   start of a pan, with the picture from before it still on screen, it rose from 0 and could not
   exceed the time since the pan began. A steady delay of 500 ms read as about 200 ms on a short
   pan and 500 ms on a long one, which looked like a queue filling. There is no queue: the app,
   the view and its outbox all keep only the newest request and picture. **Now** a paint says how
   old its picture is: how long ago the app asked for the camera it shows. The picture from before
   the motion is not counted, and the wait for the motion's first picture is logged apart.
2. **The battlefield has no sun shadows to cut.** Bevy's sun casts shadows out to 150 m from each
   camera, and the battlefield's camera is 450 m from the ground. The 14 ms of shadows belonged to
   the view's own main camera, which followed the first drone among the trees and rendered at
   1920 by 1080 for nobody, its window hidden.

**Kept:**

- With `--hidden` the view's main camera is parked: a 64 by 36 picture of the sky, where it
  stands (`RenderScale::parked`). It cannot be turned off (Bevy's atmosphere needs a live view).
- One frame in flight instead of two, and the pacer waits after Bevy asks for the frame's
  readbacks, so the wait completes them too.

**The view alone** (hidden, the test client panning at 1400 by 630, 10 s each):

| Frames in flight | Request to arrival | Pictures a second | ms a frame |
| --- | --- | --- | --- |
| 2 | 203 ms | 20.5 | 48 |
| 1 (kept) | 155 ms | 20.4 | 48 |
| 0 | 122 ms | 17.8 | 55 |

That is about 3 of the view's frames from request to arrival.

**In a game** (the expectation's pan; the view's own log):

| | Before (on screen, 2026-11-13) | Now |
| --- | --- | --- |
| The 3D view | 112 to 153 ms a frame (7 to 9 a second) | 56 to 64 ms (16 to 18 a second) |
| Pictures it sent the app a second | 7 | 16 |

The app's figures from this run rest on one paint (its window was off screen again) and are not
used. **Next:** the same on-screen run as before, with the app in front; at about 3 frames of 60 ms,
expect a picture about 200 ms old. The go/no-go (80 ms) then rests on step 3 (moving the last
picture with the camera until the next arrives) or on option 2, not on a lighter look.

## The frame-time experiment: how fast can the view be?

The question (from a review of step 3 against option 2): if the battlefield's camera gets to about
20 ms a frame, a picture is about 60 ms old, and a picture warped to the camera now (step 3, as a
ground plane) lines up within a few pixels. The view alone, hidden, the test client panning, each
case stripped by a throwaway switch (not committed):

| The battlefield camera, and | ms a frame | Request to arrival | Pictures a second |
| --- | --- | --- | --- |
| everything (#127) | 47.5 | 155 ms | 20.9 |
| the sun's shadows off | 38.1 | 126 ms | 25.8 |
| the view's main camera parked 10 km up | 42.2 | 137 ms | 23.6 |
| and the view's HUD camera off | 41.5 | 136 ms | 23.6 |
| no trees or grass | 35.1 | 118 ms | 27.8 |
| no atmosphere, fog, clouds, exposure, bloom or TAA | 38.7 | 126 ms | 20.1 |
| no trees, no look, no shadows | 24.6 | 87 ms | 39.9 |
| main camera high, no HUD, no trees, no look | 25.7 | 90 ms | 38.3 |
| nothing drawn at all | 31.1 | 105 ms | 32.1 |
| nothing drawn, no look, no shadows, no HUD | 22.3 | 80 ms | 44.3 |

What it says:

1. **A picture is 3.5 of the view's frames old, whatever the frame costs.** From 155 ms at 48 ms a
   frame to 80 ms at 22 ms.
2. **The view costs 22 ms a frame with nothing to draw.** So the oldest a picture can be with this
   view is about 80 ms, not 60.
3. **The parked main camera still pays for shadows:** it stands among the trees and they fall in its
   cascades. Parked 10 km up it saves 6 ms (but the 3D grass follows the main camera, and would be
   dropped).
4. **The rest is the look of the battlefield:** the trees cost 12 ms and the look 9 ms.
   Together, with shadows, 23 ms.

## Steps 2 and 3, and the main camera parked high

Chosen after the experiment: move the last picture to the camera now, rather than make the picture
younger.

- **The picture moves with the camera (step 3, `teleoperation_battlefield::warp`).** The app asks
  for a picture with a margin: 10 % each side, 8 % above and 35 % below, where the near ground
  moves fastest. It is as sharp as the battlefield in the middle, and about 1.7 times the pixels.
  The camera of the picture looks a little lower and wider from the same place. The app draws it
  as the camera now sees the flat ground, in 64 bands. A pan or a zoom keeps the rows of the
  ground as rows, so each band is the picture moved and scaled; GPUI draws it with `paint_image`
  under a content mask. The map is drawn with the camera now. A turn cannot be drawn this way: the
  battlefield keeps the picture's heading until its picture comes.
  - Unit tests: at rest the picture lies on the map within 0.5 px; after 0.15 s of a full-speed pan
    either way, or a step of the wheel, within 1 px; the margin covers the battlefield after such
    a pan.
  - Hills are not flat: they slide against the map by a few pixels at 90 ms of age, up to about
    35 px at 140 ms on the steepest slopes (the advisor's numbers). Not measured on screen yet.
- **A picture that comes is drawn at once (step 2).** The link wakes the window through a channel
  (`Battlefield::arrivals`), not the 33 ms timer. While a key or the mouse at an edge moves the
  camera, the window asks for animation frames and moves the camera in each.
- **The main camera of the view is parked 10 km up** (with `--hidden`), so no tree falls in its
  shadow cascades. The grass is streamed round where it stood (`ParkedFrom`).
- **The measure.** A new figure, the lag of the battlefield as drawn: how long ago the app asked
  for the camera it draws from. The expectation «The valley moves with the camera» (at most 20 ms)
  failed before the warp with 429 ms and passes with it at 0 ms (forced draws in a test run). The
  picture's own age keeps its pending expectation, now on a pan of its own (5 s): the harness
  skips every plan that shares an event with a pending expectation.
- **A fix in the measure:** a motion none of whose pictures was drawn read as 0 ms late; it now
  reads as long as the motion.

Still to do: an on-screen game to see the warp and the hills, and the app's draw rate with
animation frames.
