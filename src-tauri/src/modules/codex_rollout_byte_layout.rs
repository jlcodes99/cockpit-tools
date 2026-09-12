//! Preserve byte cursors in Codex paginated history without editing its databases.

pub(crate) fn must_stop_before_repair(
    running: bool,
    requires_stopped: bool,
    rewrites_content: bool,
    dry_run: bool,
) -> bool {
    running && !dry_run && (requires_stopped || rewrites_content)
}

pub(crate) fn preserve_line_byte_len(
    original: &str,
    mut updated: String,
) -> Result<String, String> {
    let padding = original.len().checked_sub(updated.len()).ok_or_else(|| {
        "Cannot rewrite indexed Codex history: the new provider metadata exceeds the original record byte length. Keep the original rollout and history index; this migration requires an offline, index-aware operation.".to_string()
    })?;
    // JSON permits trailing spaces. Keep every following record at its original offset.
    updated.extend(std::iter::repeat_n(' ', padding));
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_writer_blocks_quick_content_rewrite_but_not_dry_run() {
        assert!(must_stop_before_repair(true, false, true, false));
        assert!(!must_stop_before_repair(true, false, true, true));
        assert!(!must_stop_before_repair(false, false, true, false));
        assert!(!must_stop_before_repair(true, false, false, false));
        assert!(must_stop_before_repair(true, true, false, false));
    }

    #[test]
    fn shorter_provider_keeps_next_record_offset() {
        let original = r#"{"model_provider":"synthetic-provider"}"#;
        let updated = r#"{"model_provider":"openai"}"#.to_string();
        let result = preserve_line_byte_len(original, updated.clone()).unwrap();
        assert_eq!(result.len(), original.len());
        assert_eq!(result.trim_end(), updated);
    }

    #[test]
    fn longer_provider_consumes_existing_padding() {
        let original = "{\"model_provider\":\"openai\"}            ";
        let updated = r#"{"model_provider":"synthetic-provider"}"#.to_string();
        assert_eq!(
            preserve_line_byte_len(original, updated.clone()).unwrap(),
            updated
        );
    }

    #[test]
    fn growth_without_capacity_is_rejected() {
        assert!(preserve_line_byte_len("{}", "{\"x\":1}".to_string()).is_err());
    }

    #[test]
    fn measures_utf8_bytes_not_characters() {
        let original = "{\"x\":\"\u{4e2d}\u{6587}\"}";
        let result = preserve_line_byte_len(original, "{\"x\":\"a\"}".to_string()).unwrap();
        assert_eq!(result.len(), original.len());
    }

    #[test]
    fn equal_length_content_is_unchanged() {
        assert_eq!(
            preserve_line_byte_len("abc", "xyz".to_string()).unwrap(),
            "xyz"
        );
    }
}
