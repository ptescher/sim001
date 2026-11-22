"""The last seconds of a Towers drone before a crash, from a recorded game.

    .ardupilot-venv/bin/python tools/flightlog/crash.py GAME_DIR [DRONE T] [--seconds S]

GAME_DIR is what `towers_server --record GAME_DIR` wrote: `tracks.tsv` (every drone ten times a
second), `crashes.tsv` and each drone's ArduPilot log (`drone<i>.BIN`). With no DRONE and T, every
crash in `crashes.tsv`. Each line is the drone as the server saw it (position, height, airspeed,
attitude, order, the leg the ground flies it on, the nearest other drone) beside what the executor
asked for at that moment (its roll, pitch and throttle, the envelope's rules that changed them and
the separation escape), from the `AGNT` record of its log. The two clocks are matched on the plan
ids, which both record. Needs pymavlink (in `.ardupilot-venv`, made by `ardupilot/build_native.sh`).
"""
import argparse
import csv
import os
import sys

from pymavlink import mavutil

RULES = {1: "sep", 2: "fence", 4: "ceil", 8: "floor", 16: "stall", 32: "clamp"}
ESCAPES = ["-", "climb", "sink", "R", "L", "R+climb", "L+climb", "R+sink", "L+sink", "level"]


def agnt(path):
    log = mavutil.mavlink_connection(path)
    out = []
    while True:
        m = log.recv_match(type="AGNT")
        if m is None:
            return out
        out.append(m.to_dict())


def show(game, drone, t, seconds):
    rows = [r for r in csv.DictReader(open(os.path.join(game, "tracks.tsv")), delimiter="\t")
            if r["drone"] == str(drone) and t - seconds <= float(r["t"]) <= t + 0.05]
    log = os.path.join(game, f"drone{drone}.BIN")
    steps = agnt(log) if os.path.exists(log) else []
    first = {}
    for a in steps:
        first.setdefault(str(a["Plan"]), a["TimeUS"] / 1e6)
    seen = {}
    for r in rows:
        seen.setdefault(r["plan"], float(r["t"]))
    offsets = sorted(first[p] - gt for p, gt in seen.items() if p in first and p not in ("-1", "0"))
    off = offsets[len(offsets) // 2] if offsets else None
    if off is None:
        print(f"  (no ArduPilot log matched for drone {drone}: the executor's side is left out)")
    steps = [(a["TimeUS"] / 1e6 - off, a) for a in steps] if off is not None else []
    j = 0
    for r in rows[::5]:
        gt = float(r["t"])
        while j + 1 < len(steps) and steps[j + 1][0] <= gt:
            j += 1
        line = (f"t {gt:6.1f} at {float(r['x']):5.0f} {float(r['y']):5.0f} alt {float(r['alt']):5.1f} "
                f"as {float(r['airspeed']):4.1f} roll {r['roll']:>4} pitch {r['pitch']:>3} order {r['order']:<8} "
                f"leg {'circle' if r['leg_orbit'] == '1' else 'go to'} ({r['leg_x']}, {r['leg_y']}) alt {r['leg_alt']} "
                f"near {r['near'] or '-'} {r['near_d']} {r['near_dz']}")
        if steps:
            a = steps[j][1]
            rules = "+".join(n for b, n in RULES.items() if a["Rule"] & b) or "-"
            esc = ESCAPES[a["Esc"]] if a["Esc"] < len(ESCAPES) else str(a["Esc"])
            line += f" | asks roll {a['Roll'] // 100:>4} pitch {a['Pitch'] // 100:>3} thr {a['Thr']:.2f} rules {rules} escape {esc}"
        print(line)


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("game")
    p.add_argument("drone", nargs="?", type=int)
    p.add_argument("t", nargs="?", type=float)
    p.add_argument("--seconds", type=float, default=14.0)
    a = p.parse_args()
    if a.drone is not None and a.t is not None:
        show(a.game, a.drone, a.t, a.seconds)
        return
    crashes = list(csv.DictReader(open(os.path.join(a.game, "crashes.tsv")), delimiter="\t"))
    if not crashes:
        print("no crash in this game")
    for c in crashes:
        print(f"=== drone {c['drone']} at {c['t']} s: {c['reason']} at ({c['x']}, {c['y']}), {c['alt']} m")
        show(a.game, int(c["drone"]), float(c["t"]), a.seconds)


if __name__ == "__main__":
    sys.exit(main())
