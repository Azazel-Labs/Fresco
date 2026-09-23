use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn collect(at: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(at)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "fr") {
            files.push(path);
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let engine = root.join("engine");
    println!("cargo:rerun-if-changed=engine");
    let mut files = Vec::new();
    collect(&engine, &mut files)?;
    files.sort();
    let mut generated = String::from("pub static SOURCES: &[EngineSource] = &[\n");
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .expect("engine source under package root")
            .to_string_lossy()
            .replace('\\', "/");
        generated.push_str(&format!(
            "EngineSource {{ path: {relative:?}, source: include_str!({path:?}) }},\n",
            path = file.to_string_lossy(),
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("build output")).join("engine_sources.rs"),
        generated,
    )
}
