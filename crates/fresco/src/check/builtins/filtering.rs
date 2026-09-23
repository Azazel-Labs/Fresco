//! Filtering effect builtin (analytic prefiltering, §3).
//!
//! The `filtering` effect controls whether procedural patterns (checker, stripe)
//! use analytic prefiltering to eliminate aliasing and shimmer under minification.
//!
//! Usage:
//!   - `filtering(off); pattern` — force unfiltered point-sampling  
//!   - `filtering(on); pattern` — force filtering (requires footprint)
//!
//! By default, patterns automatically enable filtering when a footprint is
//! available (via `canvas_space` Jacobian).

use crate::ast::Expr;
use crate::builtin;
use crate::check::Value;
use crate::diag::Diag;
use crate::hir;

builtin! {
    name = "filtering",
    signature = single {
        args(mode: Expr = "filtering mode: 'on' or 'off'"),
        result = Scalar,
        caps = PURE,
    },
    check = |ctx, mode| {
        // Parse the mode argument (should be a variable: on or off)
        let state = match &mode.node {
            Expr::Var(name) if name == "on" => hir::FilteringState::ForceOn,
            Expr::Var(name) if name == "off" => hir::FilteringState::ForceOff,
            _ => {
                ctx.diags.push(
                    Diag::error(
                        mode.span.clone(),
                        "filtering mode must be `on` or `off`",
                    )
                    .with_help("use `filtering(on)` or `filtering(off)` before pattern calls"),
                );
                return None;
            }
        };

        ctx.set_filtering_state(state);

        // Return a dummy scalar value (0.0) since this is a state-setting effect
        Value::Coverage(hir::Sx::Lit(0.0))
    }
}
