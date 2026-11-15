//! Source-scoped DNS and IPv6 settings for Mihomo.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;

const DEFAULT_DNS_SERVER: &str = "https://1.1.1.1/dns-query";

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkOptions {
    pub doh: bool,
    #[serde(rename = "dnsServers", skip_serializing_if = "Vec::is_empty")]
    pub dns_servers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<bool>,
    /// Read old source and account snapshots, but never persist or apply interface binding.
    #[serde(skip_serializing)]
    pub interface: String,
}
impl NetworkOptions {
    pub fn validate(&self) -> Result<(), String> {
        let n = &self.interface;
        if n.len() > 128
            || n.chars()
                .any(|c| c.is_control() || ['/', '\\', '"', '\''].contains(&c))
            || n.trim() != n
        {
            return Err("PROXY_NETWORK_INVALID".into());
        }
        if self
            .dns_servers
            .iter()
            .any(|server| !valid_dns_server(server))
        {
            return Err("PROXY_DNS_INVALID".into());
        }
        Ok(())
    }
    pub fn apply(&self, config: &mut Value) -> Result<(), String> {
        self.validate()?;
        if self.doh {
            // No listener, fake-IP, route hijacking or system DNS changes.
            let servers: Vec<&str> = if self.dns_servers.is_empty() {
                vec![DEFAULT_DNS_SERVER]
            } else {
                self.dns_servers.iter().map(String::as_str).collect()
            };
            config["dns"] = json!({"enable":true,"ipv6":self.ipv6.unwrap_or_else(|| ipv6_default(config)),
                "enhanced-mode":"redir-host","respect-rules":false,
                "default-nameserver":["system"],
                "nameserver":servers,
                "proxy-server-nameserver":servers,
                "fallback":[],"fallback-filter":{"geoip":false}});
        }
        if let Some(ipv6) = self.ipv6 {
            config["ipv6"] = json!(ipv6);
            if let Some(dns) = config.get_mut("dns").and_then(Value::as_object_mut) {
                dns.insert("ipv6".into(), json!(ipv6));
            }
        }
        Ok(())
    }
}

pub fn ipv6_default(config: &Value) -> bool {
    // Mihomo defaults its top-level IPv6 setting to true.
    config["ipv6"].as_bool().unwrap_or(true)
}

fn valid_dns_server(server: &str) -> bool {
    if server
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '\\')
    {
        return false;
    }
    if server == "system" || server.parse::<IpAddr>().is_ok() {
        return true;
    }
    let url = if let Some((_, address)) = server.split_once("://") {
        if address.starts_with('/') {
            return false;
        }
        url::Url::parse(server)
    } else {
        url::Url::parse(&format!("udp://{server}"))
    };
    url.is_ok_and(|url| {
        matches!(
            url.scheme(),
            "udp" | "tcp" | "tls" | "http" | "https" | "quic"
        ) && url.host_str().is_some_and(|host| !host.is_empty())
            && url.port() != Some(0)
            && url.username().is_empty()
            && url.password().is_none()
            && (matches!(url.scheme(), "http" | "https") || url.path().is_empty())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opt_in_dns_and_legacy_interface_does_not_bind_outbound() {
        let base = json!({"dns":{"enable":false},"proxies":[{"type":"vless"}],"proxy-groups":[{"type":"select"}]});
        let mut c = base.clone();
        NetworkOptions::default().apply(&mut c).unwrap();
        assert_eq!(c, base);
        NetworkOptions {
            doh: true,
            interface: "en0".into(),
            ..Default::default()
        }
        .apply(&mut c)
        .unwrap();
        assert!(c["proxies"][0].get("interface-name").is_none());
        assert!(c["proxy-groups"][0].get("interface-name").is_none());
        assert!(c.get("interface-name").is_none());
        assert_eq!(c["dns"]["nameserver"], json!(["https://1.1.1.1/dns-query"]));
        assert!(c["dns"].get("listen").is_none());
        assert_eq!(c["dns"]["fallback-filter"]["geoip"], false);
        let old: NetworkOptions =
            serde_json::from_value(json!({"doh":true,"interface":"en0"})).unwrap();
        assert_eq!(old.interface, "en0");
        assert_eq!(serde_json::to_value(old).unwrap(), json!({"doh":true}));
        assert!(NetworkOptions {
            doh: true,
            interface: "bad\nname".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn ipv6_switch_also_controls_dns_without_replacing_subscription_dns() {
        let mut config = json!({"ipv6": true, "dns": {"enable": true, "enhanced-mode": "fake-ip", "ipv6": true}});
        NetworkOptions {
            ipv6: Some(false),
            ..Default::default()
        }
        .apply(&mut config)
        .unwrap();
        assert_eq!(
            config,
            json!({"ipv6": false, "dns": {"enable": true, "enhanced-mode": "fake-ip", "ipv6": false}})
        );
        NetworkOptions {
            ipv6: Some(true),
            ..Default::default()
        }
        .apply(&mut config)
        .unwrap();
        assert_eq!(config["dns"]["ipv6"], true);
        assert_eq!(config["ipv6"], true);
    }

    #[test]
    fn custom_resolvers_replace_dns_and_follow_the_ipv6_setting() {
        let mut config = json!({"ipv6": false, "dns": {"listen": "127.0.0.1:53", "nameserver-policy": {"example.test": ["system"]}}});
        let network = NetworkOptions {
            doh: true,
            dns_servers: vec![
                "https://dns.example.test/dns-query".into(),
                "2001:db8::53".into(),
                "tls://dns.example.test:853".into(),
            ],
            ..Default::default()
        };
        network.apply(&mut config).unwrap();
        assert_eq!(config["dns"]["nameserver"], json!(network.dns_servers));
        assert_eq!(
            config["dns"]["proxy-server-nameserver"],
            config["dns"]["nameserver"]
        );
        assert_eq!(config["dns"]["ipv6"], false);
        assert!(config["dns"].get("listen").is_none());
        assert!(config["dns"].get("nameserver-policy").is_none());
        assert_eq!(
            NetworkOptions {
                dns_servers: vec!["https://".into()],
                ..network
            }
            .validate(),
            Err("PROXY_DNS_INVALID".into())
        );
    }
}
