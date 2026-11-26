use ipnet::IpNet;
use std::{collections::HashSet, net::IpAddr};

pub(crate) const INVALID: &str = "codex.proxy.directHosts.invalid";

fn normalize(raw: &str) -> Option<String> {
    if let Ok(network) = raw.parse::<IpNet>() {
        return Some(network.trunc().to_string());
    }
    let literal = raw
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(raw);
    if let Ok(address) = literal.parse::<IpAddr>() {
        return Some(address.to_string());
    }
    let name = raw
        .strip_prefix("*.")
        .or_else(|| raw.strip_prefix('.'))
        .unwrap_or(raw);
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.len() > 253
        || !name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
    {
        return None;
    }
    Some(name.to_ascii_lowercase())
}

pub(crate) fn parse(raw: &str) -> Result<Vec<String>, String> {
    // Keep the serialized rules within the Windows process launch argument budget.
    if raw.len() > 4096 {
        return Err(INVALID.into());
    }
    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for entry in raw
        .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        let rule = normalize(entry).ok_or(INVALID)?;
        if seen.insert(rule.clone()) {
            rules.push(rule);
        }
    }
    Ok(rules)
}

pub(crate) fn matches(rules: &[String], host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    let address = host.parse::<IpAddr>().ok();
    let host = host.to_ascii_lowercase();
    rules.iter().any(|rule| {
        if let Some(address) = address {
            if let Ok(network) = rule.parse::<IpNet>() {
                return network.contains(&address);
            }
            return rule.parse::<IpAddr>().ok() == Some(address);
        }
        host == *rule
            || host
                .strip_suffix(rule.as_str())
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

pub(crate) fn chromium_rules(rules: &[String]) -> String {
    let mut result = String::from("localhost;127.0.0.1;[::1]");
    for rule in rules {
        match rule.parse::<IpAddr>() {
            Ok(IpAddr::V6(address)) => result.push_str(&format!(";[{address}]")),
            Ok(IpAddr::V4(_)) => {
                result.push(';');
                result.push_str(rule);
            }
            Err(_) if rule.contains('/') => {
                result.push(';');
                result.push_str(rule);
            }
            Err(_) => result.push_str(&format!(";{rule};*.{rule}")),
        }
    }
    result
}

fn is_builtin_loopback_rule(rule: &str) -> bool {
    matches!(rule, "localhost" | "127.0.0.1" | "::1")
}

pub(crate) fn extract_from_electron_args(args: &[String]) -> Vec<String> {
    let mut raw_values = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        let trimmed = arg.trim();
        if let Some(value) = trimmed.strip_prefix("--proxy-bypass-list=") {
            raw_values.push(value);
        } else if trimmed == "--proxy-bypass-list" {
            if let Some(next) = iter.peek().copied() {
                if !next.trim_start().starts_with('-') {
                    raw_values.push(next.as_str());
                    iter.next();
                }
            }
        }
    }

    let mut seen = HashSet::new();
    let mut rules = Vec::new();
    for raw in raw_values {
        let raw = raw.trim_matches(['"', '\'', '“', '”', '‘', '’']);
        for entry in raw
            .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
            .map(|entry| entry.trim_matches(['"', '\'', '“', '”', '‘', '’']))
            .filter(|entry| !entry.is_empty())
        {
            let Some(rule) = normalize(entry) else {
                continue;
            };
            if is_builtin_loopback_rule(&rule) {
                continue;
            }
            if seen.insert(rule.clone()) {
                rules.push(rule);
            }
        }
    }
    rules
}

pub(crate) fn merge_rules(primary: &[String], extra: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut merged = Vec::with_capacity(primary.len() + extra.len());
    for rule in primary.iter().chain(extra.iter()) {
        let Some(normalized) = normalize(rule) else {
            continue;
        };
        if is_builtin_loopback_rule(&normalized) {
            continue;
        }
        if seen.insert(normalized.clone()) {
            merged.push(normalized);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_intranet_hosts_and_matches_only_the_configured_destinations() {
        let rules = parse("MCP.LAN, *.corp.test; 10.2.4.3/16\n[fd00::1] .corp.test").unwrap();
        assert_eq!(rules, ["mcp.lan", "corp.test", "10.2.0.0/16", "fd00::1"]);
        for host in [
            "MCP.LAN.",
            "worker.corp.test",
            "corp.test",
            "10.2.9.2",
            "fd00:0::1",
        ] {
            assert!(matches(&rules, host), "{host}");
        }
        for host in [
            "fakecorp.test",
            "corp.test.external",
            "10.3.0.1",
            "api.openai.com",
        ] {
            assert!(!matches(&rules, host), "{host}");
        }
        assert_eq!(chromium_rules(&rules), "localhost;127.0.0.1;[::1];mcp.lan;*.mcp.lan;corp.test;*.corp.test;10.2.0.0/16;[fd00::1]");
    }

    #[test]
    fn invalid_entries_do_not_save_partial_rules() {
        for entry in [
            "*",
            "https://mcp.lan",
            "mcp.lan:8080",
            "10.2.0.0/33",
            "mcp..lan",
        ] {
            assert_eq!(parse(&format!("good.lan,{entry}")).unwrap_err(), INVALID);
        }
        assert!(parse("\n , ").unwrap().is_empty());
    }

    #[test]
    fn extracts_and_merges_valid_extra_args_proxy_bypass_rules_while_dropping_wildcards() {
        let args = vec![
            "--proxy-bypass-list=\"localhost;127.0.0.1;[::1];10.0.0.0/8;172.16.0.0/12;192.168.0.0/16\""
                .to_string(),
            "--proxy-bypass-list".to_string(),
            "*;*.corp.test;<local>".to_string(),
        ];
        let extracted = extract_from_electron_args(&args);
        assert_eq!(
            extracted,
            ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "corp.test"]
        );
        let merged = merge_rules(&["mcp.lan".to_string(), "10.0.0.0/8".to_string()], &extracted);
        assert_eq!(
            chromium_rules(&merged),
            "localhost;127.0.0.1;[::1];mcp.lan;*.mcp.lan;10.0.0.0/8;172.16.0.0/12;192.168.0.0/16;corp.test;*.corp.test"
        );
    }
}
