use fresco_example_engine::profile::{FrameInputs, camera::OrbitCamera, preview::PreviewShape};

fn frame() -> FrameInputs {
    FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [640, 480],
    }
}

#[test]
fn orbit_view_centers_target_and_maps_camera_to_origin() {
    let mut camera = OrbitCamera::default();
    let initial = camera.scene(PreviewShape::Sphere, frame());
    assert_eq!(initial.view, PreviewShape::Sphere.scene(frame()).view);
    for delta in [[70.0, 35.0], [-200.0, -60.0], [f32::MAX, f32::MAX]] {
        camera.drag(delta).unwrap();
        let scene = camera.scene(PreviewShape::Box, frame());
        scene.pack().unwrap();
        for row in 0..3 {
            let eye = (0..3)
                .map(|col| scene.view[col * 4 + row] * scene.camera_position[col])
                .sum::<f32>()
                + scene.view[12 + row];
            assert!(eye.abs() < 1.0e-5);
            for other in 0..3 {
                let dot = (0..3)
                    .map(|col| scene.view[col * 4 + row] * scene.view[col * 4 + other])
                    .sum::<f32>();
                assert!((dot - if row == other { 1.0 } else { 0.0 }).abs() < 1.0e-5);
            }
        }
        assert_eq!(scene.view[12], 0.0);
        assert_eq!(scene.view[13], 0.0);
        assert!(scene.view[14] < 0.0);
    }
}

#[test]
fn camera_limits_and_invalid_input_preserve_finite_state() {
    let mut camera = OrbitCamera::default();
    camera.zoom(f32::MAX).unwrap();
    assert_eq!(
        camera.scene(PreviewShape::Sphere, frame()).camera_position,
        [0.0, 0.0, 8.0]
    );
    camera.zoom(-f32::MAX).unwrap();
    assert_eq!(
        camera.scene(PreviewShape::Sphere, frame()).camera_position,
        [0.0, 0.0, 1.1]
    );
    camera.drag([0.0, f32::MAX]).unwrap();
    let scene = camera.scene(PreviewShape::Plane, frame());
    assert!((scene.camera_position[1] / 1.1 - 1.35_f32.sin()).abs() < 1.0e-6);
    let before = camera;
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(camera.drag([10.0, invalid]).is_err());
        assert!(camera.drag([invalid, 10.0]).is_err());
        assert!(camera.zoom(invalid).is_err());
        assert_eq!(camera, before);
    }
}

#[test]
fn inverse_wheel_deltas_restore_camera_distance() {
    let mut camera = OrbitCamera::default();
    let initial = camera.scene(PreviewShape::Sphere, frame()).camera_position[2];
    camera.zoom(-120.0).unwrap();
    assert!(camera.scene(PreviewShape::Sphere, frame()).camera_position[2] < initial);
    camera.zoom(120.0).unwrap();
    assert!(
        (camera.scene(PreviewShape::Sphere, frame()).camera_position[2] - initial).abs() < 1.0e-6
    );
}
