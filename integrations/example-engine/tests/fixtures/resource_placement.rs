pub fn source(inferred: bool) -> String {
    let explicit = include_str!("resource-placement.fr");
    if inferred {
        explicit
            .replace("at after_opaque as target {", "")
            .replace("} // placement", "")
            .replace("target.color", "opaque.color")
            .replace("target.depth", "opaque.depth")
    } else {
        explicit.into()
    }
}
