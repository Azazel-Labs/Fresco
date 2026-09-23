use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Trait for resolving module imports. Implementations provide both path
/// canonicalization (for deduplication) and file content loading.
pub(crate) trait ModuleLoader {
    /// Return the canonical key for the root/entrypoint file. Used as the
    /// initial visited-set entry and as the `source_file` for root-level
    /// function declarations.
    fn root_canonical_key(&self, filename: &str) -> String;

    /// Resolve `import_path` relative to `current_canonical` and return
    /// `(canonical_key, source_text)`, or an error string describing the
    /// failure.
    fn resolve_import(
        &self,
        import_path: &str,
        current_canonical: &str,
    ) -> Result<(String, String), String>;

    /// Return implicit engine modules that should always be included for this
    /// compilation root. Entries are `(canonical_key, source_text)`.
    fn implicit_engine_modules(
        &self,
        _root_canonical: &str,
    ) -> Result<Vec<(String, String)>, String> {
        Ok(Vec::new())
    }
}

/// Filesystem user imports paired with an authoritative in-memory engine bundle.
pub(crate) struct EmbeddedEngineLoader {
    pub(crate) engine: MemModuleLoader,
    pub(crate) entrypoint: String,
}

const EMBEDDED_ENGINE_PREFIX: &str = "fresco-engine://";

impl ModuleLoader for EmbeddedEngineLoader {
    fn root_canonical_key(&self, filename: &str) -> String {
        FsModuleLoader::default().root_canonical_key(filename)
    }

    fn resolve_import(
        &self,
        import_path: &str,
        current_canonical: &str,
    ) -> Result<(String, String), String> {
        if let Some(current) = current_canonical.strip_prefix(EMBEDDED_ENGINE_PREFIX) {
            let (key, source) = self.engine.resolve_import(import_path, current)?;
            Ok((format!("{EMBEDDED_ENGINE_PREFIX}{key}"), source))
        } else {
            FsModuleLoader::default().resolve_import(import_path, current_canonical)
        }
    }

    fn implicit_engine_modules(
        &self,
        _root_canonical: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let key = self.engine.root_canonical_key(&self.entrypoint);
        let source = self
            .engine
            .files
            .get(&key)
            .ok_or_else(|| format!("embedded engine entry `{key}` is missing"))?;
        Ok(vec![(
            format!("{EMBEDDED_ENGINE_PREFIX}{key}"),
            source.clone(),
        )])
    }
}

/// Filesystem-based loader used by the CLI and single-file compile path.
#[derive(Default)]
pub(crate) struct FsModuleLoader<'a> {
    pub(crate) engine_dir: Option<&'a Path>,
}

impl ModuleLoader for FsModuleLoader<'_> {
    fn root_canonical_key(&self, filename: &str) -> String {
        let p = PathBuf::from(filename);
        std::fs::canonicalize(&p).unwrap_or(p).display().to_string()
    }

    fn resolve_import(
        &self,
        import_path: &str,
        current_canonical: &str,
    ) -> Result<(String, String), String> {
        let current = Path::new(current_canonical);
        let base = current.parent().ok_or_else(|| {
            format!(
                "cannot resolve import `{import_path}` from `{current_canonical}`; \
                 use a file path relative to the importing source file"
            )
        })?;
        let full_path = base.join(import_path);
        let canonical = std::fs::canonicalize(&full_path).map_err(|e| {
            format!(
                "cannot read imported module `{}` from `{current_canonical}`: {e}\n\
                 resolved path: `{}`",
                import_path,
                full_path.display()
            )
        })?;
        let src = std::fs::read_to_string(&canonical)
            .map_err(|e| format!("cannot read imported module `{}`: {e}", canonical.display()))?;
        Ok((canonical.display().to_string(), src))
    }

    fn implicit_engine_modules(
        &self,
        root_canonical: &str,
    ) -> Result<Vec<(String, String)>, String> {
        if let Some(engine_dir) = self.engine_dir {
            // An explicit profile is authoritative: never fall back to discovery
            // or to recursively loading an incomplete profile.
            let entry = engine_dir.join("engine.fr");
            let canonical = std::fs::canonicalize(&entry).map_err(|e| {
                format!(
                    "cannot resolve explicit engine entry `{}`: {e}",
                    entry.display()
                )
            })?;
            let src = std::fs::read_to_string(&canonical).map_err(|e| {
                format!(
                    "cannot read explicit engine entry `{}`: {e}",
                    canonical.display()
                )
            })?;
            return Ok(vec![(canonical.display().to_string(), src)]);
        }
        let root_path = Path::new(root_canonical);
        let Some(mut search_dir) = root_path.parent() else {
            return Ok(Vec::new());
        };

        let mut engine_dir: Option<PathBuf> = None;
        loop {
            let candidate = search_dir.join("engine");
            if candidate.is_dir() {
                engine_dir = Some(candidate);
                break;
            }
            let Some(parent) = search_dir.parent() else {
                break;
            };
            search_dir = parent;
        }

        let Some(engine_dir) = engine_dir else {
            return Ok(Vec::new());
        };

        let entry = engine_dir.join("engine.fr");
        if entry.is_file() {
            let canonical = std::fs::canonicalize(&entry)
                .map_err(|e| format!("cannot resolve engine entry `{}`: {e}", entry.display()))?;
            let src = std::fs::read_to_string(&canonical)
                .map_err(|e| format!("cannot read engine entry `{}`: {e}", canonical.display()))?;
            return Ok(vec![(canonical.display().to_string(), src)]);
        }

        let mut files = Vec::new();
        collect_fr_files_recursive(&engine_dir, &mut files).map_err(|e| {
            format!(
                "cannot read engine module directory `{}`: {e}",
                engine_dir.display()
            )
        })?;
        files.sort();

        let mut out = Vec::with_capacity(files.len());
        for file in files {
            let canonical = std::fs::canonicalize(&file).unwrap_or(file.clone());
            let src = std::fs::read_to_string(&canonical)
                .map_err(|e| format!("cannot read engine module `{}`: {e}", canonical.display()))?;
            out.push((canonical.display().to_string(), src));
        }

        Ok(out)
    }
}

/// In-memory loader for WASM and testing: resolves imports against a
/// pre-populated map of `virtual_path → source_text`.
pub(crate) struct MemModuleLoader {
    pub(crate) files: HashMap<String, String>,
}

impl ModuleLoader for MemModuleLoader {
    fn root_canonical_key(&self, filename: &str) -> String {
        normalize_virtual_path("", filename).unwrap_or_else(|| filename.to_string())
    }

    fn resolve_import(
        &self,
        import_path: &str,
        current_canonical: &str,
    ) -> Result<(String, String), String> {
        let base_dir = match current_canonical.rfind('/') {
            Some(i) => &current_canonical[..i],
            None => "",
        };
        let canonical = normalize_virtual_path(base_dir, import_path).ok_or_else(|| {
            format!("cannot resolve import path `{import_path}` from `{current_canonical}`")
        })?;
        let src = self.files.get(&canonical).cloned().ok_or_else(|| {
            let available: Vec<&str> = self.files.keys().map(String::as_str).collect();
            format!(
                "module `{canonical}` not found in virtual file system \
                 (available: {})",
                available.join(", ")
            )
        })?;
        Ok((canonical, src))
    }

    fn implicit_engine_modules(
        &self,
        root_canonical: &str,
    ) -> Result<Vec<(String, String)>, String> {
        // Match filesystem loading: select the nearest ancestor engine directory.
        let mut directory = root_canonical
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent);
        loop {
            let prefix = if directory.is_empty() {
                "engine/".to_string()
            } else {
                format!("{directory}/engine/")
            };
            let entry = format!("{prefix}engine.fr");
            if let Some(src) = self.files.get(&entry) {
                return Ok(vec![(entry, src.clone())]);
            }
            let mut modules = self
                .files
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(".fr"))
                .map(|(key, source)| (key.clone(), source.clone()))
                .collect::<Vec<_>>();
            if !modules.is_empty() {
                modules.sort_by(|left, right| left.0.cmp(&right.0));
                return Ok(modules);
            }
            if directory.is_empty() {
                return Ok(Vec::new());
            }
            directory = directory.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
    }
}

fn collect_fr_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_fr_files_recursive(&path, out)?;
            continue;
        }
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("fr") {
            out.push(path);
        }
    }
    Ok(())
}

/// Normalize a virtual path by resolving `.` and `..` components.
/// `base_dir` is the parent directory of the current file (may be empty
/// for a root file). Returns `None` if the resolved path would be empty.
fn normalize_virtual_path(base_dir: &str, import_path: &str) -> Option<String> {
    let combined = if base_dir.is_empty() {
        import_path.to_string()
    } else {
        format!("{base_dir}/{import_path}")
    };

    let mut parts: Vec<&str> = Vec::new();
    for component in combined.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let result = parts.join("/");
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
