use serde_json::Value;

pub(crate) fn field<'a>(value: Option<&'a Value>, key: &str) -> Option<&'a str> {
    value?
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn scope<'a>(
    info: Option<&'a Value>,
    plan: Option<&'a Value>,
) -> (Option<&'a str>, Option<&'a str>) {
    let source = field(info, "login_source").or_else(|| field(plan, "login_source"));
    let organization = field(info, "orgId").or_else(|| field(plan, "orgId"));
    (source, organization)
}

pub(crate) fn matches(
    left_id: Option<&str>,
    right_id: Option<&str>,
    left_email: Option<&str>,
    right_email: Option<&str>,
) -> bool {
    match (left_id, right_id) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => match (left_email, right_email) {
            (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
            _ => false,
        },
    }
}

pub(crate) fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }

    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("账号 ID 非法，包含路径字符".to_string());
    }

    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.');
    if !valid {
        return Err("账号 ID 非法，仅允许字母/数字/._-".to_string());
    }

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_email_does_not_merge_different_known_user_ids() {
        let email = Some("member@example.invalid");
        assert!(!matches(Some("user-a"), Some("user-b"), email, email));
        assert!(matches(Some("user-a"), None, email, email));
    }
}
