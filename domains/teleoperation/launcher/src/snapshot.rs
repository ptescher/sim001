//! A picture of the window, for the screenshots of `--expect --docs`: GPUI renders it off screen
//! (`Window::capture_image`, vendor/gpui), so it needs neither the window nor the display to be
//! visible, nor the Screen Recording permission. Where GPUI can't (not macOS) there is none, and
//! the docs say "Screenshot to come".

/// The window as a JPEG, or `None` when there is no picture of it.
pub fn window_jpeg(window: &mut gpui::Window, cx: &mut gpui::App) -> Option<Vec<u8>> {
    let rgb = image::DynamicImage::ImageRgba8(window.capture_image(cx)?).into_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&rgb).ok()?;
    Some(jpeg)
}
