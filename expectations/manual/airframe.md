# The T1-S airframe

This page gives the numbers of the plane that the sim flies. They are estimates built from the published specifications of the HEEWING T1-S.

| | |
| --- | --- |
| Wing span | 790 mm |
| Length | 715 mm |
| Motors | 2 × 1406, 3150 KV, with 3830 propellers |
| Battery | 4S 1800 mAh |
| Weight | 800 g with the battery |
| Wing area | 0.115 m² (about 70 g/dm²) |
| V-tail dihedral | 35° |
| Static thrust | About 4.9 N for each motor |

## What the flight model gives

- Hands-off trim at about 13 m/s
- A stall at about 9.5 m/s
- A roll rate of about 330°/s and a pitch rate of about 190°/s
- About 15 minutes at half throttle

The model sums the forces of each wing section and V-tail panel in its own airflow. This includes prop wash and the downwash of the wing. Stalls, tip stalls, adverse yaw and dihedral effect come from the shape. It also models differential thrust, battery voltage sag, wind with turbulence, ground contact and collisions.

The T1-S VTOL has two tilt-rotors and a rear rotor. See [[Flying a VTOL]] for how it flies.
