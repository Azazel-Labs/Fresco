use fresco_artifact::{ManifestPassPlan, ManifestRoot};
use fresco_example_engine::runtime::pass_plan::ValidatedPassPlan;

fn plan() -> ManifestPassPlan {
    serde_json::from_value(serde_json::json!({
        "passes": [
            {"id": 91, "stage": 2, "locality": "point", "start_layer": 2, "end_layer": 2, "count": 1, "kernel_strategy": "fused", "entry_point": "finish", "inputs": [{"from_pass": 40, "target_id": 7, "binding": 3}]},
            {"id": 40, "stage": 1, "locality": "local", "start_layer": 1, "end_layer": 1, "count": 1, "kernel_strategy": "inline-taps", "entry_point": "blur", "output_target": 7, "inputs": [{"from_pass": 12, "target_id": 5, "binding": 2}]},
            {"id": 12, "stage": 0, "locality": "point", "start_layer": 0, "end_layer": 0, "count": 1, "kernel_strategy": "fused", "entry_point": "source", "output_target": 5}
        ],
        "edges": [{"from": 12, "to": 40, "reason": "materialize"}, {"from": 40, "to": 91, "reason": "sample"}],
        "targets": [{"id": 5, "format": "rgba16float", "scale": 0.5, "lifetime": "transient"}, {"id": 7, "format": "rgba16float", "scale": 1, "lifetime": "transient"}]
    })).unwrap()
}

#[test]
fn schedules_sparse_ids_by_dependencies_and_resolves_sizes_before_allocation() {
    let validated = ValidatedPassPlan::new(&plan()).unwrap();
    assert_eq!(
        validated.passes().map(|pass| pass.id).collect::<Vec<_>>(),
        [12, 40, 91]
    );
    assert_eq!(validated.final_pass(), 91);
    let sizes = validated.target_sizes([101, 55], 1024).unwrap();
    assert_eq!(
        sizes
            .iter()
            .map(|(target, size)| (target.id, *size))
            .collect::<Vec<_>>(),
        [(5, [51, 28]), (7, [101, 55])]
    );
    assert!(validated.target_sizes([2048, 55], 1024).is_err());
    assert!(validated.target_sizes([0, 55], 1024).unwrap().is_empty());
    assert_eq!(validated.target_sizes([1, 1], 1024).unwrap()[0].1, [1, 1]);
}

#[test]
fn malformed_resource_graphs_fail_before_gpu_preparation() {
    let mutations: &[fn(&mut ManifestPassPlan)] = &[
        |p| p.passes.clear(),
        |p| p.passes[1].id = 91,
        |p| p.passes[1].entry_point.clear(),
        |p| p.targets[1].id = 5,
        |p| p.targets[1].scale = f32::NAN,
        |p| p.targets[1].scale = 0.0,
        |p| p.targets[1].scale = -1.0,
        |p| p.targets[1].lifetime = "frame".into(),
        |p| p.targets[1].format = "unknown".into(),
        |p| p.passes[1].output_target = Some(999),
        |p| p.passes[1].output_target = Some(5),
        |p| p.passes[1].output_target = None,
        |p| p.passes[0].output_target = Some(7),
        |p| p.edges[0].from = 999,
        |p| p.edges.push(p.edges[0].clone()),
        |p| p.edges[0].from = 91,
        |p| p.passes[0].inputs[0].target_id = 5,
        |p| {
            let input = p.passes[0].inputs[0].clone();
            p.passes[0].inputs.push(input);
        },
        |p| {
            p.edges.remove(0);
        },
        |p| p.passes[0].inputs.clear(),
        |p| {
            p.edges[0].from = 40;
            p.passes[1].inputs[0].from_pass = 40;
            p.passes[1].inputs[0].target_id = 7;
            p.edges.push(fresco_artifact::ManifestEdge {
                from: 12,
                to: 91,
                reason: "branch".into(),
            });
            p.passes[0].inputs.push(fresco_artifact::ManifestPassInput {
                from_pass: 12,
                target_id: 5,
                binding: 4,
            });
        },
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut broken = plan();
        mutate(&mut broken);
        assert!(
            ValidatedPassPlan::new(&broken).is_err(),
            "mutation {index} must be rejected"
        );
    }
}

#[test]
fn validates_real_compiler_plans_for_point_and_local_effects() {
    for body in [
        "fill(#ff0000)",
        "compose { grey(ctx.uv.x) |> blur(radius: 2px) }",
    ] {
        let mut files = fresco_example_engine::source_files();
        files.insert(
            "main.fr".into(),
            format!("canvas probe(ctx: CanvasContext) -> color {{ {body} }}"),
        );
        let compiled = fresco::driver::compile_bundle_virtual(&files, "main.fr", false).unwrap();
        let manifest: ManifestRoot = serde_json::from_str(&compiled.manifest).unwrap();
        let source = &manifest.canvases[0].pass_plan;
        let validated = ValidatedPassPlan::new(source).unwrap();
        assert_eq!(validated.passes().count(), source.passes.len());
        if body.contains("blur") {
            assert!(
                source.passes.len() > 1,
                "exercise a real resource dependency"
            );
        }
        assert_eq!(
            validated.passes().last().unwrap().id,
            validated.final_pass()
        );
    }
}
