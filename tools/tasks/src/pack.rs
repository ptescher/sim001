//! Packs the results of the last `tasks docs` run for GitHub Pages: `site/results/` (the test
//! results) and `site/screenshots/` (the screenshots the manual shows, shrunk to 960 px wide).
//! Commit both: the Pages workflow can't run the tests, so it builds the manual from these.

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// How wide the screenshots on the site are.
const WIDTH: u32 = 960;
const QUALITY: u8 = 60;

/// The screenshots (`screenshots/<test>/<n>.jpg` or `.svg`) a page refers to.
pub fn screenshot_refs(html: &str) -> BTreeSet<String> {
    html.match_indices("screenshots/")
        .filter_map(|(at, _)| {
            let rest = &html[at..];
            let end = rest
                .find(|c: char| matches!(c, '"' | '\'' | ')' | '<' | '>') || c.is_whitespace())
                .unwrap_or(rest.len());
            let path = &rest[..end];
            (path.ends_with(".jpg") || path.ends_with(".svg")).then(|| path.to_string())
        })
        .collect()
}

/// `img` as a JPEG `WIDTH` wide (never enlarged), at `QUALITY`.
fn shrunk(img: image::DynamicImage) -> Result<Vec<u8>, String> {
    let img = if img.width() > WIDTH { img.resize(WIDTH, u32::MAX, FilterType::Lanczos3) } else { img };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, QUALITY).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
    Ok(out)
}

/// `src` as a JPEG `WIDTH` wide (never enlarged), at `QUALITY`.
pub fn shrink_jpeg(src: &Path, dst: &Path) -> Result<(), String> {
    let img = image::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let out = shrunk(img).map_err(|e| format!("{}: {e}", dst.display()))?;
    std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(dst, out).map_err(|e| format!("{}: {e}", dst.display()))
}

const JPEG_DATA: &str = "data:image/jpeg;base64,";

/// `svg` with each JPEG in it shrunk as `shrink_jpeg` shrinks a file (the console's screenshot
/// with the TX15's screen above it is an SVG round a JPEG of the window). The `<image>` keeps
/// its size on the page.
pub fn shrink_svg(svg: &str) -> Result<String, String> {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find(JPEG_DATA) {
        let (before, after) = rest.split_at(at + JPEG_DATA.len());
        let end = after.find(['"', '\'']).unwrap_or(after.len());
        let jpeg = base64::decode(&after[..end]).ok_or("an image in the SVG isn't base64")?;
        let img = image::load_from_memory(&jpeg).map_err(|e| e.to_string())?;
        out.push_str(before);
        out.push_str(&base64::encode(&shrunk(img)?));
        rest = &after[end..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Standard base64, for the pictures inside an SVG.
mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let (mut n, mut bits) = (0u32, 0);
        for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
            n = n << 6 | ALPHABET.iter().position(|&a| a == c)? as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((n >> bits) as u8);
            }
        }
        Some(out)
    }
}

fn pages(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "html")).collect()
        })
        .unwrap_or_default()
}

/// Packs `root/docs` into `root/site`. Returns the number of screenshots.
pub fn pack(root: &Path) -> Result<usize, String> {
    let (docs, site) = (root.join("docs"), root.join("site"));
    let results = docs.join("results");
    if !results.is_dir() {
        return Err(format!("{} is not there: run `tasks docs` first", results.display()));
    }
    // Only what the generated pages show: the old runs' leftovers stay behind.
    let mut refs = BTreeSet::new();
    for page in pages(&docs.join("site")) {
        refs.extend(screenshot_refs(&std::fs::read_to_string(&page).map_err(|e| format!("{}: {e}", page.display()))?));
    }
    for gone in ["results", "screenshots"] {
        let _ = std::fs::remove_dir_all(site.join(gone));
    }
    std::fs::create_dir_all(site.join("results")).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(&results).map_err(|e| e.to_string())? {
        let from = entry.map_err(|e| e.to_string())?.path();
        std::fs::copy(&from, site.join("results").join(from.file_name().unwrap()))
            .map_err(|e| format!("{}: {e}", from.display()))?;
    }
    for r in &refs {
        let (from, to) = (docs.join(r), site.join(r));
        if r.ends_with(".jpg") {
            shrink_jpeg(&from, &to)?;
        } else {
            let svg = std::fs::read_to_string(&from).map_err(|e| format!("{}: {e}", from.display()))?;
            let svg = shrink_svg(&svg).map_err(|e| format!("{}: {e}", from.display()))?;
            std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&to, svg).map_err(|e| format!("{}: {e}", to.display()))?;
        }
    }
    Ok(refs.len())
}

pub fn run(root: &Path) -> i32 {
    match pack(root) {
        Ok(n) => {
            println!("packed the results and {n} screenshots into site/");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sim001-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_jpeg_in_an_svg_is_shrunk() {
        assert_eq!(base64::encode(b"Man"), "TWFu");
        assert_eq!(base64::encode(b"Ma"), "TWE=");
        assert_eq!(base64::decode("TWE=").unwrap(), b"Ma");
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 95).encode_image(&image::RgbImage::new(1920, 1080)).unwrap();
        let svg = format!("<svg><image width=\"1920\" href=\"{JPEG_DATA}{}\"/></svg>", base64::encode(&jpeg));
        let out = shrink_svg(&svg).unwrap();
        assert!(
            out.starts_with("<svg><image width=\"1920\" href=\"data:image/jpeg;base64,") && out.ends_with("\"/></svg>")
        );
        let inner = &out[out.find(JPEG_DATA).unwrap() + JPEG_DATA.len()..out.rfind('"').unwrap()];
        let img = image::load_from_memory(&base64::decode(inner).unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (960, 540));
        assert_eq!(shrink_svg("<svg><rect/></svg>").unwrap(), "<svg><rect/></svg>");
    }

    #[test]
    fn the_screenshots_a_page_shows_are_found() {
        let html = r#"<img src="screenshots/sim--first-throw/00.jpg"> <a href='screenshots/x--y/03.svg'>
            <img src="screenshots/a/b.png"> screenshots/notquoted.jpg more <img src="../screenshots/sim--first-throw/00.jpg">"#;
        let found: Vec<_> = screenshot_refs(html).into_iter().collect();
        assert_eq!(
            found,
            vec!["screenshots/notquoted.jpg", "screenshots/sim--first-throw/00.jpg", "screenshots/x--y/03.svg"]
        );
    }

    #[test]
    fn a_screenshot_is_shrunk_but_never_enlarged() {
        let d = temp("shrink");
        let big = image::RgbImage::from_fn(1920, 1080, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90]));
        big.save(d.join("big.jpg")).unwrap();
        let small = image::RgbImage::from_fn(400, 300, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, 90]));
        small.save(d.join("small.jpg")).unwrap();
        shrink_jpeg(&d.join("big.jpg"), &d.join("out/big.jpg")).unwrap();
        shrink_jpeg(&d.join("small.jpg"), &d.join("out/small.jpg")).unwrap();
        assert_eq!(image::image_dimensions(d.join("out/big.jpg")).unwrap(), (960, 540));
        assert_eq!(image::image_dimensions(d.join("out/small.jpg")).unwrap(), (400, 300));
        assert!(
            std::fs::metadata(d.join("out/big.jpg")).unwrap().len()
                < std::fs::metadata(d.join("big.jpg")).unwrap().len()
        );
    }

    /// The results are copied, and only the screenshots the pages show come along, the old ones'
    /// leftovers staying behind.
    #[test]
    fn packing_takes_the_results_and_the_screenshots_the_pages_show() {
        let root = temp("pack");
        let (docs, site) = (root.join("docs"), root.join("site"));
        std::fs::create_dir_all(docs.join("results")).unwrap();
        std::fs::create_dir_all(docs.join("site")).unwrap();
        std::fs::write(docs.join("results/sim.tsv"), "results").unwrap();
        std::fs::write(
            docs.join("site/section.html"),
            r#"<img src="screenshots/t1/00.jpg"><img src="screenshots/t1/01.svg">"#,
        )
        .unwrap();
        let shot = image::RgbImage::from_fn(1200, 700, |x, _| image::Rgb([(x % 200) as u8, 40, 40]));
        for name in ["t1/00.jpg", "old/00.jpg"] {
            std::fs::create_dir_all(docs.join("screenshots").join(name).parent().unwrap()).unwrap();
            shot.save(docs.join("screenshots").join(name)).unwrap();
        }
        std::fs::write(docs.join("screenshots/t1/01.svg"), "<svg/>").unwrap();
        // A stale file in the site from an earlier run.
        std::fs::create_dir_all(site.join("screenshots/gone")).unwrap();
        std::fs::write(site.join("screenshots/gone/00.jpg"), "stale").unwrap();

        assert_eq!(pack(&root), Ok(2));
        assert_eq!(std::fs::read_to_string(site.join("results/sim.tsv")).unwrap(), "results");
        assert_eq!(image::image_dimensions(site.join("screenshots/t1/00.jpg")).unwrap().0, 960);
        assert_eq!(std::fs::read_to_string(site.join("screenshots/t1/01.svg")).unwrap(), "<svg/>");
        assert!(!site.join("screenshots/old").exists(), "a screenshot no page shows");
        assert!(!site.join("screenshots/gone").exists(), "the stale leftovers are cleared");
    }

    #[test]
    fn packing_with_no_run_says_what_to_do() {
        let root = temp("norun");
        assert!(pack(&root).unwrap_err().contains("tasks docs"));
    }
}
