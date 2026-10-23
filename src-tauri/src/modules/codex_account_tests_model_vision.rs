// Codex 账号测试：按 models.json 的完整模型 ID 和图片输入声明判断默认图片能力。

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
fn models_with_catalog_image_input_declarations_default_to_vision_input() {
    for model in [
        "gpt-5.6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-6.1-sol",
        "gpt-6-astra",
        "gpt-6-sol",
        "gpt-6-luna",
        "codex-auto-review",
        "claude-haiku-4-5",
        "claude-haiku-5-5",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "gemini-3.8-flash",
        "gemini-4-argon",
        "grok-4.7",
        "openai/gpt-5.6-sol",
        "openai/gpt-6-astra",
        "openai/gpt-6-sol",
        " OPENAI/GPT-6-LUNA ",
    ] {
        assert!(
            super::model_defaults_to_vision_input(model),
            "{model} 在目录中声明图片输入，应默认识图"
        );
    }
}

#[test]
fn models_absent_from_catalog_or_without_image_declarations_default_to_false() {
    for model in [
        "gpt-5.5-pro",
        "gpt-6-luna-custom",
        "gpt-99.0",
        "gpt-5.4",
        "gpt-5.4-mini",
        "gpt-5.3-codex",
        "gpt-5",
        "gpt-4o",
        "gpt-3.5-turbo",
        "gpt-reserve",
        "gpt-image-2.5",
        "deepseek-v4-pro",
        "claude-sonnet-4-5",
        "gemini-2.5-pro",
        "glm-4.6",
    ] {
        assert!(
            !super::model_defaults_to_vision_input(model),
            "{model} 未找到或未声明图片输入，应默认不支持"
        );
    }
}
