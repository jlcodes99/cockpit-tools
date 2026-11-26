//! A production desktop build must embed the local UI, not navigate to a URL.
use std::path::{Path, PathBuf};
use tauri_utils::config::{Config, FrontendDist};

pub fn effective_config(
    root: &Path,
    target: tauri_utils::platform::Target,
    override_json: Option<&str>,
) -> Result<Config, String> {
    // Match tauri-build's platform config and TAURI_CONFIG merge exactly.
    let (mut config, paths) = tauri_utils::config::parse::read_from(target, root)
        .map_err(|error| format!("Cannot read Tauri config: {error}"))?;
    for path in paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    if let Some(override_json) = override_json {
        let patch = serde_json::from_str(override_json)
            .map_err(|error| format!("Invalid TAURI_CONFIG: {error}"))?;
        json_patch::merge(&mut config, &patch);
    }
    serde_json::from_value(config).map_err(|error| format!("Invalid Tauri config: {error}"))
}

pub fn validate(config: &Config, root: &Path) -> Result<Vec<PathBuf>, String> {
    let relative = match config.build.frontend_dist.as_ref() {
        Some(FrontendDist::Directory(path)) if !path.is_absolute() => path,
        _ => return Err("Production frontendDist must be a relative directory (for example ../dist). A Windows absolute path is parsed as a URL and embeds no UI.".into()),
    };
    let dist = root.join(relative).canonicalize()
        .map_err(|error| format!("Frontend directory is missing: {error}"))?;
    if !dist.is_dir() { return Err("Frontend output is not a directory".into()); }
    let index = dist.join("index.html");
    let html = std::fs::read_to_string(&index)
        .map_err(|error| format!("Frontend index.html is missing or unreadable: {error}"))?;
    if html.contains("/@vite/client") || html.contains("/src/main.") {
        return Err("Frontend index.html is a development entry; run the production frontend build".into());
    }
    // Vite emits quoted local asset URLs in index.html. Validate every such
    // reference, including modulepreload chunks, rather than accepting an empty
    // assets directory or merely finding a build-success log message.
    let mut files = vec![index];
    let mut js = false;
    let mut css = false;
    for reference in html.split(['\"', '\'']).filter_map(|part| {
        part.strip_prefix("./assets/")
            .or_else(|| part.strip_prefix("/assets/"))
            .or_else(|| part.strip_prefix("assets/"))
    }) {
        let path = dist.join("assets").join(reference);
        let canonical = path.canonicalize()
            .map_err(|error| format!("Referenced frontend asset is missing: {}: {error}", path.display()))?;
        if !canonical.starts_with(&dist) || !canonical.is_file() {
            return Err("Frontend asset must be a file inside the frontend directory".into());
        }
        if std::fs::metadata(&canonical).map_err(|error| error.to_string())?.len() == 0 {
            return Err(format!("Frontend asset is empty: {}", canonical.display()));
        }
        js |= canonical.extension().is_some_and(|extension| extension == "js");
        css |= canonical.extension().is_some_and(|extension| extension == "css");
        files.push(canonical);
    }
    if !html.contains("<script") || !js || !css {
        return Err("Frontend index.html must reference nonempty production JavaScript and CSS assets".into());
    }
    Ok(files)
}

#[cfg(not(test))]
pub fn verify_production_frontend(target: &str) {
    println!("cargo:rerun-if-changed=build_frontend.rs");
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    // Unit tests and tauri dev do not embed the frontend. The embedding mode is
    // Tauri's own dependency metadata, not the optimization/profile name.
    if tauri_build::is_dev() { return; }
    let root = std::env::current_dir().expect("build directory is required");
    let config = effective_config(&root, tauri_utils::platform::Target::from_triple(target),
        std::env::var("TAURI_CONFIG").ok().as_deref())
        .unwrap_or_else(|error| panic!("Frontend packaging guard: {error}"));
    let files = validate(&config, &root)
        .unwrap_or_else(|error| panic!("Frontend packaging guard: {error}"));
    for file in &files { println!("cargo:rerun-if-changed={}", file.display()); }
    println!("cargo:warning=Frontend packaging guard verified {} local entry/assets", files.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "cockpit-frontend-guard-{}-{stamp}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(path.join("src-tauri")).unwrap();
            std::fs::create_dir_all(path.join("dist/assets")).unwrap();
            std::fs::write(path.join("src-tauri/tauri.conf.json"), r#"{"identifier":"test.cockpit","build":{"frontendDist":"../dist"}}"#).unwrap();
            std::fs::write(path.join("dist/index.html"), r#"<script src="./assets/main.js"></script><link href="./assets/main.css">"#).unwrap();
            std::fs::write(path.join("dist/assets/main.js"), "console.log('fixture');").unwrap();
            std::fs::write(path.join("dist/assets/main.css"), "body{}").unwrap();
            Self(path)
        }
        fn root(&self) -> PathBuf { self.0.join("src-tauri") }
        fn config(&self, patch: Option<&str>) -> Config {
            effective_config(&self.root(), tauri_utils::platform::Target::Windows, patch).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let root = std::env::temp_dir().canonicalize().unwrap();
            if let Ok(target) = self.0.canonicalize() {
                assert!(target.starts_with(&root) && target != root);
                assert!(target.file_name().unwrap().to_string_lossy().starts_with("cockpit-frontend-guard-"));
                let _ = std::fs::remove_dir_all(target);
            }
        }
    }

    #[test]
    fn accepts_real_local_entry_and_referenced_assets() {
        let f = Fixture::new();
        assert_eq!(validate(&f.config(None), &f.root()).unwrap().len(), 3);
    }
    #[test]
    fn rejects_windows_drive_url_after_actual_tauri_deserialization() {
        let f = Fixture::new();
        for path in [r"C:\ui\dist", "C:/ui/dist", "https://example.test"] {
            let patch = serde_json::json!({"build":{"frontendDist":path}}).to_string();
            let config = f.config(Some(&patch));
            assert!(matches!(config.build.frontend_dist, Some(FrontendDist::Url(_))));
            assert!(validate(&config, &f.root()).is_err());
        }
    }
    #[test]
    fn platform_config_and_environment_override_use_tauri_merge_order() {
        let f = Fixture::new();
        std::fs::write(f.root().join("tauri.windows.conf.json"), r#"{"build":{"frontendDist":"https://example.test"}}"#).unwrap();
        assert!(validate(&f.config(None), &f.root()).is_err());
        let config = f.config(Some(r#"{"build":{"frontendDist":"../dist"}}"#));
        assert!(validate(&config, &f.root()).is_ok());
    }
    #[test]
    fn rejects_missing_directory_or_index() {
        let f = Fixture::new();
        assert!(validate(&f.config(Some(r#"{"build":{"frontendDist":"../absent"}}"#)), &f.root()).is_err());
        std::fs::remove_file(f.0.join("dist/index.html")).unwrap();
        assert!(validate(&f.config(None), &f.root()).is_err());
    }
    #[test]
    fn rejects_missing_or_empty_referenced_assets() {
        let f = Fixture::new();
        std::fs::write(f.0.join("dist/assets/main.js"), "").unwrap();
        assert!(validate(&f.config(None), &f.root()).is_err());
        std::fs::remove_file(f.0.join("dist/assets/main.css")).unwrap();
        assert!(validate(&f.config(None), &f.root()).is_err());
    }
    #[test]
    fn rejects_development_html_or_empty_asset_manifest() {
        let f = Fixture::new();
        for html in ["<script src='/@vite/client'></script>", "<html>no frontend bundles</html>"] {
            std::fs::write(f.0.join("dist/index.html"), html).unwrap();
            assert!(validate(&f.config(None), &f.root()).is_err());
        }
    }
}
