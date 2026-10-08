//! Deliberately bounded share-link compatibility; never echo credentials in errors.
//! The legacy intermediate schema remains readable for saved links; the runtime
//! converts it to the pinned Mihomo configuration without persisting a migration.
use base64::{engine::general_purpose, Engine as _};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use url::Url;

const INVALID: &str = "PROXY_INVALID_URL";
const UNSUPPORTED: &str = "PROXY_UNSUPPORTED_OPTION";
type Parsed<T> = Result<T, String>;

fn decode(s: &str) -> Parsed<String> {
    let mut bytes = Vec::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if i + 2 >= b.len() {
                return Err(INVALID.into());
            }
            let hex = |x: u8| (x as char).to_digit(16).map(|n| n as u8);
            bytes.push(hex(b[i + 1]).ok_or(INVALID)? * 16 + hex(b[i + 2]).ok_or(INVALID)?);
            i += 3;
        } else {
            bytes.push(b[i]);
            i += 1;
        }
    }
    let result = String::from_utf8(bytes).map_err(|_| INVALID)?;
    if result.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    Ok(result)
}

fn b64(s: &str) -> Parsed<Vec<u8>> {
    [
        general_purpose::STANDARD,
        general_purpose::STANDARD_NO_PAD,
        general_purpose::URL_SAFE,
        general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|e| e.decode(s).ok())
    .ok_or_else(|| INVALID.into())
}

fn query(url: &Url, allowed: &[&str]) -> Parsed<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for part in url
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|s| !s.is_empty())
    {
        let (k, v) = part.split_once('=').ok_or(INVALID)?;
        let k = decode(k)?;
        let v = decode(v)?;
        if !allowed.contains(&k.as_str()) {
            return Err(UNSUPPORTED.into());
        }
        if result.insert(k, v).is_some() {
            return Err(INVALID.into());
        }
    }
    Ok(result)
}

fn valid_uuid(value: &str) -> Parsed<()> {
    if value.len() != 36
        || !value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(INVALID.into());
    }
    Ok(())
}

fn endpoint(url: &Url, kind: &str) -> Parsed<Value> {
    if !matches!(url.path(), "" | "/") {
        return Err(UNSUPPORTED.into());
    }
    let host = url
        .host_str()
        .ok_or(INVALID)?
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(INVALID.into());
    }
    let port = url
        .port()
        .or_else(|| matches!(kind, "hysteria2" | "trojan" | "tuic").then_some(443))
        .ok_or(INVALID)?;
    if port == 0 {
        return Err(INVALID.into());
    }
    if let Some(fragment) = url.fragment() {
        decode(fragment)?;
    }
    Ok(json!({"type":kind,"server":host,"server_port":port,"connect_timeout":"10s"}))
}

fn get<'a>(q: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    q.get(key).map(String::as_str).unwrap_or(default)
}

fn tls(out: &mut Value, q: &BTreeMap<String, String>, required: bool) -> Parsed<()> {
    for key in ["insecure", "allowInsecure"] {
        if let Some(value) = q.get(key) {
            if !matches!(value.as_str(), "0" | "false") {
                return Err(UNSUPPORTED.into());
            }
        }
    }
    let security = get(q, "security", if required { "tls" } else { "none" });
    if !matches!(security, "none" | "tls" | "reality") || (required && security == "none") {
        return Err(UNSUPPORTED.into());
    }
    if security == "none" {
        if ["sni", "alpn", "fp", "pbk", "sid"]
            .iter()
            .any(|k| q.contains_key(*k))
        {
            return Err(UNSUPPORTED.into());
        }
        return Ok(());
    }
    let mut t = json!({"enabled":true});
    if let Some(sni) = q.get("sni") {
        if sni.is_empty() || sni.chars().any(char::is_whitespace) {
            return Err(INVALID.into());
        }
        t["server_name"] = json!(sni);
    }
    if let Some(alpn) = q.get("alpn") {
        let items: Vec<_> = alpn.split(',').collect();
        if items.iter().any(|v| v.is_empty() || v.len() > 255) {
            return Err(INVALID.into());
        }
        t["alpn"] = json!(items);
    }
    if let Some(fp) = q.get("fp") {
        if !matches!(
            fp.as_str(),
            "chrome"
                | "firefox"
                | "safari"
                | "ios"
                | "android"
                | "edge"
                | "360"
                | "qq"
                | "random"
                | "randomized"
        ) {
            return Err(UNSUPPORTED.into());
        }
        t["utls"] = json!({"enabled":true,"fingerprint":fp});
    }
    if security == "reality" {
        let pbk = q.get("pbk").ok_or(INVALID)?;
        if general_purpose::URL_SAFE_NO_PAD
            .decode(pbk)
            .map_err(|_| INVALID)?
            .len()
            != 32
        {
            return Err(INVALID.into());
        }
        let sid = get(q, "sid", "");
        if sid.len() > 16 || sid.len() % 2 != 0 || !sid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(INVALID.into());
        }
        t["reality"] = json!({"enabled":true,"public_key":pbk,"short_id":sid});
        if t.get("utls").is_none() {
            t["utls"] = json!({"enabled":true,"fingerprint":"chrome"});
        }
    } else if q.contains_key("pbk") || q.contains_key("sid") {
        return Err(UNSUPPORTED.into());
    }
    out["tls"] = t;
    Ok(())
}

fn transport(out: &mut Value, q: &BTreeMap<String, String>) -> Parsed<()> {
    match get(q, "type", "tcp") {
        "tcp" => {
            if ["host", "path", "serviceName"]
                .iter()
                .any(|k| q.contains_key(*k))
            {
                return Err(UNSUPPORTED.into());
            }
        }
        "ws" | "httpupgrade" => {
            if q.contains_key("serviceName") {
                return Err(UNSUPPORTED.into());
            }
            let mut t = json!({"type":get(q,"type","ws"),"path":get(q,"path","/")});
            if let Some(host) = q.get("host") {
                if get(q, "type", "ws") == "httpupgrade" {
                    t["host"] = json!(host);
                } else {
                    t["headers"] = json!({"Host":host});
                }
            }
            out["transport"] = t;
        }
        "grpc" => {
            if q.contains_key("host") || q.contains_key("path") {
                return Err(UNSUPPORTED.into());
            }
            out["transport"] = json!({"type":"grpc","service_name":get(q,"serviceName","")});
        }
        _ => return Err(UNSUPPORTED.into()),
    }
    Ok(())
}

fn ss(input: &str) -> Parsed<Value> {
    let rest = input.strip_prefix("ss://").ok_or(INVALID)?;
    // SIP002 requires userinfo base64 only (not the entire legacy authority).
    let url = Url::parse(input).map_err(|_| INVALID)?;
    let mut out = endpoint(&url, "shadowsocks")?;
    query(&url, &[])?;
    let authority = rest.split(['/', '?', '#']).next().ok_or(INVALID)?;
    let user = authority.rsplit_once('@').ok_or(INVALID)?.0;
    let raw = decode(user)?;
    let credentials = if raw.contains(':') {
        raw
    } else {
        String::from_utf8(b64(&raw)?).map_err(|_| INVALID)?
    };
    let (method, password) = credentials.split_once(':').ok_or(INVALID)?;
    if ![
        "aes-128-gcm",
        "aes-192-gcm",
        "aes-256-gcm",
        "chacha20-ietf-poly1305",
        "xchacha20-ietf-poly1305",
        "2022-blake3-aes-128-gcm",
        "2022-blake3-aes-256-gcm",
        "2022-blake3-chacha20-poly1305",
    ]
    .contains(&method)
    {
        return Err(UNSUPPORTED.into());
    }
    if password.is_empty() || credentials.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    if method.starts_with("2022-") {
        let len = if method == "2022-blake3-aes-128-gcm" {
            16
        } else {
            32
        };
        for key in password.split(':') {
            if b64(key)?.len() != len {
                return Err(INVALID.into());
            }
        }
    }
    out["method"] = json!(method);
    out["password"] = json!(password);
    Ok(out)
}

fn vmess(input: &str) -> Parsed<Value> {
    let bytes = b64(input.strip_prefix("vmess://").ok_or(INVALID)?)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| INVALID)?;
    let obj = value.as_object().ok_or(INVALID)?;
    let allowed = [
        "v", "ps", "add", "port", "id", "aid", "scy", "net", "type", "host", "path", "tls", "sni",
        "alpn", "fp", "insecure",
    ];
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(UNSUPPORTED.into());
    }
    let field = |key: &str, fallback: &str| -> Parsed<String> {
        match obj.get(key) {
            None => Ok(fallback.into()),
            Some(Value::String(s)) if !s.chars().any(char::is_control) => Ok(s.clone()),
            Some(Value::Number(n)) => Ok(n.to_string()),
            _ => Err(INVALID.into()),
        }
    };
    if field("v", "2")? != "2"
        || field("aid", "0")? != "0"
        || !matches!(field("type", "none")?.as_str(), "none" | "")
    {
        return Err(UNSUPPORTED.into());
    }
    let id = field("id", "")?;
    valid_uuid(&id)?;
    let host = field("add", "")?;
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || ['/', '@', '?', '#'].contains(&c))
    {
        return Err(INVALID.into());
    }
    let port: u16 = field("port", "")?.parse().map_err(|_| INVALID)?;
    if port == 0 {
        return Err(INVALID.into());
    }
    let security = field("scy", "auto")?;
    if !["auto", "aes-128-gcm", "chacha20-poly1305", "zero", "none"].contains(&security.as_str()) {
        return Err(UNSUPPORTED.into());
    }
    let mut out = json!({"type":"vmess","server":host.trim_start_matches('[').trim_end_matches(']'),"server_port":port,"uuid":id,"security":security,"alter_id":0,"connect_timeout":"10s"});
    let mut q = BTreeMap::new();
    let tls_value = field("tls", "")?;
    if !matches!(tls_value.as_str(), "" | "none" | "tls") {
        return Err(UNSUPPORTED.into());
    }
    q.insert(
        "security".into(),
        if tls_value.is_empty() {
            "none".into()
        } else {
            tls_value
        },
    );
    q.insert("type".into(), field("net", "tcp")?);
    for (from, to) in [
        ("host", "host"),
        ("path", "path"),
        ("sni", "sni"),
        ("alpn", "alpn"),
        ("fp", "fp"),
        ("insecure", "insecure"),
    ] {
        let s = field(from, "")?;
        if !s.is_empty() {
            q.insert(to.into(), s);
        }
    }
    tls(&mut out, &q, false)?;
    transport(&mut out, &q)?;
    Ok(out)
}

/// Parse only supported, explicitly validated options; callers must not log the result.
pub fn parse_node_link(input: &str) -> Parsed<Value> {
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err(INVALID.into());
    }
    let input = input.trim();
    if input.starts_with("vmess://") {
        return vmess(input);
    }
    if input.starts_with("ss://") {
        return ss(input);
    }
    let url = Url::parse(input).map_err(|_| INVALID)?;
    let kind = match url.scheme() {
        "hy2" => "hysteria2",
        s => s,
    };
    if !["vless", "trojan", "hysteria2", "tuic"].contains(&kind) {
        return Err(UNSUPPORTED.into());
    }
    let mut out = endpoint(&url, kind)?;
    let user = decode(url.username())?;
    if user.is_empty() {
        return Err(INVALID.into());
    }
    match kind {
        "vless" | "trojan" => {
            if url.password().is_some() {
                return Err(INVALID.into());
            }
            let allowed = if kind == "vless" {
                vec![
                    "security",
                    "sni",
                    "alpn",
                    "fp",
                    "pbk",
                    "sid",
                    "insecure",
                    "allowInsecure",
                    "type",
                    "host",
                    "path",
                    "serviceName",
                    "flow",
                    "encryption",
                ]
            } else {
                vec![
                    "security",
                    "sni",
                    "alpn",
                    "fp",
                    "insecure",
                    "allowInsecure",
                    "type",
                    "host",
                    "path",
                    "serviceName",
                ]
            };
            let q = query(&url, &allowed)?;
            if kind == "vless" {
                valid_uuid(&user)?;
                if get(&q, "encryption", "none") != "none" {
                    return Err(UNSUPPORTED.into());
                }
                out["uuid"] = json!(user);
                if let Some(flow) = q.get("flow") {
                    if flow != "xtls-rprx-vision"
                        || get(&q, "type", "tcp") != "tcp"
                        || !matches!(get(&q, "security", "none"), "tls" | "reality")
                    {
                        return Err(UNSUPPORTED.into());
                    }
                    out["flow"] = json!(flow);
                }
            } else {
                out["password"] = json!(user);
            }
            tls(&mut out, &q, kind == "trojan")?;
            transport(&mut out, &q)?;
        }
        "hysteria2" => {
            let q = query(&url, &["sni", "alpn", "insecure", "obfs", "obfs-password"])?;
            let password = match url.password() {
                Some(p) => format!("{}:{}", user, decode(p)?),
                None => user,
            };
            out["password"] = json!(password);
            if let Some(obfs) = q.get("obfs") {
                if obfs != "salamander" {
                    return Err(UNSUPPORTED.into());
                }
                let p = q
                    .get("obfs-password")
                    .filter(|s| !s.is_empty())
                    .ok_or(INVALID)?;
                out["obfs"] = json!({"type":"salamander","password":p});
            } else if q.contains_key("obfs-password") {
                return Err(UNSUPPORTED.into());
            }
            tls(&mut out, &q, true)?;
        }
        "tuic" => {
            valid_uuid(&user)?;
            let password = decode(url.password().ok_or(INVALID)?)?;
            if password.is_empty() {
                return Err(INVALID.into());
            }
            let q = query(
                &url,
                &[
                    "sni",
                    "alpn",
                    "insecure",
                    "congestion_control",
                    "udp_relay_mode",
                ],
            )?;
            let congestion = get(&q, "congestion_control", "cubic");
            let relay = get(&q, "udp_relay_mode", "native");
            if !["cubic", "new_reno", "bbr"].contains(&congestion)
                || !["native", "quic"].contains(&relay)
            {
                return Err(UNSUPPORTED.into());
            }
            out["uuid"] = json!(user);
            out["password"] = json!(password);
            out["congestion_control"] = json!(congestion);
            out["udp_relay_mode"] = json!(relay);
            tls(&mut out, &q, true)?;
        }
        _ => return Err(UNSUPPORTED.into()),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    const ID: &str = "11111111-2222-3333-4444-555555555555";
    #[test]
    fn protocols_and_ipv6() {
        for (link, kind) in [
            (
                format!(
                    "vless://{ID}@[::1]:443?security=tls&type=ws&path=%2Fproxy&host=example.com"
                ),
                "vless",
            ),
            (
                "trojan://p%40ss+word@example.com:443?sni=example.com".into(),
                "trojan",
            ),
            (
                "hy2://user:pass@example.com:443?obfs=salamander&obfs-password=a%2Bb".into(),
                "hysteria2",
            ),
            (
                format!("tuic://{ID}:password@example.com:443?congestion_control=bbr&alpn=h3"),
                "tuic",
            ),
            (
                format!(
                    "ss://{}@example.com:8388",
                    general_purpose::STANDARD_NO_PAD.encode("aes-256-gcm:password")
                ),
                "shadowsocks",
            ),
        ] {
            assert_eq!(parse_node_link(&link).unwrap()["type"], kind);
        }
        assert_eq!(
            parse_node_link("trojan://p%40ss+word@example.com:443").unwrap()["password"],
            "p@ss+word"
        );
    }
    #[test]
    fn vmess_json_and_security() {
        let obj = json!({"v":"2","add":"example.com","port":"443","id":ID,"net":"ws","path":"/proxy","tls":"tls","sni":"example.com"});
        let uri = format!(
            "vmess://{}",
            general_purpose::STANDARD.encode(obj.to_string())
        );
        assert_eq!(parse_node_link(&uri).unwrap()["transport"]["type"], "ws");
        for field in ["unknown", "allowInsecure"] {
            let mut v = obj.clone();
            v[field] = json!("1");
            assert_eq!(
                parse_node_link(&format!(
                    "vmess://{}",
                    general_purpose::STANDARD.encode(v.to_string())
                ))
                .unwrap_err(),
                UNSUPPORTED
            );
        }
    }
    #[test]
    fn rejects_unsupported_without_leaking() {
        for suffix in [
            "?insecure=1",
            "?unknown=TOPSECRET",
            "?type=xhttp",
            "?security=none",
            "?security=reality",
            "?path=/ignored",
        ] {
            let err = parse_node_link(&format!("trojan://TOPSECRET@example.com:443{suffix}"))
                .unwrap_err();
            assert!(!err.contains("TOPSECRET"));
        }
        assert_eq!(
            parse_node_link("trojan://pass@example.com:443?sni=a&sni=b").unwrap_err(),
            INVALID
        );
        assert_eq!(
            parse_node_link("trojan://pass@example.com:443?insecure=1").unwrap_err(),
            UNSUPPORTED
        );
        let grpc =
            parse_node_link("trojan://pass@example.com:443?type=grpc&serviceName=TunService")
                .unwrap();
        assert_eq!(
            grpc["transport"],
            json!({"type":"grpc","service_name":"TunService"})
        );
    }
    #[test]
    fn malformed_and_limits() {
        for uri in [
            "vless://bad@example.com:443",
            "trojan://p%ZZ@example.com:443",
            "trojan://p%0A@example.com:443",
            "trojan://pass@example.com:0",
            "trojan://pass@example.com:443/path",
            "vmess://!!!!",
            "ss://!!!!@example.com:443",
        ] {
            assert!(parse_node_link(uri).is_err(), "accepted malformed input");
        }
        assert!(parse_node_link(&"x".repeat(8193)).is_err());
        assert_eq!(decode("a+b%2Bc").unwrap(), "a+b+c");
    }

    #[test]
    fn httpupgrade_uses_dedicated_host_and_hy2_defaults_to_443() {
        let value = parse_node_link(
            "trojan://password@example.com:443?type=httpupgrade&host=cdn.example&path=%2Fpath",
        )
        .unwrap();
        assert_eq!(value["transport"]["host"], "cdn.example");
        assert!(value["transport"].get("headers").is_none());
        assert_eq!(
            parse_node_link("hy2://password@example.com").unwrap()["server_port"],
            443
        );
    }
}
