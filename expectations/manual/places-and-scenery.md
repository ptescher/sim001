# Places and scenery

SIM001 flies in an alpine valley and in real places. This page tells you what you can see, and how to choose the place.

## The Alpine Valley

This is the default place. It has mountains with granite faces and snow, a mirror lake, about 24,000 trees up to a treeline, meadows with grass and flowers, rocks, fair-weather clouds and haze on the far ridges. The windsock by the pilot box shows the wind.

## Real places

Two real places are built from USGS lidar, NAIP aerial photos and OpenStreetMap:

- `cypress`: Cypress Island Airfield, WA
- `berkeley`: the Park at the Berkeley Waterfront, CA

Choose one with `--map NAME`. In the station, `cd domains/teleoperation/console && cargo run --release -- --station --map cypress` builds the map pack the first time. The station then starts ArduPilot with home at the pilot box of that place, and the tactical map shows its aerial photo. In `sim001_view`, use `--map NAME` so the scenery matches the real field. See [[Watching a real plane]].

To add a place, see `maps/README.md` in the repository. It has the data, the licenses and the steps.

## Wind and weather

Press `[` and `]` for the wind speed, comma and full stop for the wind direction, and `/` for the turbulence. The wind has a boundary-layer profile.

## Picture quality

Rasterized lighting with shadow maps is the default. `--raytrace` adds ray-traced shadows, global illumination and reflections. F5 and F6 change the render size. `SIM001_TREE_FRAC=0.5` draws half of the forest.
