use super::*;

/// Looking down −z at x, y in −10..10 (orthographic): depth 0 at z = 50, 1 at z = −50.
const VIEW: [[f32; 4]; 4] = [[0.1, 0.0, 0.0, 0.0], [0.0, 0.1, 0.0, 0.0], [0.0, 0.0, -0.01, 0.0], [0.0, 0.0, 0.5, 1.0]];
const SIZE: u32 = 64;

/// The pixel of model point (x, y).
fn px(x: f32, y: f32) -> [i64; 2] {
    [((x / 10.0 + 1.0) * 0.5 * SIZE as f32) as i64, ((1.0 - y / 10.0) * 0.5 * SIZE as f32) as i64]
}

/// A square face at z = 0 (x, y in −5..5), and three edges: 1 on it, 2 well behind it, 3 behind
/// it by less than the lift.
fn scene() -> PickScene {
    let mut s = PickScene::default();
    let (a, b, c, d) = ([-5.0, -5.0, 0.0], [5.0, -5.0, 0.0], [5.0, 5.0, 0.0], [-5.0, 5.0, 0.0]);
    s.triangle([a, b, c]);
    s.triangle([a, c, d]);
    s.segment([-8.0, 0.0, 0.0], [8.0, 0.0, 0.0], 1);
    s.segment([2.0, -8.0, -3.0], [2.0, 8.0, -3.0], 2);
    s.segment([-2.0, -8.0, -0.2], [-2.0, 8.0, -0.2], 3);
    s
}

#[test]
fn the_pick_buffer_names_the_edges_seen_at_each_pixel() {
    let (_, device, queue) = match crate::open_headless() {
        Ok(d) => d,
        Err(e) => {
            assert!(std::env::var_os("SOLIDCRAFT_REQUIRE_GPU").is_none(), "a GPU is required here: {e}");
            eprintln!("skipping: {e}");
            return;
        }
    };
    let mut pb = PickBuffer::new(&device);
    let out = PickOut::default();
    let view = PickView { key: 7, view_proj: VIEW, eye: [0.0, 0.0, 100.0, 0.0], back: [0.0, 0.0, 1.0], lift: 0.5, size: [SIZE, SIZE] };
    let scene = scene();
    let all = [0, 0, SIZE as i64 - 1, SIZE as i64 - 1];
    for _ in 0..8 {
        let mut encoder = device.create_command_encoder(&Default::default());
        pb.prepare(&device, &queue, &mut encoder, 1, &scene, &view, Some([SIZE / 2, SIZE / 2]), &out);
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if out.lock().unwrap().tile(7, [SIZE, SIZE], all).is_some() {
            break;
        }
    }
    let results = out.lock().unwrap();
    let tile = results.tile(7, [SIZE, SIZE], all).expect("the tile is read back within a few frames");
    let at = |x: f32, y: f32| {
        let [px, py] = px(x, y);
        tile.ids_in([px - 1, py - 1, px + 1, py + 1])
    };
    assert_eq!(at(0.0, 0.0), vec![1], "the edge on the face");
    assert_eq!(at(2.0, 3.0), Vec::<u32>::new(), "the edge behind the face is hidden");
    assert_eq!(at(2.0, 7.5), vec![2], "and seen where the face does not cover it");
    assert_eq!(at(-2.0, 3.0), vec![3], "an edge behind the face by less than the lift counts as seen");
    assert_eq!(at(3.5, 3.5), Vec::<u32>::new(), "the face itself names no edge");
    assert!(results.tile(8, [SIZE, SIZE], all).is_none(), "another view's tile does not answer");
    drop(results);
    assert!(!pb.waiting());
}

#[test]
fn tiles_cover_the_window_around_the_cursor_inside_the_view() {
    assert_eq!(tile_around([10, 10], [1000, 800]), ([0, 0], [TILE, TILE]));
    assert_eq!(tile_around([500, 400], [1000, 800]), ([500 - TILE / 2, 400 - TILE / 2], [TILE, TILE]));
    assert_eq!(tile_around([999, 799], [1000, 800]), ([1000 - TILE, 800 - TILE], [TILE, TILE]));
    assert_eq!(tile_around([30, 20], [50, 40]), ([0, 0], [50, 40]));
    let t = PickTile { key: 1, origin: [100, 100], size: [TILE, TILE], ids: vec![0; (TILE * TILE) as usize] };
    assert!(t.covers(1, [1000, 800], [110, 110, 120, 120]));
    assert!(!t.covers(1, [1000, 800], [90, 110, 120, 120]));
    assert!(!t.covers(2, [1000, 800], [110, 110, 120, 120]));
    // A window past the view's border needs only the pixels inside it.
    let corner = PickTile { key: 1, origin: [0, 0], size: [TILE, TILE], ids: vec![5; (TILE * TILE) as usize] };
    assert!(corner.covers(1, [1000, 800], [-8, -8, 8, 8]));
    assert_eq!(corner.ids_in([-8, -8, 8, 8]), vec![5]);
}
