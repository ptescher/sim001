//! Loading screen: the world is generated on a background thread while the window stays
//! responsive, then handed to the main thread in stages (assets, then trees in batches),
//! and the ray-traced camera warms up behind the overlay before it fades out.

use crate::theme::{P, color};
use crate::camera::RenderScale;
use simulation_physics::Obstacle;
use crate::terrain::{self, Tree};
use crate::terrain_render;
use crate::vegetation::{self, VegetationBuild, VegetationHandles};
use crate::Obstacles;
use bevy::prelude::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(States, Default, Clone, Copy, Eq, PartialEq, Hash, Debug)]
pub enum AppState {
    #[default]
    Loading,
    Running,
}

/// Everything heavy, produced off the main thread.
pub struct WorldBuild {
    terrain: Vec<(Mesh, Image)>,
    trees: Vec<Tree>,
    obstacles: Vec<Obstacle>,
    vegetation: VegetationBuild,
}

#[derive(Default)]
struct Shared {
    fraction: f32,
    stage: String,
    result: Option<WorldBuild>,
}

enum Phase {
    /// Background generation running.
    Generating,
    /// Uploading terrain chunks to the GPU, a texel budget per frame.
    Uploading { terrain: Vec<(Mesh, Image)>, total: usize, trees: Vec<Tree>, vegetation: Option<VegetationBuild> },
    /// Spawning trees in batches.
    Planting { trees: Vec<Tree>, handles: VegetationHandles, next: usize },
    /// Camera on behind the overlay so pipelines compile and the BVH builds.
    WarmingUp { since: Instant, frames: u32 },
    /// Overlay fading out.
    Fading { since: Instant },
}

#[derive(Resource)]
pub struct Loading {
    shared: Arc<Mutex<Shared>>,
    phase: Phase,
    started: Instant,
    shown_fraction: f32,
}

#[derive(Component)]
struct LoadingRoot;
#[derive(Component)]
struct LoadingBar;
#[derive(Component)]
struct LoadingStage;
#[derive(Component)]
struct LoadingTip;

const TREES_PER_FRAME: usize = 800;
/// Shown one at a time on the loading screen, five seconds each.
#[derive(Resource, Clone, Default)]
pub struct Tips(pub Vec<&'static str>);

pub fn loading_plugin(app: &mut App) {
    app.init_state::<AppState>()
        .add_systems(Startup, start_loading)
        .add_systems(Update, (advance_loading, update_loading_ui).chain().run_if(resource_exists::<Loading>));
}

fn start_loading(mut commands: Commands) {
    let shared = Arc::new(Mutex::new(Shared { stage: "Starting".into(), ..default() }));
    let worker = shared.clone();
    std::thread::Builder::new()
        .name("world-gen".into())
        .spawn(move || {
            terrain_render::background_priority();
            let set = |f: f32, s: &str| {
                let mut g = worker.lock().unwrap();
                g.fraction = f;
                g.stage = s.to_string();
            };
            set(0.01, &format!("Surveying {}", terrain::site_title()));
            let trees = terrain::trees();
            let obstacles = terrain::obstacles(&trees, &terrain::buildings());
            set(0.06, "Baking terrain");
            let terrain = terrain_render::build_terrain(|done, total| {
                set(0.06 + 0.70 * done as f32 / total as f32, &format!("Baking terrain  {done}/{total}"));
            });
            let mut step = 0.0;
            let vegetation = vegetation::build_vegetation(|stage| {
                step += 1.0;
                set(0.76 + 0.04 * step, stage);
            });
            let mut g = worker.lock().unwrap();
            g.fraction = 0.88;
            g.stage = "Uploading to the GPU".into();
            g.result = Some(WorldBuild { terrain, trees, obstacles, vegetation });
        })
        .expect("spawn world generation thread");

    commands.insert_resource(Loading { shared, phase: Phase::Generating, started: Instant::now(), shown_fraction: 0.0 });
    spawn_loading_ui(&mut commands);
}

#[allow(clippy::too_many_arguments)]
fn advance_loading(
    mut commands: Commands,
    mut loading: ResMut<Loading>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut render_scale: ResMut<RenderScale>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let loading = &mut *loading;
    match &mut loading.phase {
        Phase::Generating => {
            // Keep the ray-traced view tiny while generating: the GPU isn't needed yet.
            render_scale.suspended = true;
            let result = loading.shared.lock().unwrap().result.take();
            if let Some(build) = result {
                commands.insert_resource(Obstacles(build.obstacles));
                let total = build.terrain.len();
                loading.phase = Phase::Uploading {
                    terrain: build.terrain.into_iter().rev().collect(),
                    total,
                    trees: build.trees,
                    vegetation: Some(build.vegetation),
                };
            }
        }
        Phase::Uploading { terrain, total, trees, vegetation } => {
            // ~6 Mtexels per frame keeps upload hitches small (the centre chunk goes alone).
            let mut budget: u64 = 6_000_000;
            let mut batch = Vec::new();
            while let Some((_, img)) = terrain.last() {
                let texels = img.width() as u64 * img.height() as u64;
                if !batch.is_empty() && texels > budget {
                    break;
                }
                budget = budget.saturating_sub(texels);
                batch.push(terrain.pop().unwrap());
            }
            terrain_render::spawn_terrain_chunks(batch, &mut commands, &mut meshes, &mut materials, &mut images);
            let done = *total - terrain.len();
            set_stage(&loading.shared, 0.88 + 0.02 * done as f32 / *total as f32, &format!("Uploading terrain  {done}/{total}"));
            if terrain.is_empty() {
                let handles = vegetation::upload_vegetation(vegetation.take().unwrap(), &mut commands, &mut meshes, &mut materials, &mut images);
                set_stage(&loading.shared, 0.90, "Planting the forest");
                loading.phase = Phase::Planting { trees: std::mem::take(trees), handles, next: 0 };
            }
        }
        Phase::Planting { trees, handles, next } => {
            let end = (*next + TREES_PER_FRAME).min(trees.len());
            for i in *next..end {
                vegetation::spawn_tree(&mut commands, handles, i, &trees[i]);
            }
            *next = end;
            set_stage(
                &loading.shared,
                0.90 + 0.06 * end as f32 / trees.len().max(1) as f32,
                &format!("Planting the forest  {end}/{}", trees.len()),
            );
            if end >= trees.len() {
                info!("spawned {} trees", trees.len());
                commands.insert_resource(handles.clone());
                render_scale.suspended = false;
                set_stage(&loading.shared, 0.97, "Warming up the ray tracer");
                loading.phase = Phase::WarmingUp { since: Instant::now(), frames: 0 };
            }
        }
        Phase::WarmingUp { since, frames } => {
            *frames += 1;
            // Give pipeline compilation and the first BVH builds a moment behind the overlay.
            if *frames > 45 && since.elapsed().as_secs_f32() > 1.5 {
                set_stage(&loading.shared, 1.0, "Ready");
                info!("world ready in {:.1} s", loading.started.elapsed().as_secs_f32());
                next_state.set(AppState::Running);
                loading.phase = Phase::Fading { since: Instant::now() };
            }
        }
        Phase::Fading { .. } => {}
    }
}

fn set_stage(shared: &Arc<Mutex<Shared>>, f: f32, s: &str) {
    let mut g = shared.lock().unwrap();
    g.fraction = f;
    g.stage = s.to_string();
}

fn spawn_loading_ui(commands: &mut Commands) {
    let text = |size: f32| TextFont { font_size: FontSize::Px(size), ..default() };
    commands.spawn((
        LoadingRoot,
        Node {
            position_type: PositionType::Absolute,
            width: percent(100.0),
            height: percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: px(14.0),
            ..default()
        },
        // The attitude indicator's sky and ground either side of the console, in the shared
        // theme's colours (see crate::theme).
        BackgroundGradient::from(LinearGradient::to_bottom(vec![
            color(P.sky).into(),
            color(P.canvas).into(),
            color(P.ground).into(),
        ])),
        GlobalZIndex(i32::MAX - 1),
        children![
            (Text::new("HEEWING T1-S Ranger"), text(44.0), TextColor(color(P.text))),
            (Text::new(terrain::site_title().to_lowercase()), text(18.0), TextColor(color(P.muted))),
            (
                Node {
                    width: px(520.0),
                    height: px(10.0),
                    margin: UiRect::top(px(26.0)),
                    border_radius: BorderRadius::all(px(5.0)),
                    ..default()
                },
                BackgroundColor(color(P.screen)),
                children![(
                    LoadingBar,
                    Node { width: percent(0.0), height: percent(100.0), border_radius: BorderRadius::all(px(5.0)), ..default() },
                    BackgroundColor(color(P.ok)),
                )],
            ),
            (LoadingStage, Text::new(""), text(15.0), TextColor(color(P.text))),
            (
                LoadingTip,
                Text::new(""),
                text(14.0),
                TextColor(color(P.muted)),
                Node { margin: UiRect::top(px(40.0)), max_width: px(640.0), ..default() },
            ),
        ],
    ));
}

#[allow(clippy::type_complexity)]
fn update_loading_ui(
    mut commands: Commands,
    time: Res<Time>,
    mut loading: ResMut<Loading>,
    tips: Res<Tips>,
    mut bar: Query<&mut Node, With<LoadingBar>>,
    mut texts: ParamSet<(Query<&mut Text, With<LoadingStage>>, Query<&mut Text, With<LoadingTip>>)>,
    mut root: Query<(Entity, &mut BackgroundGradient), With<LoadingRoot>>,
    mut colors: Query<&mut TextColor>,
    children: Query<&Children>,
    mut bgs: Query<&mut BackgroundColor>,
) {
    let (fraction, stage) = {
        let g = loading.shared.lock().unwrap();
        (g.fraction, g.stage.clone())
    };
    // Ease the bar toward the reported progress so it moves smoothly.
    let dt = time.delta_secs().min(0.1);
    loading.shown_fraction += (fraction - loading.shown_fraction) * (1.0 - (-dt * 6.0).exp());
    for mut node in &mut bar {
        node.width = percent(loading.shown_fraction * 100.0);
    }
    for mut t in &mut texts.p0() {
        let elapsed = loading.started.elapsed().as_secs_f32();
        t.0 = format!("{stage}   ({elapsed:.0} s)");
    }
    let tip = tips.0.get((loading.started.elapsed().as_secs() / 5) as usize % tips.0.len().max(1)).copied().unwrap_or("");
    for mut t in &mut texts.p1() {
        if t.0 != tip {
            t.0 = tip.to_string();
        }
    }

    // Fade out, then remove the overlay and this resource.
    if let Phase::Fading { since } = loading.phase {
        let a = 1.0 - (since.elapsed().as_secs_f32() / 0.6).clamp(0.0, 1.0);
        for (root_e, mut grad) in &mut root {
            let BackgroundGradient(gs) = &mut *grad;
            for g in gs.iter_mut() {
                if let Gradient::Linear(lg) = g {
                    for stop in lg.stops.iter_mut() {
                        stop.color = stop.color.with_alpha(a);
                    }
                }
            }
            for e in children.iter_descendants(root_e) {
                if let Ok(mut c) = colors.get_mut(e) {
                    c.0 = c.0.with_alpha(c.0.alpha().min(a));
                }
                if let Ok(mut b) = bgs.get_mut(e) {
                    b.0 = b.0.with_alpha(b.0.alpha().min(a));
                }
            }
            if a <= 0.0 {
                commands.entity(root_e).despawn();
                commands.remove_resource::<Loading>();
            }
        }
    }
}
