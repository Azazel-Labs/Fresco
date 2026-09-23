use fresco_example_engine::profile::{
    FrameInputs,
    preview::{PreviewGeometry, PreviewShape, sphere_scene},
};

#[test]
fn plane_preview_projects_to_visible_area() {
    let scene = PreviewShape::Plane.scene(FrameInputs {
        time: 0.0,
        delta_time: 0.0,
        physical_size: [640, 640],
    });
    scene.pack().unwrap();
    let transform = |matrix: &[f32; 16], point: [f32; 4]| -> [f32; 4] {
        std::array::from_fn(|row| (0..4).map(|col| matrix[col * 4 + row] * point[col]).sum())
    };
    let projected: Vec<_> = [
        [-1.0, 0.0, -1.0, 1.0],
        [-1.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, -1.0, 1.0],
    ]
    .map(|point| {
        let clip = transform(
            &scene.projection,
            transform(&scene.view, transform(&scene.model, point)),
        );
        let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
        assert!((-1.0..=1.0).contains(&ndc[0]));
        assert!((-1.0..=1.0).contains(&ndc[1]));
        assert!((0.0..=1.0).contains(&ndc[2]));
        ndc
    })
    .into();
    let a = [
        projected[1][0] - projected[0][0],
        projected[1][1] - projected[0][1],
    ];
    let b = [
        projected[2][0] - projected[0][0],
        projected[2][1] - projected[0][1],
    ];
    assert!(
        a[0] * b[1] - a[1] * b[0] > 0.5,
        "front-facing plane occupies meaningful screen area"
    );
}

#[test]
fn plane_and_cube_preserve_winding_normals_uvs_and_surface_area() {
    for (mesh, vertices, triangles, expected_area) in [
        (PreviewGeometry::plane(), 289, 512, 4.0),
        (PreviewGeometry::cube(), 24, 12, 24.0),
    ] {
        assert_eq!(mesh.vertex_count, vertices);
        assert_eq!(mesh.indices.len(), triangles * 3);
        assert_eq!(mesh.positions.len(), vertices as usize * 3);
        assert_eq!(mesh.normals.len(), mesh.positions.len());
        assert_eq!(mesh.tangents.len(), mesh.positions.len());
        assert_eq!(mesh.uv.len(), vertices as usize * 2);
        assert_eq!(mesh.uv2.len(), mesh.uv.len());
        for (uv, uv2) in mesh.uv.iter().zip(&mesh.uv2) {
            assert!((0.0..=1.0).contains(uv));
            assert_eq!(*uv2, *uv * 2.0);
        }
        for (normal, tangent) in mesh
            .normals
            .as_chunks::<3>()
            .0
            .iter()
            .zip(mesh.tangents.as_chunks::<3>().0)
        {
            assert_eq!(normal.iter().map(|v| v * v).sum::<f32>(), 1.0);
            assert_eq!(tangent.iter().map(|v| v * v).sum::<f32>(), 1.0);
            assert_eq!(
                normal.iter().zip(tangent).map(|(a, b)| a * b).sum::<f32>(),
                0.0
            );
        }
        let mut area = 0.0;
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let points: Vec<_> = triangle
                .iter()
                .map(|index| {
                    assert!(*index < vertices);
                    let start = *index as usize * 3;
                    &mesh.positions[start..start + 3]
                })
                .collect();
            let a: [f32; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
            let b: [f32; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
            let cross = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let start = triangle[0] as usize * 3;
            assert!(
                (0..3)
                    .map(|i| cross[i] * mesh.normals[start + i])
                    .sum::<f32>()
                    > 0.0
            );
            area += cross.iter().map(|v| v * v).sum::<f32>().sqrt() * 0.5;
        }
        assert!((area - expected_area).abs() < 1.0e-5);
    }
}

#[test]
fn preview_sphere_has_outward_winding_and_distinct_uv_streams() {
    let mesh = PreviewGeometry::sphere();
    assert_eq!(mesh.vertex_count, 65 * 65);
    assert_eq!(mesh.indices.len(), 64 * 64 * 6);
    assert_eq!(mesh.positions.len(), mesh.vertex_count as usize * 3);
    assert_eq!(mesh.normals, mesh.positions);
    assert_eq!(mesh.tangents.len(), mesh.positions.len());
    assert_eq!(mesh.uv.len(), mesh.vertex_count as usize * 2);
    assert_eq!(mesh.uv2.len(), mesh.uv.len());
    assert_ne!(mesh.uv, mesh.uv2);
    for normal in mesh.normals.as_chunks::<3>().0 {
        assert!((normal.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1.0e-5);
    }
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let points: Vec<_> = triangle
            .iter()
            .map(|index| {
                assert!(*index < mesh.vertex_count);
                let start = *index as usize * 3;
                &mesh.positions[start..start + 3]
            })
            .collect();
        let a: [f32; 3] = std::array::from_fn(|i| points[1][i] - points[0][i]);
        let b: [f32; 3] = std::array::from_fn(|i| points[2][i] - points[0][i]);
        let cross = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        if cross.iter().map(|v| v * v).sum::<f32>() > 1.0e-12 {
            assert!((0..3).map(|i| cross[i] * points[0][i]).sum::<f32>() > 0.0);
        }
    }
}

#[test]
fn preview_camera_uses_webgpu_depth_and_tracks_aspect_ratio() {
    let frame = FrameInputs {
        time: 2.0,
        delta_time: 0.25,
        physical_size: [640, 640],
    };
    let square = sphere_scene(frame);
    square.pack().unwrap();
    for (z, expected) in [(-0.1, 0.0), (-100.0, 1.0)] {
        let clip_z = square.projection[10] * z + square.projection[14];
        let clip_w = square.projection[11] * z + square.projection[15];
        assert!((clip_z / clip_w - expected).abs() < 1.0e-6);
    }
    let wide = sphere_scene(FrameInputs {
        physical_size: [1280, 640],
        ..frame
    });
    assert_eq!(wide.projection[0] * 2.0, square.projection[0]);
    assert_eq!(wide.projection[5], square.projection[5]);
    assert_eq!(wide.frame.time, frame.time);
}
