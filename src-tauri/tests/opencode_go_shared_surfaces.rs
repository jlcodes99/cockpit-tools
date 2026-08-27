#[test]
fn tray_layout_and_renderers_own_a_dedicated_opencode_go_platform() {
    let layout = include_str!("../src/modules/tray_layout.rs");
    let tray = include_str!("../src/modules/tray.rs");
    let native_menu = include_str!("../src/modules/macos_native_menu.rs");

    assert!(layout.contains("PLATFORM_OPENCODE_GO"));
    assert!(tray.contains("OpenCodeGo"));
    assert!(tray.contains("build_opencode_go_display_info"));
    assert!(native_menu.contains("build_opencode_go_cards"));
    assert!(native_menu.contains("PlatformId::OpenCodeGo"));
}

#[test]
fn opencode_go_shared_surfaces_read_only_the_dedicated_store() {
    let tray = include_str!("../src/modules/tray.rs");
    let native_menu = include_str!("../src/modules/macos_native_menu.rs");
    let tray_builder = tray
        .split("fn build_opencode_go_display_info")
        .nth(1)
        .and_then(|body| body.split("fn is_claude_desktop_account").next())
        .expect("OpenCode Go tray builder");
    let native_builder = native_menu
        .split("fn build_opencode_go_cards")
        .nth(1)
        .and_then(|body| body.split("fn build_antigravity_cards").next())
        .expect("OpenCode Go native menu builder");

    for builder in [tray_builder, native_builder] {
        assert!(builder.contains("opencode_go::list_connections"));
        assert!(!builder.contains("codex_account"));
        assert!(!builder.contains("codex_api_key_provider_usage"));
        assert!(!builder.contains("load_codex_model_providers"));
    }
}
