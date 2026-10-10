#!/usr/bin/env python3
"""End-to-end check of the t1s_sim <-> ArduPlane SITL link.

Run inside the SITL container:  docker exec t1s-sitl python3 /check_link.py
Waits for the EKF to settle, arms, selects FBWA (the sim's throttle stick then makes
ArduPilot spool the motors, which hand-launches the plane), then reports telemetry,
switches to LOITER and reports again.
"""
import math
import sys
import time

from pymavlink import mavutil

m = mavutil.mavlink_connection("tcp:127.0.0.1:5760", source_system=255)
m.wait_heartbeat(timeout=60)
print(f"heartbeat from system {m.target_system}, mode {m.flightmode}", flush=True)
m.mav.request_data_stream_send(m.target_system, m.target_component, mavutil.mavlink.MAV_DATA_STREAM_ALL, 10, 1)


override = None


def drain(seconds, label):
    end = time.time() + seconds
    last = 0.0
    last_ovr = 0.0
    while time.time() < end:
        if override and time.time() - last_ovr > 0.5:
            last_ovr = time.time()
            m.mav.rc_channels_override_send(m.target_system, m.target_component, *override)
        msg = m.recv_match(blocking=True, timeout=1)
        if msg is None:
            continue
        t = msg.get_type()
        if t == "STATUSTEXT":
            print(f"  [{label}] STATUSTEXT: {msg.text}", flush=True)
        if t == "VFR_HUD" and time.time() - last > 2.0:
            last = time.time()
            att = m.messages.get("ATTITUDE")
            rc = m.messages.get("RC_CHANNELS")
            srv = m.messages.get("SERVO_OUTPUT_RAW")
            s = f"  [{label}] mode {m.flightmode:8} armed {m.motors_armed()} alt {msg.alt:6.1f} m AS {msg.airspeed:5.1f} GS {msg.groundspeed:5.1f} thr {msg.throttle:3d}%"
            if att:
                s += f" roll {math.degrees(att.roll):+5.1f} pitch {math.degrees(att.pitch):+5.1f}"
            if rc:
                s += f" | RC {rc.chan1_raw} {rc.chan2_raw} {rc.chan3_raw} {rc.chan4_raw} ch7 {rc.chan7_raw} ch8 {rc.chan8_raw}"
            if srv:
                s += f" | SRV {srv.servo1_raw} {srv.servo2_raw} {srv.servo3_raw} {srv.servo4_raw} {srv.servo5_raw}"
            print(s, flush=True)


def set_mode(name):
    mode_id = m.mode_mapping()[name]
    m.mav.set_mode_send(m.target_system, mavutil.mavlink.MAV_MODE_FLAG_CUSTOM_MODE_ENABLED, mode_id)


print("waiting for EKF / GPS (up to 90 s)...", flush=True)
deadline = time.time() + 90
while time.time() < deadline:
    msg = m.recv_match(type=["STATUSTEXT", "GPS_RAW_INT", "EKF_STATUS_REPORT"], blocking=True, timeout=1)
    if msg and msg.get_type() == "STATUSTEXT":
        print("  STATUSTEXT:", msg.text, flush=True)
        if "using GPS" in msg.text:
            break
drain(5, "pre-arm")

set_mode("FBWA")
time.sleep(1)
print("arming...", flush=True)
m.mav.command_long_send(m.target_system, m.target_component, mavutil.mavlink.MAV_CMD_COMPONENT_ARM_DISARM, 0, 1, 0, 0, 0, 0, 0, 0)
drain(8, "armed")
if not m.motors_armed():
    print("normal arm refused; force-arming for the test", flush=True)
    m.mav.command_long_send(m.target_system, m.target_component, mavutil.mavlink.MAV_CMD_COMPONENT_ARM_DISARM, 0, 1, 21196, 0, 0, 0, 0, 0)
    drain(5, "armed")

# Throttle stick to 80% via RC override (UINT16_MAX = leave channel to the sim's RC).
IGN = 65535
# Climb: 80% throttle and a little back stick, to clear the trees and hills before loitering.
override = [IGN, 1650, 1800, IGN, IGN, IGN, IGN, IGN]
drain(40, "FBWA")
override = [IGN, 1500, 1800, IGN, IGN, IGN, IGN, IGN]
set_mode("LOITER")
drain(60, "LOITER")
sys.exit(0 if m.motors_armed() else 1)
