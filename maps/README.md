# Real-world maps

Besides the procedural Alpine Valley, the sim can fly real places built from free public data:

| Map | Place | Notes |
| --- | --- | --- |
| `cypress` | Cypress Island Airfield, Cypress Island, WA | Private gravel strip (about 690 m, 16/34) on a 290 m ridge in dense Douglas fir forest, with the San Juan Islands around it |
| `berkeley` | Park at the Berkeley Waterfront (formerly Cesar Chavez Park), Berkeley, CA | 90 acres of open dry grass on a peninsula in the bay, with the marina and the city beside it |

## Fly one

```bash
cd gcs && cargo run --release -- --station --map cypress
```

The station builds the map pack the first time, which takes a few minutes to download. It starts ArduPilot SITL with its home at the place's pilot box, then opens the sim, the sensor feed and the console on that map. The sim can also fly a map on its own: `cargo run --release -- --map berkeley`.

## Where the data comes from

| Layer | Source | Resolution | License |
| --- | --- | --- | --- |
| Elevation | [USGS 3DEP](https://www.usgs.gov/3d-elevation-program), bare-earth lidar DEM | 1 m, resampled to 2 m | Public domain |
| Aerial photos | [USGS/USDA NAIP](https://naip-usdaonline.hub.arcgis.com/) | 0.3 m. The pack keeps 1 m over the whole map and 0.4 m within 800 m of home | Public domain |
| Near-infrared | NAIP band 4 | 1 m | Public domain |
| Runway, buildings | [OpenStreetMap](https://www.openstreetmap.org/) via Overpass | vector | © OpenStreetMap contributors, [ODbL](https://opendatacommons.org/licenses/odbl/) |

Trees and land cover aren't downloaded. `tools/mapbuild` derives them from the photos:

- **Water:** near sea level and unvegetated, or very dark in near-infrared. Dark patches smaller than 0.5 ha are shadows and are dropped, which keeps inland lakes.
- **Trees:** a stronger vegetation signal (NDVI) than the image's typical land, darker than it, and textured like crowns where the signal is weak. The thresholds adapt to each region, because NAIP differs a lot between states and years: Berkeley's photos are far brighter than Washington's.
- **Paved:** neutral grey and unvegetated. Dry grass is just as unvegetated, but it's warm in hue. OSM runways and building footprints are drawn on top.
- **Tree placement:** trees are grown on a jittered grid inside the canopy mask. They're 5.5 m apart within 700 m of home, sparser further out (the photo carries the distant forest), and capped at 60,000. Heights and the conifer share come from `map.txt`.

## Add a place

1. Create `maps/<name>/map.txt`:

   ```
   title=My Field
   center_lat=...        # centre of the square to fetch
   center_lon=...
   size_m=5000           # side of the square (metres)
   home_lat=...          # the pilot box = ArduPilot home
   home_lon=...
   launch_heading=270    # degrees true
   tree_min_m=8          # tree heights grown from the canopy
   tree_max_m=20
   conifer_fraction=0.5
   ```

2. Build it:

   ```bash
   cargo run --release --manifest-path tools/mapbuild/Cargo.toml -- <name>
   ```

   This writes `maps/<name>/build/` (git-ignored) and the ground station's map in `gcs/assets/<name>/`. Downloads are cached in `build/cache/`, so re-runs take seconds and a flaky server can't lose data. NAIP and 3DEP cover the US only.

3. Check `build/landcover_preview.png` against `build/ortho_wide.jpg`. Brown is ground, green trees, blue water, grey paved, red buildings. If trees or water are off for the region, `MAPBUILD_PROBE="lat,lon;lat,lon"` prints the vegetation statistics around known spots to tune the thresholds in `classify()`.

## What's in a pack

| File | Contents |
| --- | --- |
| `pack.txt` | Home (lat, lon, ground altitude), the raster squares, credits |
| `height.f32` | Ground elevation, metres above mean sea level, north-up |
| `ortho_wide.jpg`, `ortho_core.jpg` | The aerial photos (1 m and 0.4 m) |
| `landcover.png` | One class per pixel: ground, tree, water, paved, building |
| `trees.bin` | The trees grown from the canopy |
| `buildings.txt`, `runway.txt` | OSM footprints as oriented boxes |

Positions are metres north and east of home, the same origin ArduPilot uses, so the sim, SITL and the ground station all agree. The pack format is in `station/src/mappack.rs`.
