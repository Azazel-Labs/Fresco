//! Image texture builtin.

use crate::ast::Expr;
use crate::builtin;
use crate::check::Value;
use crate::hir::{Layer, Sx};

builtin! {
    name = "image",
    signature = single {
        args(
            texture: Expr = "texture name identifier",
            at: Optional<Vec2> = "optional sample coordinate"
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
                    "`image` expects a texture name identifier",
                )
                .with_help(
                    "example: `image(my_texture)` — the name becomes the texture binding slot",
                ),
            );
            return None;
        }
        };

        if at.is_none() {
            let _ = ctx.implicit_texture_uv_allowed(
                texture.span.clone(),
                &tex_name,
                "sample explicitly",
                "use `image(tex, at: uv)` or `tex.at(uv)`; staged rollout: `#pragma check.warn_implicit_texture_uv = true` for warnings or `#pragma check.allow_implicit_texture_uv = true` to suppress the diagnostic",
            );
        }

        ctx.hir.register_texture(&tex_name);
        ctx.hir.notes.push(format!(
            "texture: image({tex_name}) registered at group=1, binding assigned by lowering"
        ));
        let sample_at = at;
        let layer_id = if let Some((sample_x, sample_y)) = sample_at.clone() {
            ctx.hir.layer(Layer::ImageAt {
                tex_name: tex_name.clone(),
                sample_x,
                sample_y,
            })
        } else {
            ctx.hir.layer(Layer::Image {
                tex_name: tex_name.clone(),
            })
        };
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
                    let sample_at_vec = sample_at;
                    ctx.scopes.push(std::collections::HashMap::new());
                    for ch_def in &channels {
                        let channel = Sx::TexChannel {
                            tex_name: tex_name.clone(),
                            channel: ch_def.channel_idx,
                            sample_at: sample_at_vec.clone().map(Box::new),
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
                sample_at,
            }
        } else {
            Value::Layer(layer_id)
        }
    }
}
