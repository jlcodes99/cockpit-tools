use std::fs;
use std::path::{Path, PathBuf};

fn normalize(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindsurfVersionMetadata {
    pub ide_version: Option<String>,
    pub extension_version: Option<String>,
}

fn json_version(path: &Path) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(&fs::read_to_string(path).ok()?).ok()?;
    normalize(
        value
            .get("productVersion")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("version").and_then(|v| v.as_str())),
    )
}

#[cfg(target_os = "macos")]
fn plist_version(path: &Path) -> Option<String> {
    let out = std::process::Command::new("plutil")
        .args(["-p"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.trim().starts_with("\"CFBundleShortVersionString\"")
            || line.trim().starts_with("\"CFBundleVersion\"")
        {
            if let Some(value) = line.split("=>").nth(1) {
                if let Some(version) = normalize(Some(value.trim().trim_matches('"'))) {
                    return Some(version);
                }
            }
        }
    }
    None
}

fn candidates(product: &str, configured_path: Option<&str>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = configured_path.and_then(|v| normalize(Some(v))) {
        roots.push(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    roots.push(PathBuf::from(format!("/Applications/{}.app", product)));
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            roots.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join(product)
                    .join(format!("{}.exe", product)),
            );
        }
        if let Ok(program) = std::env::var("ProgramFiles") {
            roots.push(
                PathBuf::from(program)
                    .join(product)
                    .join(format!("{}.exe", product)),
            );
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local").join("share").join(product));
    }

    let mut files = Vec::new();
    for root in roots {
        #[cfg(target_os = "macos")]
        {
            let app = if root.extension().and_then(|v| v.to_str()) == Some("app") {
                root.clone()
            } else {
                root.ancestors()
                    .find(|p| p.extension().and_then(|v| v.to_str()) == Some("app"))
                    .map(Path::to_path_buf)
                    .unwrap_or(root.clone())
            };
            files.push(app.join("Contents").join("Info.plist"));
            files.push(app.join("Contents/Resources/app/product.json"));
            files.push(app.join("Contents/Resources/app/package.json"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            if root.is_file() {
                files.push(root.clone());
            }
            let base = if root.is_file() {
                root.parent().unwrap_or(&root)
            } else {
                &root
            };
            files.push(base.join("resources/app/product.json"));
            files.push(base.join("resources/app/package.json"));
            files.push(base.join("product.json"));
            files.push(base.join("package.json"));
        }
    }
    files
}

#[cfg(target_os = "windows")]
fn windows_file_version(path: &Path) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let script = r#"$p=[Environment]::GetEnvironmentVariable('COCKPIT_CLIENT_EXE','Process'); if ([string]::IsNullOrWhiteSpace($p)) { exit 2 }; $v=(Get-Item $p).VersionInfo; $value=$v.ProductVersion; if ([string]::IsNullOrWhiteSpace($value)) { $value=$v.FileVersion }; [Console]::Write($value)"#;
    let out = std::process::Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .env("COCKPIT_CLIENT_EXE", path)
        .output()
        .ok()?;
    if out.status.success() {
        normalize(Some(&String::from_utf8_lossy(&out.stdout)))
    } else {
        None
    }
}

pub fn detect_client_version(product: &str, configured_path: Option<&str>) -> Option<String> {
    for path in candidates(product, configured_path) {
        if !path.exists() {
            continue;
        }
        #[cfg(target_os = "macos")]
        if path.file_name().and_then(|v| v.to_str()) == Some("Info.plist") {
            if let Some(v) = plist_version(&path) {
                return Some(v);
            }
        }
        #[cfg(target_os = "windows")]
        if path
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| v.eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
        {
            if let Some(v) = windows_file_version(&path) {
                return Some(v);
            }
        }
        if let Some(v) = json_version(&path) {
            return Some(v);
        }
    }
    None
}

fn application_roots(product: &str, configured_path: Option<&str>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(path) = configured_path.and_then(|v| normalize(Some(v))) {
        roots.push(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    roots.push(PathBuf::from(format!("/Applications/{}.app", product)));
    #[cfg(target_os = "windows")]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            roots.push(PathBuf::from(local).join("Programs").join(product));
        }
        if let Ok(program) = std::env::var("ProgramFiles") {
            roots.push(PathBuf::from(program).join(product));
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local").join("share").join(product));
    }
    roots
}

fn application_base(root: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let app = if root.extension().and_then(|v| v.to_str()) == Some("app") {
            root.to_path_buf()
        } else {
            root.ancestors()
                .find(|p| p.extension().and_then(|v| v.to_str()) == Some("app"))
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.to_path_buf())
        };
        app.join("Contents")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if root.is_file() {
            root.parent().unwrap_or(root).to_path_buf()
        } else {
            root.to_path_buf()
        }
    }
}

fn extract_language_server_version(source: &str) -> Option<String> {
    // Official Devin/Windsurf extension-host source exposes a generated constant,
    // e.g. `var F6="1.48.2"; ... get languageServerVersion(){return F6}`.
    let marker = "get languageServerVersion(){return ";
    let marker_pos = source.find(marker)?;
    let ident = source[marker_pos + marker.len()..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect::<String>();
    if ident.is_empty()
        || !ident
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
    {
        return None;
    }
    let declaration = format!("var {}=\"", ident);
    let value_start = source.find(&declaration)? + declaration.len();
    let value_end = source[value_start..].find('"')? + value_start;
    normalize(Some(&source[value_start..value_end]))
}

/// Read the product and language-server versions used by the official
/// Windsurf/Devin metadata payload. Filesystem inspection is intentionally
/// synchronous; callers on async paths should run this in `spawn_blocking`.
pub fn detect_windsurf_version_metadata(
    product: &str,
    configured_path: Option<&str>,
) -> WindsurfVersionMetadata {
    let mut ide_version = None;
    let mut extension_version = None;
    for root in application_roots(product, configured_path) {
        let base = application_base(&root);
        for product_json in [
            base.join("Resources/app/product.json"),
            base.join("resources/app/product.json"),
        ] {
            if let Ok(text) = fs::read_to_string(product_json) {
                ide_version = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|value| {
                        normalize(value.get("windsurfVersion").and_then(|v| v.as_str()))
                    });
                if ide_version.is_some() {
                    break;
                }
            }
        }
        if extension_version.is_none() {
            let source =
                base.join("Resources/app/out/vs/workbench/api/node/extensionHostProcess.js");
            let source = if source.exists() {
                source
            } else {
                base.join("resources/app/out/vs/workbench/api/node/extensionHostProcess.js")
            };
            if let Ok(text) = fs::read_to_string(source) {
                extension_version = extract_language_server_version(&text);
            }
        }
        if ide_version.is_some() && extension_version.is_some() {
            break;
        }
    }
    ide_version = ide_version.or_else(|| detect_client_version(product, configured_path));
    WindsurfVersionMetadata {
        ide_version,
        extension_version,
    }
}

#[cfg(test)]
mod tests {
    use super::extract_language_server_version;

    #[test]
    fn parses_official_language_server_constant() {
        let source = r#"var F6="1.48.2";var yw=class{get languageServerVersion(){return F6}}"#;
        assert_eq!(
            extract_language_server_version(source).as_deref(),
            Some("1.48.2")
        );
    }
}
