pub fn normalize_config_toml_spacing(content: &str) -> String {
    let mut normalized = String::with_capacity(content.len());
    let mut blank_line_count = 0usize;

    for line in content.lines() {
        if line.trim().is_empty() {
            blank_line_count += 1;
            if blank_line_count <= 1 {
                normalized.push('\n');
            }
            continue;
        }

        blank_line_count = 0;
        normalized.push_str(line);
        normalized.push('\n');
    }

    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_config_toml_spacing;

    #[test]
    fn collapses_repeated_blank_lines() {
        let input = "model = \"gpt-5\"\n\n\n\nsandbox_mode = \"danger-full-access\"\n\n[desktop]\n";
        let output = normalize_config_toml_spacing(input);

        assert_eq!(
            output,
            "model = \"gpt-5\"\n\nsandbox_mode = \"danger-full-access\"\n\n[desktop]\n"
        );
    }
}
