use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const START: &str = "<!-- readme:sample ";
const END: &str = "<!-- readme:end -->";

#[derive(serde::Deserialize)]
struct Manifest {
    samples: Vec<Sample>,
}

#[derive(serde::Deserialize)]
struct Sample {
    id: String,
    source: String,
    entrypoint: Option<String>,
}

fn load_manifest(root: &Path) -> Result<Vec<Sample>, String> {
    let text =
        fs::read_to_string(root.join("docs/readme/samples.json")).map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut ids = HashSet::new();
    for sample in &manifest.samples {
        if !sample.id.starts_with(|c: char| c.is_ascii_lowercase())
            || !sample
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            || !ids.insert(&sample.id)
        {
            return Err(format!(
                "invalid or duplicate README sample ID: {}",
                sample.id
            ));
        }
        for path in std::iter::once(&sample.source).chain(sample.entrypoint.iter()) {
            if !(path.starts_with("examples/")
                || path.starts_with("integrations/example-engine/engine/"))
                || !path.ends_with(".fr")
                || path.contains('\\')
                || path.contains(':')
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err(format!(
                    "README source must be a repository example or example engine source: {path}"
                ));
            }
        }
    }
    Ok(manifest.samples)
}

pub(crate) fn run(root: &Path, check: bool) -> Result<(), i32> {
    // Match the CLI's compiler stack budget, including on Windows where the
    // main thread's default stack is too small for the recursive parser.
    let result = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("readme-compiler".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, || execute(root, check))
            .map_err(|error| error.to_string())?
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    });
    result.map_err(|error| {
        eprintln!("README samples: {error}");
        1
    })
}

fn execute(root: &Path, check: bool) -> Result<(), String> {
    let mut files = HashMap::new();
    let samples = load_manifest(root)?;
    collect(root, &root.join("examples"), &mut files)?;
    files.extend(fresco_example_engine::source_files());
    files.extend(
        fresco_example_engine::source_files()
            .into_iter()
            .map(|(path, source)| (format!("integrations/example-engine/{path}"), source)),
    );
    let path = root.join("README.md");
    let original = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let rendered = render(&original.replace("\r\n", "\n"), &files, &samples)?;

    // Compile manifest entrypoints, including import harnesses for contract excerpts.
    let mut entries: Vec<_> = samples
        .iter()
        .map(|sample| sample.entrypoint.as_ref().unwrap_or(&sample.source))
        .collect();
    entries.sort();
    entries.dedup();
    for entry in &entries {
        fresco::driver::compile_bundle_virtual(&files, entry, false).map_err(|diagnostics| {
            format!(
                "{entry} failed compilation:\n{}",
                diagnostics
                    .iter()
                    .map(|d| { format!("{}:{}: {}", d.file, d.span_start, d.message) })
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })?;
    }
    if original != rendered {
        if check {
            return Err("README.md samples are stale; run `cargo xtask readme-sync` and commit the source and README changes".into());
        }
        fs::write(path, rendered).map_err(|e| e.to_string())?;
    }
    println!(
        "README samples synchronized; {} source programs compiled.",
        entries.len()
    );
    Ok(())
}

fn collect(root: &Path, dir: &Path, files: &mut HashMap<String, String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_dir() {
            collect(root, &path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "fr") {
            let key = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(
                key,
                fs::read_to_string(&path)
                    .map_err(|e| e.to_string())?
                    .replace("\r\n", "\n"),
            );
        }
    }
    Ok(())
}

fn render(
    markdown: &str,
    files: &HashMap<String, String>,
    samples: &[Sample],
) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = markdown;
    let mut seen = HashSet::new();
    while let Some(start) = rest.find(START) {
        let before = &rest[..start];
        reject_unmanaged(before)?;
        output.push_str(before);
        rest = &rest[start + START.len()..];
        let (id, body) = rest
            .split_once(" -->\n")
            .ok_or("malformed README source marker")?;
        let (old, after) = body.split_once(END).ok_or("missing README end marker")?;
        if old.contains(START) {
            return Err("nested README source markers".into());
        }
        let sample = samples
            .iter()
            .find(|sample| sample.id == id)
            .ok_or_else(|| format!("unknown README sample: {id}"))?;
        if !seen.insert(id) {
            return Err(format!("duplicate README sample marker: {id}"));
        }
        let source = files
            .get(&sample.source)
            .ok_or_else(|| format!("unknown README source: {}", sample.source))?;
        output.push_str(&format!(
            "{START}{id} -->\n```fresco\n{}\n```\n{END}",
            source.trim_end()
        ));
        rest = after;
    }
    reject_unmanaged(rest)?;
    if seen.is_empty() {
        return Err("no managed README samples found".into());
    }
    for sample in samples {
        if !seen.contains(sample.id.as_str()) {
            return Err(format!("README sample is not embedded: {}", sample.id));
        }
    }
    output.push_str(rest);
    Ok(output)
}

fn reject_unmanaged(text: &str) -> Result<(), String> {
    if text.contains("```fresco") || text.contains("~~~fresco") || text.contains(END) {
        return Err("unmanaged Fresco code block or unmatched end marker; add readme:sample/readme:end markers".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Sample> {
        vec![Sample {
            id: "a".into(),
            source: "examples/a.fr".into(),
            entrypoint: None,
        }]
    }

    #[test]
    fn refresh_preserves_prose_and_is_idempotent() {
        let files = HashMap::from([("examples/a.fr".into(), "canvas a() {}\n".into())]);
        let input = format!("before\n{START}a -->\nold\n{END}\nafter\n");
        let updated = render(&input, &files, &samples()).unwrap();
        assert!(updated.starts_with("before\n"));
        assert!(updated.ends_with("\nafter\n"));
        assert!(updated.contains("```fresco\ncanvas a() {}\n```"));
        assert_eq!(render(&updated, &files, &samples()).unwrap(), updated);
    }

    #[test]
    fn broken_or_unmanaged_samples_fail_closed() {
        let files = HashMap::new();
        for input in [
            "```fresco\ncanvas a() {}\n```".to_string(),
            format!("{START}missing.fr -->\n```fresco\n```\n{END}"),
            format!("{START}missing.fr -->\nno end"),
            format!("{START}a.fr -->\n{START}b.fr -->\n{END}"),
            END.to_string(),
            "no examples".to_string(),
        ] {
            assert!(
                render(&input, &files, &samples()).is_err(),
                "accepted: {input}"
            );
        }
    }

    #[test]
    fn manifest_mapping_changes_source_without_changing_readme_markers() {
        let input = format!("{START}a -->\nold\n{END}");
        let files = HashMap::from([
            ("examples/a.fr".into(), "first source".into()),
            (
                "examples/10) fundamentals/moved.fr".into(),
                "updated source".into(),
            ),
        ]);
        let mut selected = samples();
        assert!(
            render(&input, &files, &selected)
                .unwrap()
                .contains("first source")
        );
        selected[0].source = "examples/10) fundamentals/moved.fr".into();
        let updated = render(&input, &files, &selected).unwrap();
        assert!(updated.contains("updated source"));
        assert!(updated.contains("<!-- readme:sample a -->"));
        assert!(render(&format!("{input}\n{input}"), &files, &selected).is_err());
        selected.push(Sample {
            id: "unused".into(),
            source: "examples/a.fr".into(),
            entrypoint: None,
        });
        assert!(render(&input, &files, &selected).is_err());
    }

    #[test]
    fn check_is_read_only_and_compile_errors_block_sync() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = crate::workspace_root().join(format!(
            "target/readme-sync-tests/{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("docs/readme")).unwrap();
        fs::create_dir_all(root.join("examples/engine")).unwrap();
        fs::write(
            root.join("examples/engine/engine.fr"),
            include_str!("../../tests/render-policy/engine/engine.fr"),
        )
        .unwrap();
        fs::write(
            root.join("docs/readme/samples.json"),
            r#"{"samples":[{"id":"a","source":"examples/a.fr"}]}"#,
        )
        .unwrap();
        let source = root.join("examples/a.fr");
        fs::write(
            &source,
            "canvas a(uv: coord) -> color { compose { fill(#fff) } }\n",
        )
        .unwrap();
        let readme = root.join("README.md");
        let stale = format!("{START}a -->\nold\n{END}\n");
        fs::write(&readme, &stale).unwrap();
        assert!(run(&root, true).is_err());
        assert_eq!(fs::read_to_string(&readme).unwrap(), stale);
        assert!(run(&root, false).is_ok());
        assert!(run(&root, true).is_ok());
        let synced = fs::read_to_string(&readme).unwrap();
        fs::write(
            &source,
            "canvas a(uv: coord) -> color { compose { nonexistent() } }\n",
        )
        .unwrap();
        assert!(run(&root, false).is_err());
        assert_eq!(fs::read_to_string(&readme).unwrap(), synced);
        // A contract excerpt is validated through its importing example.
        fs::write(&source, "fn ink() -> color { return #fff }\n").unwrap();
        fs::write(
            root.join("examples/main.fr"),
            "import \"a.fr\"\ncanvas main(uv: coord) -> color { compose { fill(ink()) } }\n",
        )
        .unwrap();
        fs::write(root.join("docs/readme/samples.json"),
            r#"{"samples":[{"id":"a","source":"examples/a.fr","entrypoint":"examples/main.fr","preview":false}]}"#).unwrap();
        assert!(run(&root, false).is_ok());
        assert!(run(&root, true).is_ok());
        let synced = fs::read_to_string(&readme).unwrap();
        fs::write(&source, "fn ink() -> color { return nonexistent() }\n").unwrap();
        assert!(run(&root, false).is_err());
        assert_eq!(fs::read_to_string(&readme).unwrap(), synced);
        for source in [
            "../a.fr",
            "examples/../a.fr",
            "docs/readme/a.fr",
            "examples/a.txt",
        ] {
            fs::write(
                root.join("docs/readme/samples.json"),
                serde_json::json!({"samples": [{"id": "a", "source": source}]}).to_string(),
            )
            .unwrap();
            assert!(load_manifest(&root).is_err(), "accepted: {source}");
        }
        fs::write(root.join("docs/readme/samples.json"),
            r#"{"samples":[{"id":"a","source":"examples/a.fr"},{"id":"a","source":"examples/main.fr"}]}"#).unwrap();
        assert!(load_manifest(&root).is_err());
    }
}
