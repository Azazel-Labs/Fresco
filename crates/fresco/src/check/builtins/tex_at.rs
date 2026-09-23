//! Internal helper for texture `.at(...)` syntax.

use crate::ast::Expr;
use crate::builtin;
use crate::check::Value;
use crate::hir::{Layer, Sx};

builtin! {
    name = "tex_at",
    signature = single {
        args(
            texture: Expr = "texture name identifier",
            at: Optional<Vec2> = "explicit sample coordinate"
        ),
        result = Layer,
        caps = PURE,
    },
    check = |ctx, texture, at| {
        let tex_name = match &texture.node {
            Expr::Var(n) => n.clone(),
            _ => {
                ctx.diags.push(
                    crate::diag::Diag::error(
                        texture.span.clone(),
                        "invalid `tex.at(...)` receiver; expected a texture identifier",
                    )
                    .with_help("example: `albedo.at(uv)` or `orm.at(uv * 2.0).roughness`"),
                );
                return None;
            }
        };

        let Some(sample_at) = at else {
            ctx.diags.push(
                crate::diag::Diag::error(
                    texture.span.clone(),
                    "`tex.at(...)` requires an explicit sample coordinate",
                )
                .with_help("example: `albedo.at(uv)`"),
            );
            return None;
        };
        let (sample_x, sample_y) = sample_at.clone();

        ctx.hir.register_texture(&tex_name);
        ctx.hir.notes.push(format!(
            "texture: tex_at({tex_name}, ...) registered at group=1, binding assigned by lowering"
        ));

        let layer_id = ctx.hir.layer(Layer::ImageAt {
            tex_name: tex_name.clone(),
            sample_x,
            sample_y,
        });

        let type_name = ctx
            .hir
            .texture_metadata
            .get(&tex_name)
            .and_then(|m| m.texture_type_name.clone());

        if let Some(type_name) = type_name {
            if let Some(def) = ctx.hir.texture_type_defs.get(&type_name) {
                let result_expr = def.result_expr.clone();
                let channels = def.channels.clone();
                if let Some(result_expr) = result_expr {
                    let sample_at_vec = Some(Box::new(sample_at));
                    ctx.scopes.push(std::collections::HashMap::new());
                    for ch_def in &channels {
                        let channel = Sx::TexChannel {
                            tex_name: tex_name.clone(),
                            channel: ch_def.channel_idx,
                            sample_at: sample_at_vec.clone(),
                            decode_mul: ch_def.decode_mul,
                            decode_add: ch_def.decode_add,
                            decode_expr: ch_def.decode_expr.clone().map(Box::new),
                        };
                        ctx.bind(ch_def.channel_name.clone(), Value::Scalar(channel.clone()));
                        ctx.bind(ch_def.semantic_name.clone(), Value::Scalar(channel));
                    }
                    let value = ctx.eval(&result_expr);
                    ctx.scopes.pop();
                    return value;
                }
            }
            Value::TypedTextureSample {
                layer_id,
                tex_name,
                sample_at: Some(sample_at),
            }
        } else {
            Value::Layer(layer_id)
        }
    }
}
