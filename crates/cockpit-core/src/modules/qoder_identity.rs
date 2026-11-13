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
