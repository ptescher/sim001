//! Frame pacing for a 3D view that presents no window (plans/battlefield-latency.md, step 5).
//!
//! Bevy presents a window only when a camera draws into it, and a window's present is what holds
//! the CPU back to the GPU. `towers_view` draws its cameras into images, so nothing held it back:
//! it queued frames while the valley loaded, and the GPU then ran 13 frames (720 ms) behind it for
//! the rest of the game, which is how late each picture of the battlefield was. After each frame
//! the render thread now waits until the GPU has finished the frame [`IN_FLIGHT`] frames before.

use bevy::prelude::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderSystems};
use std::collections::VecDeque;
use std::time::Duration;

/// How many frames the GPU may be behind the CPU. Measured with the view alone, panning
/// (plans/battlefield-latency-results.md): 2 gave pictures 203 ms after their request, 1 gave
/// 155 ms at the same frame rate, 0 gave 122 ms but 13 % fewer pictures.
pub const IN_FLIGHT: usize = 1;
/// The longest the render thread waits for the GPU (a frame the GPU never finishes must not hang
/// the view).
const WAIT_AT_MOST: Duration = Duration::from_millis(500);

/// The frames submitted and not yet waited for, oldest first.
pub struct Pacer<T> {
    submitted: VecDeque<T>,
}

impl<T> Default for Pacer<T> {
    fn default() -> Self {
        Pacer { submitted: VecDeque::new() }
    }
}

impl<T> Pacer<T> {
    /// A frame was submitted: the earlier frame to wait for, once more than [`IN_FLIGHT`] are.
    pub fn submitted(&mut self, frame: T) -> Option<T> {
        self.submitted.push_back(frame);
        if self.submitted.len() > IN_FLIGHT { self.submitted.pop_front() } else { None }
    }
}

#[derive(Resource, Default)]
struct Frames(Pacer<wgpu::SubmissionIndex>);

/// After each frame is rendered, the render thread waits for the GPU to finish the frame
/// [`IN_FLIGHT`] frames before. It waits after Bevy has asked for the frame's readbacks
/// (`map_buffers`, in `Cleanup`), so that the wait also completes the readback of that frame.
pub fn plugin(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<Frames>().add_systems(Render, pace.in_set(RenderSystems::PostCleanup));
    }
}

fn pace(device: Res<RenderDevice>, queue: Res<RenderQueue>, mut frames: ResMut<Frames>) {
    // An empty submission marks the end of this frame's work on the queue.
    let mark = queue.submit([]);
    if let Some(earlier) = frames.0.submitted(mark) {
        let _ = device.poll(wgpu::PollType::Wait { submission_index: Some(earlier), timeout: Some(WAIT_AT_MOST) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_frames_do_not_wait() {
        let mut p = Pacer::default();
        for k in 0..IN_FLIGHT as u32 {
            assert_eq!(p.submitted(k), None, "frame {k}");
        }
    }

    #[test]
    fn then_each_frame_waits_for_the_one_in_flight_frames_before_it() {
        let mut p = Pacer::default();
        for k in 0..IN_FLIGHT as u32 {
            p.submitted(k);
        }
        for k in IN_FLIGHT as u32..20 {
            assert_eq!(p.submitted(k), Some(k - IN_FLIGHT as u32));
        }
    }
}
