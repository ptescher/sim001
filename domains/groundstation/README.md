# Ground station

The pilot's console on the ground: a GPUI ground control station that flies ArduPlane SITL
and the sim over MAVLink, with a PFD, a tactical map, mission planning and the sensor feed.

| Crate | Path | Role |
| --- | --- | --- |
| `groundstation_console` | [console/](console/README.md) | The console itself (binary `t1s_gcs`). Excluded from the workspace: it keeps its own lockfile |
| `groundstation_shared` | [shared/](shared) | What the sim and the console both use: the sensor feed wire protocol, the window layout planner and the map pack format. No dependencies |
