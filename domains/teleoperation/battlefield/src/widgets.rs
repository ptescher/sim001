//! The widgets of the bars: titled panels, the buttons of a command card with their keys, and
//! camera tiles. Each one records where it is painted, so a test driver clicks it where a person would.

use crate::draw::*;
use crate::theme::*;
use gpui::{
    Bounds, Corners, Div, IntoElement, Pixels, RenderImage, SharedString, Stateful, canvas, div, prelude::*, px,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    /// Where each button was last painted, by id, so `--expect` can click it the way a person
    /// would. Written while painting, read between frames; both on the UI thread.
    static BUTTONS: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// Where the button with this id is on screen now, if it is.
pub fn button_bounds(id: &str) -> Option<Bounds<Pixels>> {
    BUTTONS.with(|b| b.borrow().get(id).copied())
}

/// Records where the element with `id` is painted, for `--expect`.
pub fn recorder(id: SharedString) -> impl IntoElement {
    canvas(move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(id, b)), |_, _, _, _| {}).absolute().inset_0()
}

/// A button of the command card: a square with its name, and its key in the corner.
pub fn command(id: &'static str, name: &'static str, key: &'static str, active: bool) -> Stateful<Div> {
    let (bg, fg, border) = if active { (ACTIVE_FILL, GREEN, GREEN_DIM) } else { (CONTROL, WHITE, BEZEL_EDGE) };
    div()
        .id(id)
        .relative()
        .w(px(62.0))
        .h(px(52.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(c(bg))
        .text_color(c(fg))
        .text_sm()
        .border_1()
        .border_color(c(border))
        .rounded(px(CONTROL_RADIUS))
        .cursor_pointer()
        .hover(|s| s.bg(c(CONTROL_HOVER)))
        .active(|s| s.opacity(0.7))
        .child(name)
        .child(div().absolute().top(px(2.0)).right(px(4.0)).text_xs().text_color(c(AMBER)).child(key))
        .child(recorder(id.into()))
}

/// A titled panel of the bars, as on the console.
pub fn panel(title: &'static str, body: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .min_h(px(0.0))
        .bg(c(BEZEL))
        .border_1()
        .border_color(c(BEZEL_EDGE))
        .rounded(px(PANEL_RADIUS))
        .p(px(BEZEL_PAD))
        .gap(px(4.0))
        .child(div().px_2().bg(c(TITLE_BG)).text_color(c(GREY)).text_xs().child(title))
        .child(div().flex_1().min_h(px(0.0)).bg(c(SCREEN)).rounded(px(SCREEN_RADIUS)).overflow_hidden().child(body))
}

/// A camera's picture (or NO SIGNAL, or NO LINK), `w` by `h` pixels, with its name over it.
pub fn camera_tile(
    id: String,
    label: &str,
    image: Option<Arc<RenderImage>>,
    no_link: bool,
    w: f32,
    h: f32,
    selected: bool,
) -> Stateful<Div> {
    let recorded: SharedString = id.clone().into();
    let label = label.to_string();
    div()
        .id(SharedString::from(id))
        .relative()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .border_1()
        .border_color(c(if selected { WHITE } else { BEZEL_EDGE }))
        .cursor_pointer()
        .child(
            canvas(
                move |b, _, _| BUTTONS.with(|m| m.borrow_mut().insert(recorded, b)),
                move |b, _, window, cx| {
                    let (x, y, w, h) = rect(b);
                    fill_rect(window, x, y, w, h, c(0x000000));
                    match &image {
                        Some(img) => {
                            let _ = window.paint_image(b, Corners::default(), img.clone(), 0, false);
                        }
                        None => {
                            let message = if no_link { "NO LINK" } else { "NO SIGNAL" };
                            text(window, cx, message, x + w * 0.5, y + h * 0.5, 10.0, c(GREY), Align::Center)
                        }
                    }
                    text(window, cx, &label, x + 6.0, y + 9.0, 10.0, c(WHITE), Align::Left);
                },
            )
            .absolute()
            .inset_0(),
        )
}
