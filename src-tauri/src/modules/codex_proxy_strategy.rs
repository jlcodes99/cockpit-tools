//! 自建多节点策略的纯逻辑：把用户勾选的节点副本与一个分组组装成新的来源 catalog。
//! 本模块不读写磁盘、不发起网络请求、也不接触账号：来源查询、落盘与解绑由调用方负责。
//! 保存的是原来源 outbound 的副本，账号与统一代理仍按来源 ID / 资源 ID 引用，不依赖易变的显示名。
use crate::modules::codex_proxy_subscription_parser::{
    self as parser, ParsedCatalog, ParsedGroup, ParsedNode,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// 策略来源的 kind：与 `subscription` / `manual` 并列的第三类 catalog 来源。
pub const KIND: &str = "strategy";
/// 单个策略的成员上限：与分组可执行规模、绑定后每条连接的开销保持一致。
pub const MAX_MEMBERS: usize = 64;
/// 自动分组的兜底健康检查地址，与编码缺失参数时使用的值一致。
const DEFAULT_TEST_URL: &str = "https://www.gstatic.com/generate_204";
const DEFAULT_INTERVAL: u64 = 180;
const DEFAULT_TOLERANCE: u16 = 50;
const INTERVAL_RANGE: (u64, u64) = (30, 3600);
/// 超时按秒保存，写入内核配置时再换算成毫秒；上限 30 秒足够覆盖慢节点健康检查。
const TIMEOUT_RANGE: (u64, u64) = (1, 30);
const TOLERANCE_RANGE: (u64, u64) = (0, 1000);
/// 固定错误码：校验失败不向 IPC 暴露内部细节。
const INVALID: &str = "CATALOG_INVALID";
const KINDS: [&str; 4] = ["select", "fallback", "url-test", "load-balance"];

/// 策略成员：指向某个来源里的一个受支持节点，保存时复制它的 outbound。
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyMember {
    pub source_id: String,
    pub item_id: String,
}

/// 策略参数：全部可选，缺省时使用内核与编码共用的预设值。
#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyOptions {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub interval: Option<i64>,
    #[serde(default)]
    pub timeout: Option<i64>,
    #[serde(default)]
    pub tolerance: Option<i64>,
    #[serde(default)]
    pub lazy: Option<bool>,
}

/// 待复制的节点定义：由调用方从原来源解析出来，本模块只负责组装。
pub struct StrategyNode {
    pub name: String,
    pub protocol: String,
    pub outbound: Value,
}

/// 校验后的参数：数值已收敛到编码与内核都接受的区间。
struct Resolved {
    url: Option<String>,
    interval: Option<u64>,
    timeout_ms: Option<u64>,
    tolerance: Option<u16>,
    lazy: Option<bool>,
}

/// 策略来源的四种分组类型都按 Mihomo 原生策略执行；订阅自带分组保持原有的 select/url-test 限制，
/// 因为订阅里 fallback / load-balance 的成员协议组合无法在来源导入时逐条核实。
pub fn group_supported(source_kind: &str, group_kind: &str) -> bool {
    if source_kind == KIND {
        KINDS.contains(&group_kind)
    } else {
        matches!(group_kind, "select" | "url-test")
    }
}

/// 与订阅解析器一致的稳定 ID 方案：同一个名称始终得到同一个 ID。
fn stable_id(kind: &str, value: &str) -> String {
    format!("{:x}", Sha256::digest(format!("{kind}:{value}")))[..24].into()
}

/// 分组 ID 由来源 ID 派生：改名或调整成员都不会让已有账号绑定、来源默认项失效。
pub fn group_id(source_id: &str) -> String {
    stable_id("strategy", source_id)
}

/// 来源改名时同步分组名：分组 ID 不变，已有账号绑定与来源默认项继续可用。
pub fn rename_group(catalog: &mut ParsedCatalog, name: &str) -> Result<(), String> {
    if catalog.nodes.iter().any(|node| node.name == name) {
        // 分组名与成员同名时编码会解析到节点，策略会被静默跳过。
        return Err(INVALID.into());
    }
    let group = catalog.groups.first_mut().ok_or_else(|| INVALID.to_string())?;
    group.name = name.to_owned();
    Ok(())
}

fn node_id(name: &str) -> String {
    stable_id("node", name)
}

fn bounded(value: i64, (min, max): (u64, u64)) -> Result<u64, String> {
    let value = u64::try_from(value).map_err(|_| INVALID.to_string())?;
    (min..=max)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| INVALID.to_string())
}

/// 健康检查地址不接受凭据、片段或非 http(s) 协议，避免把用户输入当成任意目标。
fn test_url(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw).map_err(|_| INVALID.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || raw.len() > 2048
    {
        return Err(INVALID.into());
    }
    Ok(raw.to_owned())
}

fn resolved(options: &StrategyOptions) -> Result<Resolved, String> {
    Ok(Resolved {
        url: options
            .url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(test_url)
            .transpose()?,
        interval: options
            .interval
            .map(|value| bounded(value, INTERVAL_RANGE))
            .transpose()?,
        timeout_ms: options
            .timeout
            .map(|value| bounded(value, TIMEOUT_RANGE))
            .transpose()?
            .map(|seconds| seconds * 1000),
        tolerance: options
            .tolerance
            .map(|value| {
                let value = bounded(value, TOLERANCE_RANGE)?;
                u16::try_from(value).map_err(|_| INVALID.to_string())
            })
            .transpose()?,
        lazy: options.lazy,
    })
}

/// 保存前校验：类型、成员与参数都必须先合法，校验失败只返回固定错误码。
pub fn validate(
    kind: &str,
    members: &[StrategyMember],
    options: &StrategyOptions,
) -> Result<(), String> {
    if !KINDS.contains(&kind) || members.is_empty() || members.len() > MAX_MEMBERS {
        return Err(INVALID.into());
    }
    let mut seen = HashSet::with_capacity(members.len());
    let invalid = members.iter().any(|member| {
        member.source_id.is_empty()
            || member.source_id.len() > 64
            || member.item_id.is_empty()
            || member.item_id.len() > 64
            || !seen.insert((member.source_id.as_str(), member.item_id.as_str()))
    });
    if invalid {
        return Err(INVALID.into());
    }
    resolved(options).map(|_| ())
}

/// 回读策略参数：编辑弹框据此恢复上次设置，避免「编辑一次就重置成默认值」。
/// 只返回脱敏后的策略参数，不含任何节点地址或凭据。
pub fn options_view(catalog: &ParsedCatalog) -> Option<Value> {
    let group = catalog.groups.first()?;
    let native = group.native.as_ref();
    let mut value = json!({ "kind": group.kind });
    let object = value.as_object_mut()?;
    if let Some(url) = group
        .url
        .as_deref()
        .or_else(|| native.and_then(|value| value.get("url")).and_then(Value::as_str))
    {
        object.insert("url".into(), json!(url));
    }
    if let Some(interval) = group
        .interval
        .or_else(|| native.and_then(|value| value.get("interval")).and_then(Value::as_u64))
    {
        object.insert("interval".into(), json!(interval));
    }
    let tolerance = group
        .tolerance
        .map(u64::from)
        .or_else(|| native.and_then(|value| value.get("tolerance")).and_then(Value::as_u64));
    if let Some(tolerance) = tolerance {
        object.insert("tolerance".into(), json!(tolerance));
    }
    // 落盘的是毫秒，表单与校验都用秒。
    if let Some(timeout_ms) = native
        .and_then(|value| value.get("timeout"))
        .and_then(Value::as_u64)
    {
        object.insert("timeout".into(), json!(timeout_ms / 1000));
    }
    if let Some(lazy) = native
        .and_then(|value| value.get("lazy"))
        .and_then(Value::as_bool)
    {
        object.insert("lazy".into(), json!(lazy));
    }
    Some(value)
}

/// 分组写进内核时的原生配置：select 不做健康检查，load-balance 不支持 timeout。
fn native(kind: &str, resolved: &Resolved) -> Value {
    if kind == "select" {
        return json!({"type": "select"});
    }
    let mut native = json!({
        "type": kind,
        "url": resolved.url.clone().unwrap_or_else(|| DEFAULT_TEST_URL.into()),
        "interval": resolved.interval.unwrap_or(DEFAULT_INTERVAL),
    });
    if kind == "url-test" {
        native["tolerance"] = json!(resolved.tolerance.unwrap_or(DEFAULT_TOLERANCE));
    }
    if kind != "load-balance" {
        if let Some(timeout) = resolved.timeout_ms {
            native["timeout"] = json!(timeout);
        }
    }
    if let Some(lazy) = resolved.lazy {
        native["lazy"] = json!(lazy);
    }
    native
}

/// 组装策略来源的 catalog：节点按名称去重并保留勾选顺序，分组 members 的顺序即主备顺序。
pub fn build(
    name: &str,
    kind: &str,
    options: &StrategyOptions,
    group_id: String,
    copies: Vec<StrategyNode>,
) -> Result<ParsedCatalog, String> {
    if !KINDS.contains(&kind) {
        return Err(INVALID.into());
    }
    let resolved = resolved(options)?;
    let mut seen = HashSet::with_capacity(copies.len());
    let mut nodes = Vec::with_capacity(copies.len());
    let mut members = Vec::with_capacity(copies.len());
    for copy in copies {
        // 同名节点只保留首次勾选的定义，避免内核配置里出现重复的代理名。
        if !seen.insert(copy.name.clone()) {
            continue;
        }
        members.push(copy.name.clone());
        nodes.push(ParsedNode {
            id: node_id(&copy.name),
            name: copy.name,
            protocol: copy.protocol,
            outbound: Some(copy.outbound),
            error: None,
        });
    }
    // 分组名与成员同名时，编码会按名称优先解析到节点，策略组会被静默跳过。
    if members.is_empty()
        || members.len() > MAX_MEMBERS
        || members.iter().any(|member| member == name)
    {
        return Err(INVALID.into());
    }
    let auto = kind != "select";
    let group = ParsedGroup {
        id: group_id,
        name: name.to_owned(),
        kind: kind.to_owned(),
        members,
        url: auto.then(|| {
            resolved
                .url
                .clone()
                .unwrap_or_else(|| DEFAULT_TEST_URL.into())
        }),
        interval: auto.then(|| resolved.interval.unwrap_or(DEFAULT_INTERVAL)),
        tolerance: (kind == "url-test")
            .then(|| resolved.tolerance.unwrap_or(DEFAULT_TOLERANCE)),
        error: None,
        native: Some(native(kind, &resolved)),
    };
    let mut catalog = ParsedCatalog {
        nodes,
        groups: vec![group],
    };
    // 用解析器同一套规则确认成员可解析，避免落盘一个当前就无法执行的分组。
    parser::validate(&mut catalog).map_err(|_| INVALID.to_string())?;
    if catalog
        .groups
        .first()
        .is_some_and(|group| group.error.is_some())
    {
        return Err(INVALID.into());
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::codex_proxy_catalog_binding as binding;

    fn copies(raw: &str) -> Vec<StrategyNode> {
        parser::parse(raw)
            .unwrap()
            .nodes
            .into_iter()
            .map(|node| StrategyNode {
                name: node.name,
                protocol: node.protocol,
                outbound: node.outbound.unwrap(),
            })
            .collect()
    }

    fn nodes() -> Vec<StrategyNode> {
        copies(
            "proxies:\n  - {name: Alpha, type: trojan, server: alpha.example, port: 443, password: private-alpha}\n  - {name: Beta, type: trojan, server: beta.example, port: 443, password: private-beta}\n  - {name: Gamma, type: trojan, server: gamma.example, port: 443, password: private-gamma}\n",
        )
    }

    fn member(source: &str, item: &str) -> StrategyMember {
        StrategyMember {
            source_id: source.into(),
            item_id: item.into(),
        }
    }

    /// 保存路径会先序列化 catalog，回读必须在同一条路径上成立。
    fn roundtrip_options(catalog: &ParsedCatalog) -> Value {
        let stored = serde_json::to_string(catalog).unwrap();
        options_view(&serde_json::from_str::<ParsedCatalog>(&stored).unwrap()).unwrap()
    }

    /// 编码后的分组必须按勾选顺序解析出成员，四种策略类型都要能落成 Mihomo 组。
    fn encoded_group(
        catalog: &ParsedCatalog,
        selections: &std::collections::BTreeMap<String, String>,
    ) -> Value {
        let group = catalog.groups[0].id.clone();
        let raw = binding::encode("strategy-source", "Strategy", &group, catalog, selections)
            .unwrap();
        assert!(binding::decode(&raw).is_ok());
        let names = binding::names(&raw).unwrap();
        let outbounds = binding::outbounds(&raw).unwrap();
        let encoded = outbounds
            .iter()
            .find(|outbound| outbound["type"] == "mihomo-group")
            .unwrap()
            .clone();
        let members = encoded["group"]["proxies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| names[tag.as_str().unwrap()].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            members,
            match catalog.groups[0].kind.as_str() {
                // 手选组编码后只保留被选中的成员，其余分组按勾选顺序编码。
                "select" => vec![selections[&group].clone()],
                _ => catalog.groups[0].members.clone(),
            },
            "编码后的成员必须与选择或主备顺序一致"
        );
        encoded
    }

    #[test]
    fn every_strategy_kind_encodes_into_a_mihomo_group_with_options() {
        let options = StrategyOptions {
            url: Some("https://health.example/generate_204".into()),
            interval: Some(60),
            timeout: Some(5),
            tolerance: Some(80),
            lazy: Some(true),
        };
        let mut select = std::collections::BTreeMap::new();
        for kind in KINDS {
            let catalog = build(
                "Strategy",
                kind,
                &options,
                "group-id".into(),
                nodes(),
            )
            .unwrap();
            assert_eq!(catalog.groups[0].members, ["Alpha", "Beta", "Gamma"]);
            assert_eq!(catalog.groups[0].kind, kind);
            if kind == "select" {
                select.insert(catalog.groups[0].id.clone(), "Gamma".to_owned());
            }
            let encoded = encoded_group(&catalog, &select);
            let group = &encoded["group"];
            assert_eq!(group["type"], kind);
            if kind == "select" {
                assert_eq!(
                    group["proxies"].as_array().unwrap().len(),
                    1,
                    "select 组只编码被选中的成员"
                );
                assert!(group.get("url").is_none());
                continue;
            }
            assert_eq!(group["url"], "https://health.example/generate_204");
            assert_eq!(group["interval"], 60);
            assert_eq!(group["lazy"], true);
            // 超时按秒保存，写入内核配置时换算成毫秒。
            assert_eq!(group.get("timeout").map(Value::as_u64).flatten(), (kind != "load-balance").then_some(5000));
            assert_eq!(
                group.get("tolerance").map(Value::as_u64).flatten(),
                (kind == "url-test").then_some(80)
            );
        }
    }

    /// 缺省参数沿用内核与编码的预设值，不把未知选项写进内核配置。
    #[test]
    fn missing_options_fall_back_to_presets() {
        let catalog = build(
            "Strategy",
            "url-test",
            &StrategyOptions::default(),
            "group-id".into(),
            nodes(),
        )
        .unwrap();
        let group = encoded_group(&catalog, &std::collections::BTreeMap::new())["group"].clone();
        assert_eq!(group["url"], DEFAULT_TEST_URL);
        assert_eq!(group["interval"], DEFAULT_INTERVAL);
        assert_eq!(group["tolerance"], DEFAULT_TOLERANCE);
        assert!(group.get("timeout").is_none());
        assert!(group.get("lazy").is_none());
        assert_eq!(catalog.groups[0].tolerance, Some(DEFAULT_TOLERANCE));
    }

    /// 保存后再回读必须与写入值一致：超时按秒返还，未设置的参数不能冒充已设置。
    #[test]
    fn options_view_roundtrips_saved_and_missing_options() {
        let options = StrategyOptions {
            url: Some("https://health.example/generate_204".into()),
            interval: Some(90),
            timeout: Some(7),
            tolerance: Some(120),
            lazy: Some(false),
        };
        let catalog = build("Strategy", "url-test", &options, "group-id".into(), nodes()).unwrap();
        // 落盘的是毫秒，表单与校验都用秒。
        assert_eq!(catalog.groups[0].native.as_ref().unwrap()["timeout"], 7000);
        let view = roundtrip_options(&catalog);
        assert_eq!(view["kind"], "url-test");
        assert_eq!(view["url"], "https://health.example/generate_204");
        assert_eq!(view["interval"], 90);
        assert_eq!(view["timeout"], 7);
        assert_eq!(view["tolerance"], 120);
        assert_eq!(view["lazy"], false, "lazy 为 false 时同样要回读");
        // 回读只含类型与已保存的选项字段。
        assert_eq!(view.as_object().unwrap().len(), 6);

        let defaults = build(
            "Strategy",
            "url-test",
            &StrategyOptions::default(),
            "group-id".into(),
            nodes(),
        )
        .unwrap();
        let view = roundtrip_options(&defaults);
        assert_eq!(view["url"], DEFAULT_TEST_URL);
        assert_eq!(view["interval"], DEFAULT_INTERVAL);
        assert_eq!(view["tolerance"], DEFAULT_TOLERANCE);
        assert!(view.get("timeout").is_none(), "未设置的 timeout 回读为空");
        assert!(view.get("lazy").is_none(), "未设置的 lazy 回读为空");
    }

    #[test]
    fn duplicated_node_names_keep_the_first_copy_and_the_selection_order() {
        let mut list = nodes();
        list.swap(0, 2);
        list.push(StrategyNode {
            name: "Alpha".into(),
            protocol: "trojan".into(),
            outbound: json!({"type": "mihomo", "proxy": {"name": "other", "type": "trojan"}}),
        });
        let catalog = build(
            "Strategy",
            "fallback",
            &StrategyOptions::default(),
            "group-id".into(),
            list,
        )
        .unwrap();
        assert_eq!(catalog.groups[0].members, ["Gamma", "Beta", "Alpha"]);
        assert_eq!(catalog.nodes.len(), 3);
        assert_eq!(
            catalog.nodes[2].outbound.as_ref().unwrap()["proxy"]["server"],
            "alpha.example"
        );
    }

    #[test]
    fn build_rejects_unknown_kinds_and_group_names_that_shadow_a_member() {
        for kind in ["relay", "auto", "", "SELECT"] {
            assert_eq!(
                build("Strategy", kind, &StrategyOptions::default(), "group-id".into(), nodes())
                    .err()
                    .unwrap(),
                INVALID
            );
        }
        assert_eq!(
            build("Alpha", "fallback", &StrategyOptions::default(), "group-id".into(), nodes())
                .err()
                .unwrap(),
            INVALID,
            "分组名与成员同名时编码会解析到节点，必须拒绝保存"
        );
        assert_eq!(
            build("Strategy", "fallback", &StrategyOptions::default(), "group-id".into(), vec![])
                .err()
                .unwrap(),
            INVALID
        );
    }

    /// 输入校验必须在落盘前拦住重复、超限与越界数值。
    #[test]
    fn input_validation_rejects_duplicates_limits_and_out_of_range_options() {
        let list = [member("source-a", "node-a"), member("source-b", "node-b")];
        assert!(validate("fallback", &list, &StrategyOptions::default()).is_ok());
        assert_eq!(
            validate("fallback", &[], &StrategyOptions::default()).unwrap_err(),
            INVALID
        );
        assert_eq!(
            validate("relay", &list, &StrategyOptions::default()).unwrap_err(),
            INVALID
        );
        assert_eq!(
            validate(
                "fallback",
                &[member("source-a", "node-a"), member("source-a", "node-a")],
                &StrategyOptions::default()
            )
            .unwrap_err(),
            INVALID,
            "同一个来源里的同一个节点不能重复出现"
        );
        let mut overflow = Vec::new();
        for index in 0..=MAX_MEMBERS {
            overflow.push(member("source-a", &format!("node-{index}")));
        }
        assert_eq!(
            validate("fallback", &overflow, &StrategyOptions::default()).unwrap_err(),
            INVALID
        );
        assert_eq!(
            validate("fallback", &[member("", "node-a")], &StrategyOptions::default()).unwrap_err(),
            INVALID
        );
        let options = |interval, timeout, tolerance| StrategyOptions {
            url: None,
            interval,
            timeout,
            tolerance,
            lazy: None,
        };
        assert!(validate("fallback", &list, &options(Some(30), Some(1), Some(0))).is_ok());
        assert!(validate("fallback", &list, &options(Some(3600), Some(30), Some(1000))).is_ok());
        for invalid in [
            options(Some(29), None, None),
            options(Some(3601), None, None),
            options(Some(-1), None, None),
            options(None, Some(0), None),
            options(None, Some(31), None),
            options(None, None, Some(1001)),
            options(None, None, Some(-1)),
        ] {
            assert_eq!(validate("fallback", &list, &invalid).unwrap_err(), INVALID);
        }
        for url in [
            Some(""),
            Some("  "),
            Some("https://health.example/204"),
            Some("http://health.example"),
        ] {
            assert!(
                validate(
                    "fallback",
                    &list,
                    &StrategyOptions {
                        url: url.map(str::to_owned),
                        ..Default::default()
                    }
                )
                .is_ok(),
                "{url:?}"
            );
        }
        for url in [
            "ftp://health.example/204",
            "https://user:secret@health.example/204",
            "https://health.example/204#fragment",
            "not a url",
        ] {
            assert_eq!(
                validate(
                    "fallback",
                    &list,
                    &StrategyOptions {
                        url: Some(url.into()),
                        ..Default::default()
                    }
                )
                .unwrap_err(),
                INVALID,
                "{url}"
            );
        }
    }

    /// 策略来源能引用全部四种分组类型；订阅来源维持原有 select/url-test 限制。
    #[test]
    fn strategy_sources_support_every_group_kind_without_relaxing_subscriptions() {
        for kind in KINDS {
            assert!(group_supported(KIND, kind), "{kind}");
        }
        assert!(!group_supported(KIND, "relay"));
        for kind in ["select", "url-test"] {
            assert!(group_supported("subscription", kind), "{kind}");
        }
        for kind in ["fallback", "load-balance", "relay"] {
            assert!(!group_supported("subscription", kind), "{kind}");
            assert!(!group_supported("manual", kind), "{kind}");
        }
    }
}
