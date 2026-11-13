#[cfg(target_os = "macos")]
fn workbuddy_bundle_is_international(path: &Path) -> bool {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    std::iter::once(resolved.as_path())
        .chain((resolved != path).then_some(path))
        .any(|path| {
            let Some(bundle) = path.ancestors().find(|candidate| {
                candidate
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            }) else {
                return false;
            };
            let manifest = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok();
            manifest
                .as_ref()
                .and_then(plist::Value::as_dictionary)
                .and_then(|manifest| manifest.get("CFBundleIdentifier"))
                .and_then(plist::Value::as_string)
                == Some("com.workbuddy.workbuddy-ai")
        })
}

#[cfg(any(target_os = "windows", test))]
fn workbuddy_product_is_international(executable: &Path) -> bool {
    use std::io::{BufReader, Read};

    #[derive(Default, serde::Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    struct Product {
        is_oversea: bool,
        product_name: String,
        application_name: String,
    }
    let Some(parent) = executable.parent() else {
        return false;
    };
    let metadata = parent.join("resources/app.asar.unpacked/cli/product.json");
    let Ok(file) = std::fs::File::open(metadata) else {
        return false;
    };
    let product: Result<Product, _> =
        serde_json::from_reader(BufReader::new(file.take(1024 * 1024)));
    product.is_ok_and(|product| {
        product.is_oversea
            || product.product_name.eq_ignore_ascii_case("WorkBuddy AI")
            || product
                .application_name
                .eq_ignore_ascii_case("workbuddy-ai")
    })
}

fn workbuddy_install_is_international(executable: &Path) -> bool {
    #[cfg(target_os = "macos")]
    return workbuddy_bundle_is_international(executable);
    #[cfg(target_os = "windows")]
    {
        let resolved =
            std::fs::canonicalize(executable).unwrap_or_else(|_| executable.to_path_buf());
        return workbuddy_product_is_international(&resolved);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = executable;
        false
    }
}

fn select_workbuddy_launch_path(
    custom: Option<&str>,
    detect: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    if let Some(custom) = custom {
        let executable = resolve_workbuddy_macos_exec_path(custom)
            .ok_or_else(|| app_path_missing_error("workbuddy"))?;
        if !workbuddy_install_is_international(&executable) {
            return Ok(executable);
        }
    }
    detect().ok_or_else(|| app_path_missing_error("workbuddy"))
}

#[cfg(target_os = "macos")]
fn workbuddy_macos_main_command_matches(cmdline: &str, expected: &str) -> bool {
    let Some(executable) = extract_macos_exe_from_cmdline(cmdline) else {
        return false;
    };
    if executable != expected
        && (!executable
            .to_ascii_lowercase()
            .contains(".app/contents/macos/")
            || normalize_path_for_compare(&executable) != expected)
    {
        return false;
    }
    let arguments = &cmdline[executable.len()..];
    !arguments
        .split_whitespace()
        .any(|arg| arg.starts_with("--type="))
}

#[cfg(test)]
mod workbuddy_launch_path_tests {
    use super::*;

    #[test]
    fn same_windows_executable_name_is_distinguished_by_installed_product_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        for (dir, metadata, expected) in [
            (
                "domestic",
                r#"{"productName":"WorkBuddy","applicationName":"WorkBuddy","endpoint":"https://www.workbuddy.cn"}"#,
                false,
            ),
            (
                "overseas",
                r#"{"productName":"WorkBuddy AI","applicationName":"workbuddy-ai","isOversea":true}"#,
                true,
            ),
        ] {
            let path = tmp.path().join(dir);
            let product = path.join("resources/app.asar.unpacked/cli/product.json");
            std::fs::create_dir_all(product.parent().unwrap()).unwrap();
            std::fs::write(product, metadata).unwrap();
            let executable = path.join("WorkBuddy.exe");
            assert_eq!(workbuddy_product_is_international(&executable), expected);
        }
        assert!(!workbuddy_product_is_international(
            &tmp.path().join("WorkBuddy.exe")
        ));
    }

    #[cfg(target_os = "macos")]
    fn write_fixture_bundle(root: &Path, bundle_id: &str) -> PathBuf {
        let executable = root.join("Contents/MacOS/Electron");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, []).unwrap();
        let mut manifest = plist::Dictionary::new();
        manifest.insert(
            "CFBundleIdentifier".into(),
            plist::Value::String(bundle_id.into()),
        );
        plist::Value::Dictionary(manifest)
            .to_file_xml(root.join("Contents/Info.plist"))
            .unwrap();
        executable
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launch_replaces_known_overseas_bundle_and_keeps_explicit_domestic_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let overseas = tmp.path().join("WorkBuddy.app");
        let overseas_exec = write_fixture_bundle(&overseas, "com.workbuddy.workbuddy-ai");
        let domestic = tmp.path().join("Renamed WorkBuddy.app");
        let domestic_exec = write_fixture_bundle(&domestic, "com.tencent.workbuddy.mac");
        let alias = tmp.path().join("WorkBuddy Alias.app");
        std::os::unix::fs::symlink(&overseas, &alias).unwrap();

        for path in [&overseas, &overseas_exec, &alias] {
            assert_eq!(
                select_workbuddy_launch_path(path.to_str(), || Some(domestic_exec.clone()))
                    .unwrap(),
                domestic_exec,
            );
        }
        assert_eq!(
            select_workbuddy_launch_path(overseas.to_str(), || None).unwrap_err(),
            app_path_missing_error("workbuddy"),
        );
        assert!(workbuddy_install_is_international(&overseas_exec));
        assert_eq!(
            select_workbuddy_launch_path(domestic.to_str(), || panic!("valid path must win"))
                .unwrap(),
            domestic_exec,
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pid_match_uses_selected_main_binary_with_spaces_and_ignores_other_edition() {
        let expected = "/Applications/Renamed WorkBuddy.app/Contents/MacOS/Electron";
        assert!(workbuddy_macos_main_command_matches(
            &format!("{expected} --new-window"),
            expected
        ));
        assert!(!workbuddy_macos_main_command_matches(
            &format!("{expected} --type=renderer"),
            expected
        ));
        assert!(!workbuddy_macos_main_command_matches(
            "/Applications/WorkBuddy AI.app/Contents/MacOS/Electron --new-window",
            expected,
        ));
    }
}
