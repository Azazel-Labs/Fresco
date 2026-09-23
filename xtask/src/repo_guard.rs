//! Source hygiene that must also cover files outside Cargo's module graph.

use std::fs;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(root: &Path) -> Result<(), i32> {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--eol",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|error| {
            eprintln!("cannot enumerate repository files: {error}");
            1
        })?;
    if !output.status.success() {
        eprintln!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Err(1);
    }

    let mut failed = false;
    for name in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = std::str::from_utf8(name).map_err(|error| {
            eprintln!("repository path is not UTF-8: {error}");
            1
        })?;
        let (attributes, name) = name.split_once('\t').ok_or_else(|| {
            eprintln!("git ls-files returned a record without EOL metadata: {name}");
            1
        })?;
        let path = root.join(name);
        // Deleted tracked files and submodules have no source file to check.
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).map_err(|error| {
            eprintln!("cannot read {name}: {error}");
            1
        })?;
        let binary = attributes.contains("attr/-text") || attributes.contains("w/-text");
        if let Some(issue) = source_issue(Path::new(name), &bytes, binary) {
            eprintln!("{name}: {issue}");
            failed = true;
        }
    }
    if failed {
        Err(1)
    } else {
        println!("Repository source hygiene passed (LF and module filenames).");
        Ok(())
    }
}

fn source_issue(path: &Path, bytes: &[u8], binary: bool) -> Option<&'static str> {
    if path.file_name().is_some_and(|name| name == "mod.rs") {
        return Some("use a sibling <module>.rs file instead of <module>/mod.rs");
    }
    // Binary data is not subject to the text newline policy.
    if !binary
        && !bytes.contains(&0)
        && std::str::from_utf8(bytes).is_ok()
        && bytes.contains(&b'\r')
    {
        return Some("text files must use LF line endings, not CRLF or bare CR");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::source_issue;
    use std::path::Path;

    #[test]
    fn rejects_legacy_modules_even_if_empty_or_not_compiled() {
        assert!(source_issue(Path::new("src/unused/mod.rs"), b"", false).is_some());
        assert!(source_issue(Path::new("src/unused.rs"), b"mod child;\n", false).is_none());
    }

    #[test]
    fn enforces_lf_in_text_but_preserves_binary_data() {
        assert!(source_issue(Path::new("test.fr"), b"line\r\n", false).is_some());
        assert!(source_issue(Path::new("test.fr"), b"line\r", false).is_some());
        assert!(source_issue(Path::new("test.fr"), b"line\n", false).is_none());
        assert!(source_issue(Path::new("asset.wasm"), b"\0asm\r\n", true).is_none());
        assert!(source_issue(Path::new("asset.png"), b"\x89PNG\r\n", true).is_none());
        assert!(source_issue(Path::new("asset.pdf"), b"%PDF-1.7\r\n", true).is_none());
    }
}
