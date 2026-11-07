use super::{
    select_cloud_code_base_url, QuotaCloudCodeContext, TokenData,
    CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL, CLOUD_CODE_DAILY_BASE_URL, CLOUD_CODE_PROD_BASE_URL,
};

#[test]
fn token_context_normalizes_real_projects_and_discards_consumer_placeholder() {
    for project in [
        None,
        Some(""),
        Some(" \t "),
        Some("aicode-consumers"),
        Some(" aicode-consumers "),
    ] {
        let mut token = TokenData::new(
            "access".into(),
            "refresh".into(),
            3600,
            None,
            project.map(str::to_string),
            None,
        );
        token.is_gcp_tos = Some(true);
        let ctx = QuotaCloudCodeContext::from_token(&token);
        assert_eq!(ctx.preferred_project_id, None, "{project:?}");
        assert!(ctx.is_gcp_tos);
    }

    let token = TokenData::new(
        "access".into(),
        "refresh".into(),
        3600,
        None,
        Some(" project-123 ".into()),
        None,
    );
    let ctx = QuotaCloudCodeContext::from_token(&token);
    assert_eq!(ctx.preferred_project_id.as_deref(), Some("project-123"));
    assert!(!ctx.is_gcp_tos);
}

#[test]
fn gcp_routing_requires_a_real_project_even_for_direct_contexts() {
    for project in [
        None,
        Some(""),
        Some("\n\t"),
        Some("aicode-consumers"),
        Some(" aicode-consumers "),
    ] {
        let ctx = QuotaCloudCodeContext {
            preferred_project_id: project.map(str::to_string),
            is_gcp_tos: true,
        };
        assert_eq!(
            select_cloud_code_base_url(&ctx, None, false, false),
            CLOUD_CODE_DAILY_BASE_URL
        );
        assert_eq!(
            select_cloud_code_base_url(&ctx, None, true, true),
            CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL
        );
    }

    for is_gcp_tos in [false, true] {
        let ctx = QuotaCloudCodeContext {
            preferred_project_id: Some(" project-123 ".into()),
            is_gcp_tos,
        };
        assert_eq!(
            select_cloud_code_base_url(&ctx, None, false, false),
            if is_gcp_tos {
                CLOUD_CODE_PROD_BASE_URL
            } else {
                CLOUD_CODE_DAILY_BASE_URL
            },
        );
    }
}

#[test]
fn cloud_code_routing_keeps_override_gcp_internal_daily_priority() {
    let ctx = QuotaCloudCodeContext {
        preferred_project_id: Some("project-123".into()),
        is_gcp_tos: true,
    };
    assert_eq!(
        select_cloud_code_base_url(&ctx, Some(" https://override.example.test "), true, true),
        "https://override.example.test",
    );
    assert_eq!(
        select_cloud_code_base_url(&ctx, None, true, true),
        CLOUD_CODE_PROD_BASE_URL
    );
    assert_eq!(
        select_cloud_code_base_url(&ctx, Some(" \t "), true, true),
        CLOUD_CODE_PROD_BASE_URL
    );

    let default = QuotaCloudCodeContext::default();
    assert_eq!(
        select_cloud_code_base_url(
            &default,
            Some("https://override.example.test"),
            false,
            false
        ),
        "https://override.example.test",
    );
    for (is_google_internal, is_insider_or_dev) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        assert_eq!(
            select_cloud_code_base_url(&default, None, is_google_internal, is_insider_or_dev),
            if is_google_internal && is_insider_or_dev {
                CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL
            } else {
                CLOUD_CODE_DAILY_BASE_URL
            },
        );
    }
}
