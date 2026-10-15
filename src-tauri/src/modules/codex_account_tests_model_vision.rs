// Codex 账号测试：模型识图默认规则（gpt-5.5 及以上默认支持图片输入）。

#[test]
fn parses_gpt_model_versions_with_prefix_and_suffix() {
    assert_eq!(super::parse_gpt_model_version("gpt-5.5"), Some((5, 5)));
    assert_eq!(super::parse_gpt_model_version("GPT-5.6-Luna"), Some((5, 6)));
    assert_eq!(
        super::parse_gpt_model_version("openai/gpt-5.6-sol"),
        Some((5, 6))
    );
    assert_eq!(super::parse_gpt_model_version("gpt-6-astra"), Some((6, 0)));
    assert_eq!(super::parse_gpt_model_version("gpt-5"), Some((5, 0)));
    assert_eq!(super::parse_gpt_model_version("gpt-reserve"), None);
    assert_eq!(super::parse_gpt_model_version("gpt-image-2.5"), None);
    assert_eq!(super::parse_gpt_model_version("codex-auto-review"), None);
}

#[test]
fn gpt_5_5_and_later_default_to_vision_input() {
    for model in [
        "gpt-5.5",
        "gpt-5.5-pro",
        "gpt-5.6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-6-astra",
        "openai/gpt-5.6-sol",
        "openai/gpt-6-astra",
    ] {
        assert!(
            super::model_defaults_to_vision_input(model),
            "{model} 应默认识图"
        );
    }
}

#[test]
fn other_models_keep_previous_defaults() {
    for model in [
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.3-codex",
        "gpt-5",
        "gpt-4o",
        "gpt-3.5-turbo",
        "gpt-reserve",
        "gpt-image-2.5",
        "codex-auto-review",
        "deepseek-v4-pro",
        "claude-sonnet-4-5",
        "gemini-2.5-pro",
        "glm-4.6",
    ] {
        assert!(
            !super::model_defaults_to_vision_input(model),
            "{model} 不应被默认规则影响"
        );
    }
}
