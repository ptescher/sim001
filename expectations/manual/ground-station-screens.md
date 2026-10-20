# The screens of the console

This page names each screen of the ground control console and says what it shows. Use it as a map when you follow the tutorials of [[Ground control station]]. To start the console, run the station (see [[Requirements and installation]]).

The console is one window of bezelled screens. Below 1500 points wide, it changes to two columns.

| Screen | What it shows |
| --- | --- |
| Primary flight display | The horizon, with the pitch ladder, the roll scale, the heading tape, and the airspeed and altitude tapes |
| Tactical situation | A top-down photo of the valley with range rings, home, the mission, the plan you edit, the GUIDED target, the ground track and the other aircraft. Its buttons are the map tools. |
| Vehicle command | ARM (with a second press to confirm), DISARM, and one button for each flight mode |
| Telemetry trends | Two-minute charts of altitude, speeds, battery voltage and throttle |
| Caution and warning | Lamps for LINK, GPS, EKF, AHRS, BATT, ARMED, PREARM, STALL, TERRAIN, FENCE, RC and XTRACK. A warning flashes. |
| Systems and messages | Telemetry readouts, RC and servo outputs, and the log of the autopilot messages and of your commands |

## The sensor feed

The sim has a camera on the gimbal, just ahead of and below the nose. The sensor feed window shows its picture at 320 by 240 pixels, up to 30 frames a second.

- Looking straight ahead (FWD), the flight HUD is drawn over the picture.
- When you slew the camera, a reticle replaces the HUD. The reticle shows the bearing and the point where the line of sight meets the terrain.
- Drag in the picture to slew. Scroll to zoom from 4° to 90°. FWD, DOWN 30, LEFT, RIGHT 60 and NADIR are preset directions.
- Click the picture, or press LOOK on the map, to track a ground point. TRACK SPI holds the current point. STOP lets it go.

## Windows

`--station` opens three windows: the sim, the sensor feed and the console. With one display, it tiles them. With more displays, it spreads them. Use `--layout single` to tile all of them on the main display. Drag the top bar of a window to move it. Quit with Cmd+Q.
