const CORE_OAUTH: &str = include_str!("../src/modules/oauth.rs");
const DESKTOP_OAUTH: &str = include_str!("../../../src-tauri/src/modules/oauth.rs");
const CORE_TOKEN: &str = include_str!("../src/models/token.rs");
const DESKTOP_TOKEN: &str = include_str!("../../../src-tauri/src/models/token.rs");

#[test]
fn core_and_desktop_oauth_sources_only_differ_in_the_shared_test_path() {
    const CORE_TEST_PATH: &str = "#[path = \"../../../../src-tauri/src/modules/oauth_tests.rs\"]";
    const DESKTOP_TEST_PATH: &str = "#[path = \"oauth_tests.rs\"]";

    assert!(
        CORE_OAUTH.matches(CORE_TEST_PATH).count() == 1,
        "Core OAuth must declare its shared test path exactly once"
    );
    assert!(
        DESKTOP_OAUTH.matches(DESKTOP_TEST_PATH).count() == 1,
        "Desktop OAuth must declare its shared test path exactly once"
    );
    let normalized_core = CORE_OAUTH.replace(CORE_TEST_PATH, DESKTOP_TEST_PATH);
    assert!(
        normalized_core == DESKTOP_OAUTH,
        "Core and desktop OAuth implementations drifted; synchronize production behavior"
    );
}

#[test]
fn core_and_desktop_token_models_remain_identical() {
    assert!(
        CORE_TOKEN == DESKTOP_TOKEN,
        "Core and desktop TokenData models drifted; synchronize their serialized metadata"
    );
}
