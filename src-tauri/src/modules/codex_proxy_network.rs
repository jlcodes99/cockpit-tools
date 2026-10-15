//! Source-scoped DNS settings. Never change host routes or system DNS.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NetworkOptions {
    pub doh: bool,
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
        Ok(())
    }
    pub fn apply(&self, config: &mut Value) -> Result<(), String> {
        self.validate()?;
        if self.doh {
            // No listener, fake-IP, route hijacking or system DNS changes.
            // An IP literal avoids a bootstrap lookup; TLS still verifies it.
            config["dns"] = json!({"enable":true,"ipv6":true,
                "enhanced-mode":"redir-host","respect-rules":false,
                "default-nameserver":["system"],
                "nameserver":["https://1.1.1.1/dns-query"],
                "proxy-server-nameserver":["https://1.1.1.1/dns-query"],
                "fallback":[],"fallback-filter":{"geoip":false}});
        }
        Ok(())
    }
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
            interface: "bad\nname".into()
        }
        .validate()
        .is_err());
    }
}
