# Plan: the battlefield keeps up with the camera

The commander app (GPUI) draws the battlefield on a picture that another process renders: the
game's 3D view, `towers_view` (Bevy), or `sim001_view` for a real place. This plan keeps the two
processes and fixes the join between them. Issue: #95. The audit behind it: #93.

The other way, one process with Bevy owning the window, is option 2. It is where this plan goes
if the go/no-go below fails.

## Where the lag comes from (read from the code, not measured)

1. **The picture stops while the camera moves.** `steady_request`
   (`domains/visualization/battlefield/src/lib.rs`) sends a picture only when the camera held
   still for 4 frames (`STEADY`). It was added in be3b8be because the readback trails the camera
   by a number of frames that varies, and a guessed camera put the map beside the scenery. So
   while the commander pans, zooms or turns, the battlefield, and the map drawn with the picture's
   camera, stays where it was, and jumps when the camera stops.
2. **Each picture is copied many times.** Render, read back from the GPU, copied again
   (`ev.data.clone()`), sent raw on TCP (up to 2560 × 1600 × 4 bytes, about 16 MB), made into a
   new `RenderImage` and uploaded to GPUI's atlas, shown at the next tick of a 33 ms timer
   (`ui.rs`). At most 30 a second (`MAX_FPS`).
3. **The 3D view renders a lot at once**: its own window at vsync, each drone camera that
   someone watches (five cards and the PORTRAIT), and the battlefields, on the realism engine.

## Targets

- While the camera moves, the picture keeps up: at least 60 pictures a second.
- From input to a picture that shows it: under about 50 ms.
- When the camera stops, the map lies exactly on the picture.

**Go/no-go:** if steps 1 to 3 do not get under about 80 ms and 45 pictures a second, stop and go
to option 2.

## Steps

### 0. Measure

- `Request` carries a sequence number, and each `Frame` echoes the number of the request it was
  rendered for (`teleoperation_shared::battlefield`, with a new magic).
- The app keeps when it sent each number, and logs input to paint (ms), pictures a second
  while the camera moves, and the time to paint the overlays (the fog is up to 128 × 128 cells).
- `towers_view` logs its own frame time.

The numbers go into `plans/battlefield-latency-results.md` and #95.

### 1. Pair each picture with its camera exactly; remove the gate

- A `ReadbackOnce` (Bevy 0.20) spawned for each frame, in place of the one standing `Readback`.
  Its observer keeps the request the frame was rendered with. A picture's camera is then known,
  not guessed.
- `steady_request` and `STEADY` go. Pictures go out while the camera moves.
- `MAX_FPS` 60, as a setting. No extra copy of the data where the observer can take it.

The first thing this step does is check its premise: that a readback spawned in frame N copies
the picture of frame N under pipelined rendering (read from Bevy's `gpu_readback.rs`, not tried).

Checks: a unit test that the pairing holds with a camera that moves every frame; the alignment
expectations of `expectations/09-towers/02-battlefield.expect` still pass; a new one: while the
camera pans for a second, the app gets more than N pictures.

### 2. Repaint when a picture arrives

The link thread wakes the view through a channel, not the 33 ms timer. While a camera key is held
or the camera eases (#94), the view asks for animation frames; otherwise it sleeps.

### 3. Render a margin; move the last picture until the next arrives

The view renders about 15 % wider than the battlefield. The app draws the last picture moved and
scaled to the camera now, and the overlays with the camera now, so a pan or a zoom answers in
the same frame. GPUI draws an image in an axis-aligned rectangle, so a turn cannot be faked this
way: it relies on the eased camera (#94) and waits for the next picture.

Check: while moving, the map is off the scenery by a few pixels at most; at rest, by none.

### 4. Fewer copies

- The pixels in a shared-memory ring (`memmap2`, on all three platforms); TCP keeps the requests
  and the headers. One readback and one atlas upload a picture remain.
- From step 0's numbers: whether to ask for pictures at the screen's own pixels (sharper on a
  Retina screen, four times the bytes). Today the app asks in logical pixels.

### 5. A lighter 3D view while a commander plays

- Its own window hidden, or rendered at a low rate, while nobody looks at it.
- The drone cameras on the cards small and at a low rate; only the PORTRAIT full.

### 6. Optional, macOS only: no copy at all

Bevy renders into an IOSurface and GPUI shows it with `surface(CVPixelBuffer)`
(`vendor/gpui/src/elements/surface.rs`, macOS only). No readback, no upload. Linux (dma-buf) and
Windows (DXGI) would each need a path of their own and a change to the vendored GPUI. Only if
steps 1 to 5 miss the targets on a Mac.

## Order

0 to 2 first (a day or two): they show whether the hybrid can work. Then 3. Then 4 and 5 as the
numbers say. 6 only if needed.

**After step 0** (`plans/battlefield-latency-results.md`): the 3D view renders only 8 to 11
frames a second in a game, so a picture is about 100 ms old before it leaves the view. Step 5
(a lighter 3D view) moves up, to just after step 1: without it no other step can meet the
targets. Each step by the `tdd` skill, with `cargo clippy --workspace
--all-targets` and `cargo fmt`, and a PR.
