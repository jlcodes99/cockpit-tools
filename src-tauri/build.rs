#[cfg(target_os = "macos")]
use swift_rs::SwiftLinker;

use std::path::PathBuf;

#[cfg(target_os = "macos")]
fn link_macos_swift_runtime_rpaths() {
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

#[cfg(target_os = "macos")]
fn link_swift_package_products(package_name: &str) {
    // Use the same DEBUG value as swift-rs when choosing SwiftPM configuration.
    let configuration = if std::env::var("DEBUG").ok().as_deref() == Some("true") {
        "Debug"
    } else {
        "Release"
    };
    let products_dir = PathBuf::from(
        std::env::var("OUT_DIR").expect("OUT_DIR is required for Swift package linking"),
    )
    .join("swift-rs")
    .join(package_name)
    .join("out/Products")
    .join(configuration);
    // Xcode 27 / Swift 6.4 writes here. Keep swift-rs's original search path
    // as well, since older SwiftPM versions still use the target triple path.
    println!("cargo:rustc-link-search=native={}", products_dir.display());
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Build-script cfg describes the host, not the binary being linked. Reserve
    // 8 MiB for the Windows MSVC host application even when cross-compiling.
    // Scope the flag to the application binary, not the library or sidecars.
    let target = std::env::var("TARGET").expect("TARGET is required");
    if target.ends_with("-windows-msvc") {
        println!("cargo:rustc-link-arg-bin=cockpit-tools=/STACK:8388608");
    }
    println!("cargo:rustc-env=COCKPIT_RUST_TARGET={target}");

    // The cfg gate selects the build host. Only link Swift for a macOS target,
    // otherwise macOS-to-Windows builds inherit Darwin-only linker arguments.
    #[cfg(target_os = "macos")]
    if target.ends_with("-apple-darwin") {
        SwiftLinker::new("12.0")
            .with_package("MacosNativeMenuSwift", "native/macos-native-menu")
            .link();
        link_swift_package_products("MacosNativeMenuSwift");
        link_macos_swift_runtime_rpaths();
    }

    tauri_build::build()
}
