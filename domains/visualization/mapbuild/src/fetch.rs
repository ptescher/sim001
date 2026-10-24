//! Downloads: USGS ArcGIS ImageServer rasters (3DEP elevation, NAIP imagery) and OpenStreetMap
//! features via Overpass. Everything is public; requests identify the tool and retry politely.

use std::io::Read;
use std::time::Duration;

const UA: &str = "sim001-mapbuild/0.1 (flight simulator map builder)";
pub const DEM_SERVICE: &str =
    "https://elevation.nationalmap.gov/arcgis/rest/services/3DEPElevation/ImageServer/exportImage";
pub const NAIP_SERVICE: &str =
    "https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer/exportImage";
const OVERPASS: [&str; 3] = [
    "https://overpass.kumi.systems/api/interpreter",
    "https://overpass-api.de/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];

static CACHE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Every response is kept in `dir` (keyed by request), so re-runs are quick and a flaky server
/// doesn't lose data an earlier run got.
pub fn set_cache(dir: std::path::PathBuf) {
    std::fs::create_dir_all(&dir).ok();
    let _ = CACHE.set(dir);
}

fn cache_path(key: &str) -> Option<std::path::PathBuf> {
    // FNV-1a.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    CACHE.get().map(|d| d.join(format!("{h:016x}")))
}

fn cached(key: &str, fetch: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<Vec<u8>, String> {
    let path = cache_path(key);
    if let Some(b) = path.as_ref().and_then(|p| std::fs::read(p).ok()) {
        return Ok(b);
    }
    let b = fetch()?;
    if let Some(p) = path {
        std::fs::write(p, &b).ok();
    }
    Ok(b)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(240)).user_agent(UA).build()
}

fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(3 * attempt));
        }
        match agent().get(url).call() {
            Ok(resp) => {
                let mut b = Vec::new();
                resp.into_reader().read_to_end(&mut b).map_err(|e| e.to_string())?;
                // ArcGIS reports errors as JSON with a 200.
                if b.starts_with(b"{") {
                    last = String::from_utf8_lossy(&b[..b.len().min(300)]).into_owned();
                    continue;
                }
                return Ok(b);
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(format!("{last} ({url})"))
}

/// A lat/lon-aligned raster mosaic: pixel (x, y) covers lon0 + x*dlon, lat1 - y*dlat.
pub struct Mosaic<T> {
    pub w: usize,
    pub h: usize,
    pub bands: usize,
    pub lon0: f64,
    pub lat1: f64,
    pub dlon: f64,
    pub dlat: f64,
    pub data: Vec<T>,
}

impl<T: Copy + Default + Into<f32>> Mosaic<T> {
    /// Bilinear sample of `band` at a lat/lon; None outside.
    pub fn sample(&self, lat: f64, lon: f64, band: usize) -> Option<f32> {
        let fx = (lon - self.lon0) / self.dlon - 0.5;
        let fy = (self.lat1 - lat) / self.dlat - 0.5;
        if fx < 0.0 || fy < 0.0 || fx >= (self.w - 1) as f64 || fy >= (self.h - 1) as f64 {
            return None;
        }
        let (x, y) = (fx as usize, fy as usize);
        let (tx, ty) = ((fx - x as f64) as f32, (fy - y as f64) as f32);
        let at = |x: usize, y: usize| -> f32 { self.data[(y * self.w + x) * self.bands + band].into() };
        let top = at(x, y) + (at(x + 1, y) - at(x, y)) * tx;
        let bot = at(x, y + 1) + (at(x + 1, y + 1) - at(x, y + 1)) * tx;
        Some(top + (bot - top) * ty)
    }
}

/// Lat/lon box and the pixel size (degrees) to fetch it at.
pub struct Request {
    pub lat0: f64,
    pub lat1: f64,
    pub lon0: f64,
    pub lon1: f64,
    pub m_per_px: f64,
}

impl Request {
    fn dims(&self) -> (f64, f64, usize, usize) {
        // Square pixels in degrees: ArcGIS silently stretches the requested box to make the
        // output's pixels square in its spatial reference, which would misplace every tile.
        // (Across, a degree is shorter, so x gets finer than `m_per_px`; that's fine.)
        let dlat = self.m_per_px / 111_320.0;
        let dlon = dlat;
        let w = ((self.lon1 - self.lon0) / dlon).ceil() as usize;
        let h = ((self.lat1 - self.lat0) / dlat).ceil() as usize;
        (dlat, dlon, w, h)
    }
}

/// Fetches `req` as a grid of tiles no bigger than `max_px`, decoding each with `decode`
/// (returns pixels row-major, `bands` per pixel), and stitches them.
fn fetch_tiled<T: Copy + Default>(
    req: &Request,
    max_px: usize,
    bands: usize,
    what: &str,
    url_for: impl Fn(f64, f64, f64, f64, usize, usize) -> String,
    decode: impl Fn(&[u8], usize, usize) -> Result<Vec<T>, String>,
) -> Result<Mosaic<T>, String> {
    let (dlat, dlon, w, h) = req.dims();
    let (tx, ty) = (w.div_ceil(max_px), h.div_ceil(max_px));
    let (tw, th) = (w.div_ceil(tx), h.div_ceil(ty));
    let mut data = vec![T::default(); w * h * bands];
    let total = tx * ty;
    for j in 0..ty {
        for i in 0..tx {
            let (x0, y0) = (i * tw, j * th);
            let (cw, ch) = (tw.min(w - x0), th.min(h - y0));
            let lon_a = req.lon0 + x0 as f64 * dlon;
            let lat_b = req.lat1 - y0 as f64 * dlat;
            let url = url_for(lon_a, lat_b - ch as f64 * dlat, lon_a + cw as f64 * dlon, lat_b, cw, ch);
            eprint!("\r  {what}: tile {}/{total} ({cw}x{ch})      ", j * tx + i + 1);
            let bytes = cached(&url, || get_bytes(&url))?;
            let px = decode(&bytes, cw, ch).map_err(|e| format!("{what}: {e}"))?;
            for row in 0..ch {
                let src = &px[row * cw * bands..(row + 1) * cw * bands];
                let dst = ((y0 + row) * w + x0) * bands;
                data[dst..dst + cw * bands].copy_from_slice(src);
            }
        }
    }
    eprintln!("\r  {what}: {w}x{h} px in {total} tile(s), {:.2} m/px          ", req.m_per_px);
    Ok(Mosaic { w, h, bands, lon0: req.lon0, lat1: req.lat1, dlon, dlat, data })
}

fn arcgis_url(service: &str, extra: &str, lon0: f64, lat0: f64, lon1: f64, lat1: f64, w: usize, h: usize) -> String {
    format!(
        "{service}?bbox={lon0:.8},{lat0:.8},{lon1:.8},{lat1:.8}&bboxSR=4326&imageSR=4326&size={w},{h}\
&interpolation=RSP_BilinearInterpolation&f=image{extra}"
    )
}

/// USGS 3DEP bare-earth elevation (best available: 1 m lidar where flown), metres AMSL.
/// Water and no-data come back as sea level.
pub fn dem(req: &Request) -> Result<Mosaic<f32>, String> {
    // The service advertises 8000 px but fails above ~2000; 1500 px tiles are quick.
    fetch_tiled(
        req,
        1500,
        1,
        "elevation (USGS 3DEP)",
        |a, b, c, d, w, h| {
            arcgis_url(
                DEM_SERVICE,
                "&format=tiff&pixelType=F32&noDataInterpretation=esriNoDataMatchAny&compression=None",
                a,
                b,
                c,
                d,
                w,
                h,
            )
        },
        |bytes, w, h| {
            let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
            let (iw, ih) = dec.dimensions().map_err(|e| e.to_string())?;
            let img = dec.read_image().map_err(|e| e.to_string())?;
            let tiff::decoder::DecodingResult::F32(v) = img else { return Err("elevation isn't float32".into()) };
            if (iw as usize, ih as usize) != (w, h) {
                return Err(format!("elevation tile is {iw}x{ih}, expected {w}x{h}"));
            }
            Ok(v.into_iter().map(|z| if z.is_finite() && z > -500.0 && z < 9000.0 { z } else { 0.0 }).collect())
        },
    )
}

/// USGS NAIP orthoimagery: natural colour (3 bands) or near-infrared (1 band).
pub fn naip(req: &Request, nir: bool) -> Result<Mosaic<u8>, String> {
    let bands = if nir { 1 } else { 3 };
    let ids = if nir { "3" } else { "0,1,2" };
    fetch_tiled(
        req,
        2500,
        bands,
        if nir { "NAIP near-infrared" } else { "NAIP colour" },
        |a, b, c, d, w, h| {
            arcgis_url(NAIP_SERVICE, &format!("&format=jpg&compressionQuality=92&bandIds={ids}"), a, b, c, d, w, h)
        },
        |bytes, w, h| {
            let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
            if (img.width() as usize, img.height() as usize) != (w, h) {
                return Err(format!("image tile is {}x{}, expected {w}x{h}", img.width(), img.height()));
            }
            Ok(if nir { img.into_luma8().into_raw() } else { img.into_rgb8().into_raw() })
        },
    )
}

/// OSM runways and buildings in a lat/lon box, as Overpass JSON elements.
pub fn osm(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> Result<Vec<serde_json::Value>, String> {
    let q = format!(
        "[out:json][timeout:90];(way[\"aeroway\"=\"runway\"]({lat0},{lon0},{lat1},{lon1});\
way[\"building\"]({lat0},{lon0},{lat1},{lon1}););out tags geom;"
    );
    let body = cached(&q, || {
        let mut last = String::new();
        for ep in OVERPASS {
            for attempt in 0..2 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_secs(5));
                }
                match agent().post(ep).send_form(&[("data", &q)]) {
                    Ok(resp) => {
                        let mut b = Vec::new();
                        match resp.into_reader().read_to_end(&mut b) {
                            Ok(_) if b.starts_with(b"{") => return Ok(b),
                            Ok(_) => last = format!("{ep}: not JSON"),
                            Err(e) => last = format!("{ep}: {e}"),
                        }
                    }
                    Err(e) => last = format!("{ep}: {e}"),
                }
            }
        }
        Err(last)
    })?;
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    Ok(v["elements"].as_array().cloned().unwrap_or_default())
}
