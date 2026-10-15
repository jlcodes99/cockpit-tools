//! Local credential-format recognition. Never returns raw input or parser errors.
use super::codex_proxy_subscription_parser::{self as parser, ParsedCatalog, ParsedNode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImportOptions {
    pub protocol: String,
    pub fallback_name: String,
    pub format: String,
    pub skip_invalid: bool,
    pub skip_duplicates: bool,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            fallback_name: "Proxy list".into(),
            protocol: "socks5".into(),
            format: "auto".into(),
            skip_invalid: false,
            skip_duplicates: true,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub line: usize,
    pub name: Option<String>,
    pub protocol: Option<String>,
    pub server: Option<String>,
    pub port: Option<u64>,
    pub authenticated: bool,
    pub duplicate: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct ImportPreview {
    pub structured: bool,
    pub rows: Vec<PreviewRow>,
    pub valid: usize,
    pub invalid: usize,
    pub duplicates: usize,
}
pub struct Prepared {
    pub catalog: ParsedCatalog,
    pub preview: ImportPreview,
}
pub fn fingerprint(node: &ParsedNode) -> Option<String> {
    node.outbound
        .as_ref()
        .map(|v| format!("{:x}", Sha256::digest(v.to_string().as_bytes())))
}
fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
fn endpoint(value: &str, protocol: &str) -> Option<String> {
    let (host, port) = value.rsplit_once(':')?;
    if host.is_empty()
        || port.is_empty()
        || !port.bytes().all(|c| c.is_ascii_digit())
        || port.parse::<u16>().ok()? == 0
        || host
            .chars()
            .any(|c| c.is_whitespace() || ['@', '/', '?', '#', '\\'].contains(&c))
        || (host.contains(':') && !(host.starts_with('[') && host.ends_with(']')))
    {
        return None;
    }
    let u = url::Url::parse(&format!("{protocol}://{value}")).ok()?;
    if u.host_str().is_none() || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    Some(format!("{host}:{port}"))
}
fn with_auth(host: &str, auth: &str, protocol: &str) -> Option<String> {
    let host = endpoint(host, protocol)?;
    let (user, password) = auth.split_once(':')?;
    if user.is_empty() {
        return None;
    }
    Some(format!(
        "{protocol}://{}:{}@{host}",
        encode(user),
        encode(password)
    ))
}
pub fn normalize_line(input: &str, options: &ImportOptions) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() || input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("IMPORT_INVALID".into());
    }
    if input.contains("://") {
        return Ok(input.into());
    }
    if !["http", "https", "socks5", "socks5h"].contains(&options.protocol.as_str())
        || !["auto", "host_auth", "auth_at_host", "host_at_auth"].contains(&options.format.as_str())
    {
        return Err("IMPORT_INVALID".into());
    }
    let protocol = &options.protocol;
    let mut candidates = HashSet::new();
    if options.format == "auto" {
        if let Some(host) = endpoint(input, protocol) {
            candidates.insert(format!("{protocol}://{host}"));
        }
    }
    if ["auto", "host_auth"].contains(&options.format.as_str()) {
        let offset = if input.starts_with('[') {
            input.find(']').map(|n| n + 1).unwrap_or(input.len())
        } else {
            0
        };
        if let Some(first) = input[offset..].find(':').map(|n| n + offset) {
            if let Some(second) = input[first + 1..].find(':').map(|n| first + 1 + n) {
                if let Some(url) = with_auth(&input[..second], &input[second + 1..], protocol) {
                    candidates.insert(url);
                }
            }
        }
    }
    for (index, _) in input.match_indices('@') {
        if ["auto", "auth_at_host"].contains(&options.format.as_str()) {
            if let Some(url) = with_auth(&input[index + 1..], &input[..index], protocol) {
                candidates.insert(url);
            }
        }
        if ["auto", "host_at_auth"].contains(&options.format.as_str()) {
            if let Some(url) = with_auth(&input[..index], &input[index + 1..], protocol) {
                candidates.insert(url);
            }
        }
    }
    match candidates.len() {
        1 => Ok(candidates.into_iter().next().unwrap()),
        0 => Err("IMPORT_INVALID".into()),
        _ => Err("IMPORT_AMBIGUOUS".into()),
    }
}
fn display_name(node: &ParsedNode, index: usize) -> String {
    if !node
        .name
        .strip_prefix('#')
        .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    {
        return node.name.chars().take(80).collect();
    }
    let Some(out) = node.outbound.as_ref() else {
        return format!("Proxy {}", index + 1);
    };
    let out = out.get("proxy").unwrap_or(out);
    let host = out["server"].as_str().unwrap_or("");
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.into()
    };
    format!(
        "{} · {}:{}",
        match node.protocol.as_str() {
            "socks" if out["version"] == "5" => "SOCKS5".to_string(),
            "http" if out["tls"]["enabled"] == true => "HTTPS".to_string(),
            _ => node.protocol.to_uppercase(),
        },
        host,
        out.get("server_port")
            .or_else(|| out.get("port"))
            .unwrap_or(&serde_json::Value::Null)
    )
    .chars()
    .take(80)
    .collect()
}
pub fn prepare(
    input: &str,
    options: &ImportOptions,
    existing: &HashSet<String>,
) -> Result<Prepared, String> {
    if input.len() > 2 * 1024 * 1024 {
        return Err("CATALOG_LIMIT".into());
    }
    let trimmed = input.trim();
    // Structured formats preserve group references and existing parser behavior.
    if trimmed
        .lines()
        .any(|line| line.starts_with("proxies:") || line.starts_with("proxy-groups:"))
        || trimmed.starts_with('{')
        || (!trimmed.contains(':') && !trimmed.contains('@') && !trimmed.contains('\n'))
    {
        if let Ok(mut catalog) = parser::parse(trimmed) {
            let mut seen = existing.clone();
            let mut rows = Vec::new();
            let grouped = !catalog.groups.is_empty();
            let mut retained = Vec::new();
            let mut names = HashSet::new();
            for (index, mut node) in catalog.nodes.into_iter().enumerate() {
                if !grouped {
                    let base = display_name(&node, index);
                    let mut name = base.clone();
                    let mut count = 2;
                    while !names.insert(name.clone()) {
                        name = format!("{base} ({count})");
                        count += 1;
                    }
                    node.name = name;
                }
                let duplicate = !grouped && fingerprint(&node).is_some_and(|id| !seen.insert(id));
                rows.push(preview_row(index + 1, &node, duplicate));
                if grouped || !duplicate || !options.skip_duplicates {
                    retained.push(node);
                }
            }
            catalog.nodes = retained;
            let duplicates = rows.iter().filter(|r| r.duplicate).count();
            let invalid = rows.iter().filter(|r| r.error.is_some()).count();
            return Ok(Prepared {
                preview: ImportPreview {
                    structured: true,
                    valid: rows.len() - invalid - duplicates,
                    invalid,
                    duplicates,
                    rows,
                },
                catalog,
            });
        }
    }
    let mut nodes = Vec::new();
    let mut rows = Vec::new();
    let mut seen = existing.clone();
    let mut names = HashSet::new();
    let (mut valid, mut invalid, mut duplicates) = (0, 0, 0);
    for (index, line) in input.lines().enumerate() {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        if rows.len() >= 4096 {
            return Err("CATALOG_LIMIT".into());
        }
        let parsed = normalize_line(line, options)
            .and_then(|url| parser::parse(&url))
            .and_then(|mut c| {
                let mut n = c.nodes.pop().ok_or("IMPORT_INVALID")?;
                if n.error.is_some() {
                    return Err("IMPORT_INVALID".into());
                }
                n.name = display_name(&n, index);
                n.id = fingerprint(&n).ok_or("IMPORT_INVALID")?;
                Ok(n)
            });
        match parsed {
            Ok(mut node) => {
                let duplicate = !seen.insert(node.id.clone());
                if duplicate {
                    duplicates += 1;
                } else {
                    valid += 1;
                }
                let base = node.name.clone();
                let mut suffix = 2;
                while !names.insert(node.name.clone()) {
                    node.name = format!("{base} ({suffix})");
                    suffix += 1;
                }
                rows.push(preview_row(index + 1, &node, duplicate));
                if !duplicate || !options.skip_duplicates {
                    if duplicate {
                        node.id = format!("{:x}", Sha256::digest(format!("{}:{index}", node.id)));
                    }
                    nodes.push(node);
                }
            }
            Err(error) => {
                invalid += 1;
                rows.push(PreviewRow {
                    line: index + 1,
                    name: None,
                    protocol: None,
                    server: None,
                    port: None,
                    authenticated: false,
                    duplicate: false,
                    error: Some(if error == "IMPORT_AMBIGUOUS" {
                        error
                    } else {
                        "IMPORT_INVALID".into()
                    }),
                });
            }
        }
    }
    if rows.is_empty() {
        return Err("IMPORT_INVALID".into());
    }
    Ok(Prepared {
        catalog: ParsedCatalog {
            nodes,
            groups: vec![],
        },
        preview: ImportPreview {
            structured: false,
            rows,
            valid,
            invalid,
            duplicates,
        },
    })
}
fn preview_row(line: usize, node: &ParsedNode, duplicate: bool) -> PreviewRow {
    let out = node.outbound.as_ref().map(|o| o.get("proxy").unwrap_or(o));
    PreviewRow {
        line,
        name: Some(node.name.clone()),
        protocol: Some(node.protocol.to_uppercase()),
        server: out.and_then(|o| o["server"].as_str()).map(str::to_owned),
        port: out
            .and_then(|o| o.get("server_port").or_else(|| o.get("port")))
            .and_then(serde_json::Value::as_u64),
        authenticated: out.is_some_and(|o| {
            o.get("username").is_some() || o.get("password").is_some() || o.get("uuid").is_some()
        }),
        duplicate,
        error: node.error.clone(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_formats_ipv6_and_reserved_password_characters() {
        let o = ImportOptions::default();
        for row in [
            "example.com:1080:user:pass",
            "user:pass@example.com:1080",
            "example.com:1080@user:pass",
            "socks5://user:pass@example.com:1080",
        ] {
            let p = prepare(row, &o, &HashSet::new()).unwrap();
            assert_eq!(p.preview.valid, 1);
            assert_eq!(
                p.catalog.nodes[0].outbound.as_ref().unwrap()["password"],
                "pass"
            );
        }
        let p = prepare(
            "[2001:db8::1]:1080:user:p:a@ss%word",
            &ImportOptions {
                format: "host_auth".into(),
                ..o
            },
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            p.catalog.nodes[0].outbound.as_ref().unwrap()["password"],
            "p:a@ss%word"
        );
    }
    #[test]
    fn ambiguity_requires_format_and_preview_never_exposes_auth() {
        let o = ImportOptions::default();
        assert_eq!(
            normalize_line("host:123@user:456", &o).unwrap_err(),
            "IMPORT_AMBIGUOUS"
        );
        let p = prepare(
            "host:123@user:456",
            &ImportOptions {
                format: "host_at_auth".into(),
                ..o
            },
            &HashSet::new(),
        )
        .unwrap();
        let json = serde_json::to_string(&p.preview).unwrap();
        assert!(!json.contains("456"));
        assert!(!json.contains("username"));
    }
    #[test]
    fn errors_keep_line_numbers_and_duplicates_use_credentials() {
        let p = prepare(
            "host:1080:u:one\n\ninvalid\nhost:1080:u:one\nhost:1080:u:two",
            &ImportOptions::default(),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(
            (p.preview.valid, p.preview.invalid, p.preview.duplicates),
            (2, 1, 1)
        );
        assert_eq!(p.preview.rows[1].line, 3);
        assert_eq!(p.catalog.nodes.len(), 2);
        let existing = p.catalog.nodes.iter().filter_map(fingerprint).collect();
        let repeated = prepare("host:1080:u:two", &ImportOptions::default(), &existing).unwrap();
        assert_eq!(repeated.preview.duplicates, 1);
        assert!(repeated.catalog.nodes.is_empty());
    }
    #[test]
    fn base64_list_names_are_unique_and_deduplicated_against_store() {
        use base64::Engine as _;
        let input = base64::engine::general_purpose::STANDARD
            .encode("socks5://user:first@host:1080\nsocks5://user:second@host:1080");
        let p = prepare(&input, &ImportOptions::default(), &HashSet::new()).unwrap();
        assert!(p.preview.structured);
        assert_eq!(p.preview.valid, 2);
        assert_ne!(p.catalog.nodes[0].name, p.catalog.nodes[1].name);
        let existing = p.catalog.nodes.iter().filter_map(fingerprint).collect();
        let p = prepare(&input, &ImportOptions::default(), &existing).unwrap();
        assert_eq!(p.preview.duplicates, 2);
        assert!(p.catalog.nodes.is_empty());
    }
}
