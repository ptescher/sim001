# Controls

This page lists every way to fly SIM001: the keyboard, a Nintendo Switch Pro Controller, and a RadioMaster TX15 (or another EdgeTX radio). For the radio, it also shows how to set it up. Press H in the sim at any time to see the keys on screen.

## Keyboard

| Key | Action |
| --- | --- |
| W / S, A / D (or the arrow keys) | Pitch (S is nose up), and roll |
| Q / E | Rudder (the V-tail mix, with differential thrust) |
| Shift / Ctrl | Throttle up and down |
| 1 to 5, 0 | Throttle at 20, 40, 60, 80 or 100 %, and cut |
| Space | Throw the plane |
| R | Pick the plane up again |
| V | Fit a fresh battery |
| M | Flight mode: MANUAL, ANGLE (self-levelling), ACRO (rate) |
| L | Low and high rates |
| X | Differential thrust on and off |
| C | Camera: line of sight, chase, FPV |
| Z | Auto-zoom in the line-of-sight view |
| [ and ], comma and full stop, / | Wind speed, wind direction, turbulence |
| F5 / F6 | Render resolution down and up |
| P | Pause |
| H or F1 | Help |
| F2 | Radio calibration |
| F3 | Raw controller axes |
| F4 | Radio and gamepad profile |
| F11 | Full screen |
| G / T | Start a pylon race or a game of tag. The same key stops it. |
| B / N | With a flight controller: the arm switch, and the next mode |
| J | Station: change between ArduPilot and Direct control |
| Esc | Close help, the controller overlay or the calibration wizard |
| Cmd+Q | Quit |

To throw the plane, hold the throttle above 55 %, or press Space.

## Nintendo Switch Pro Controller

On macOS the sim reads the controller directly over USB or Bluetooth. It uses Mode 2: the left stick is rudder and throttle, and the right stick is roll and pitch. Set `gamepad.stick_mode = 1` in `controls.cfg` for Mode 1. Press HOME to show a diagram of what each button does.

| Button | Action |
| --- | --- |
| B (bottom) | Launch |
| A (right) | Camera |
| X (top) | Flight mode |
| Y (left) | Rates |
| + | Reset |
| − (minus) | Pause |
| ZL / ZR | Throttle cut, throttle up |
| L / R | ArduPilot arm, ArduPilot mode |
| HOME | Controller layout overlay |
| D-pad up | Auto-zoom in the line-of-sight view |

## RadioMaster TX15

Do these three steps once. The sim saves the result in `controls.cfg`.

1. **Load the model.** Plug the radio in as USB storage. Copy `domains/teleoperation/edgetx/MODELS/aero-6pos.yml` to the `MODELS` folder of its SD card. Choose the model *T1S VTOL* in the radio. Channels 1 to 4 are the sticks. CH5 is SE, the arm switch. CH8 is the mode channel: the six buttons on the front give QHOVER, QLOITER, FBWA, CRUISE, AUTO and QRTL. CH11 and CH12 are the knobs P1 and P2. Then plug the radio in over USB, and choose *USB Joystick (HID)* when EdgeTX asks.
2. **Calibrate the sticks.** Start the sim. Press F2. Put the sticks in the centre with the throttle all the way down, and press Enter. Move each stick when the wizard asks. Press Enter to skip the buttons.
3. **Map the switches.** Copy `controls.cfg.example` to `controls.cfg`. Press F3, move SE, press a mode button and turn each knob. Put the name of the axis that changes on the matching line: `radio.arm`, `radio.mode`, `radio.zoom` and `radio.view`.

P1 zooms the line-of-sight camera. P2 picks the view. Throttle is absolute, as on the real radio. Rates and expo come from the sim.

For the screen and the colours of the radio, there are two EdgeTX themes, SIM001 Night and SIM001 Day, in `domains/teleoperation/edgetx/THEMES`. Copy the folders to the `THEMES` folder of the SD card, and choose one under Radio Settings, Themes.

To fly the VTOL with the radio, see [[Setting up the TX15]] and [[Flying with a radio]].

## The file controls.cfg

`controls.cfg` holds the mapping for your radio. Each line is `radio.<name> = <axis>, <centre>, <scale>`. The axis is the channel as the sim names it. The scale is 1 or −1, to turn a direction round. The file also has `gamepad.stick_mode`, `gamepad.deadzone` and `gamepad.throttle_rate`, and `radio.names`, a list of name parts that tell the sim which devices are radios. `controls.cfg.example` explains each line.
