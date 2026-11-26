//! Canonical Model ID, Display Name, and API Provider Resolver.
//!
//! Reconciles numeric protobuf enums (`1318`, `1427`, ...), `MODEL_PLACEHOLDER_M*` keys,
//! dynamically harvested RPC cache mappings, `gen_metadata` `responseModel` aliases,
//! and Swarm `members.json` friendly model names.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelInfo {
    pub model_id: String,
    pub display_name: String,
    pub api_provider: String,
}

#[derive(Debug, Clone, Default)]
pub struct ModelResolutionInput<'a> {
    pub friendly_model: Option<&'a str>,
    pub raw_model_enum: Option<&'a str>,
    pub response_model: Option<&'a str>,
    pub api_provider: Option<&'a str>,
    pub dynamic_enum_map: Option<&'a HashMap<String, String>>,
}

#[inline]
pub fn is_builtin_enum_u64(raw_num: u64) -> bool {
    matches!(
        raw_num,
        246 | 281
            | 282
            | 290
            | 291
            | 312
            | 313
            | 327
            | 329
            | 330
            | 331
            | 332
            | 333
            | 334
            | 340
            | 341
            | 342
            | 1007
            | 1008
            | 1012
            | 1018
            | 1026
            | 1035
            | 1036
            | 1037
            | 1318
            | 1319
            | 1320
            | 1322
            | 1400
            | 1401
            | 1402
            | 1403
            | 1404
            | 1405
            | 1417
            | 1418
            | 1419
            | 1427
            | 1437
            | 1438
            | 1525
            | 1526
            | 1527
            | 1528
            | 1530
            | 1531
            | 1578
            | 1667
            | 1668
            | 1669
            | 1670
            | 1681
    )
}

pub fn lookup_builtin_enum_model(raw_key: &str) -> Option<&'static str> {
    match raw_key.trim() {
        "246" | "327" | "331" => Some("gemini-2.5-pro"),
        "281" | "282" => Some("claude-sonnet-4"),
        "290" | "291" => Some("claude-opus-4"),
        "312" | "313" | "329" | "332" => Some("gemini-2.5-flash"),
        "330" => Some("gemini-2.5-flash-lite"),
        "333" | "334" => Some("claude-sonnet-4-5-thinking"),
        "340" | "341" => Some("claude-haiku-4-5"),
        "342" | "MODEL_OPENAI_GPT_OSS_120B_MEDIUM" => Some("gpt-oss-120b-medium"),
        "1007" | "MODEL_PLACEHOLDER_M7" => Some("gemini-3-pro-high"),
        "1008" | "MODEL_PLACEHOLDER_M8" => Some("gemini-3-pro-low"),
        "1012" | "MODEL_PLACEHOLDER_M12" => Some("claude-opus-4-5-thinking"),
        "1018" | "MODEL_PLACEHOLDER_M18" => Some("gemini-3-flash"),
        "1026" | "MODEL_PLACEHOLDER_M26" => Some("claude-opus-4-6-thinking"),
        "1035" | "MODEL_PLACEHOLDER_M35" => Some("claude-sonnet-4-6-thinking"),
        "1036" | "MODEL_PLACEHOLDER_M36" => Some("gemini-3.1-pro-low"),
        "1037" | "MODEL_PLACEHOLDER_M37" => Some("gemini-3.1-pro-high"),
        "1318"
        | "1319"
        | "1320"
        | "MODEL_PLACEHOLDER_M318"
        | "MODEL_PLACEHOLDER_M319"
        | "MODEL_PLACEHOLDER_M320" => Some("gemini-3.8-flash-high"),
        "1322" | "MODEL_PLACEHOLDER_M322" => Some("gemini-3.8-flash-tiered"),
        "1400" | "MODEL_PLACEHOLDER_M400" => Some("claude-opus-5-5-low"),
        "1401" | "MODEL_PLACEHOLDER_M401" => Some("claude-opus-5-5-medium"),
        "1402" | "MODEL_PLACEHOLDER_M402" => Some("claude-opus-5-5-high"),
        "1403" | "MODEL_PLACEHOLDER_M403" => Some("claude-sonnet-5-5-low"),
        "1404" | "MODEL_PLACEHOLDER_M404" => Some("claude-sonnet-5-5-medium"),
        "1405" | "MODEL_PLACEHOLDER_M405" => Some("claude-sonnet-5-5-high"),
        "1417" | "MODEL_PLACEHOLDER_M417" => Some("gemini-4-argon-high"),
        "1418" | "MODEL_PLACEHOLDER_M418" => Some("gemini-4-argon-medium"),
        "1419" | "MODEL_PLACEHOLDER_M419" => Some("gemini-4-argon-low"),
        "1427" | "1670" | "MODEL_PLACEHOLDER_M427" | "MODEL_PLACEHOLDER_M670" => {
            Some("gemini-4-carbon-high")
        }
        "1437" | "MODEL_PLACEHOLDER_M437" => Some("gemini-4-neon-high"),
        "1438" | "MODEL_PLACEHOLDER_M438" => Some("gemini-4-neon-medium"),
        "1525" | "1530" | "MODEL_PLACEHOLDER_M525" | "MODEL_PLACEHOLDER_M530" => {
            Some("gemini-4-hydrogen-high")
        }
        "1526" | "MODEL_PLACEHOLDER_M526" => Some("gemini-4-hydrogen-medium"),
        "1527" | "MODEL_PLACEHOLDER_M527" => Some("gemini-4-hydrogen-low"),
        "1528" | "1578" | "MODEL_PLACEHOLDER_M528" | "MODEL_PLACEHOLDER_M578" => {
            Some("gemini-4-hydrogen")
        }
        "1531" | "MODEL_PLACEHOLDER_M531" => Some("gemini-4-hydrogen-xhigh"),
        "1667" | "MODEL_PLACEHOLDER_M667" => Some("gemini-pro-orchestrator"),
        "1668" | "1681" | "MODEL_PLACEHOLDER_M668" | "MODEL_PLACEHOLDER_M681" => {
            Some("gemini-argon-orchestrator")
        }
        "1669" | "MODEL_PLACEHOLDER_M669" => Some("gemini-flash-worker-agy"),
        _ => None,
    }
}

pub fn lookup_response_model(response_model: &str) -> Option<&'static str> {
    match response_model.trim().to_ascii_lowercase().as_str() {
        "gemini-3.8-flash" | "gemini-3.8-flash-n" | "gemini-3.8-flash-control" => {
            Some("gemini-3.8-flash-high")
        }
        "gemini-3.8-flash-tiered" => Some("gemini-3.8-flash-tiered"),
        "gemini-4-argon" => Some("gemini-4-argon-high"),
        "gemini-4-carbon" => Some("gemini-4-carbon-high"),
        "gemini-4-hydrogen-extra-high"
        | "gemini-4-hydrogen-tiered-xhigh"
        | "gemini 4 hydrogen extra high"
        | "gemini 4 hydrogen (extra high)"
        | "gemini 4 hydrogen (xhigh)" => Some("gemini-4-hydrogen-xhigh"),
        "gemini-4-hydrogen-tiered-high"
        | "gemini 4 hydrogen high"
        | "gemini 4 hydrogen (high)" => Some("gemini-4-hydrogen-high"),
        "claude-opus-5-5-medium" => Some("claude-opus-5-5-medium"),
        "claude-opus-5-5-high" => Some("claude-opus-5-5-high"),
        _ => None,
    }
}

pub fn resolve_model_context_limit(model_id: &str) -> u64 {
    let lower = model_id.trim().to_ascii_lowercase();
    if lower.starts_with("claude-") {
        200_000
    } else if lower.starts_with("gpt-") {
        128_000
    } else if lower.contains("argon") || lower.contains("hydrogen") || lower.contains("neon") || lower.contains("carbon") || lower.starts_with("gemini-") {
        // Default Antigravity desktop checkpoint clamp is 256,000 tokens
        256_000
    } else {
        256_000
    }
}

pub fn canonicalize_model_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(mapped) = lookup_response_model(trimmed) {
        return mapped.to_string();
    }
    if let Some(mapped) = lookup_builtin_enum_model(trimmed) {
        return mapped.to_string();
    }
    trimmed.to_ascii_lowercase()
}

fn is_placeholder_or_numeric(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    let upper = t.to_ascii_uppercase();
    if upper == "UNKNOWN" || upper.starts_with("MODEL_PLACEHOLDER_") {
        return true;
    }
    t.bytes().all(|b| b.is_ascii_digit())
}

fn lookup_dynamic_enum_map(
    raw_enum: &str,
    dynamic_map: &HashMap<String, String>,
) -> Option<String> {
    let key = raw_enum.trim();
    if key.is_empty() {
        return None;
    }
    if let Some(found) = dynamic_map.get(key) {
        if !found.trim().is_empty() && !is_placeholder_or_numeric(found) {
            return Some(canonicalize_model_id(found));
        }
    }
    if let Ok(num) = key.parse::<u64>() {
        if num >= 1000 {
            let placeholder = format!("MODEL_PLACEHOLDER_M{}", num - 1000);
            if let Some(found) = dynamic_map.get(&placeholder) {
                if !found.trim().is_empty() && !is_placeholder_or_numeric(found) {
                    return Some(canonicalize_model_id(found));
                }
            }
        }
    }
    None
}

fn fallback_enum_slug(raw_enum: &str) -> Option<String> {
    let trimmed = raw_enum.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(num) = trimmed.parse::<u64>() {
        if num >= 1000 {
            return Some(format!("model-m{}", num - 1000));
        }
        if num > 0 {
            return Some(format!("model-{}", num));
        }
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    if let Some(suffix) = upper.strip_prefix("MODEL_PLACEHOLDER_M") {
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Some(format!("model-m{}", suffix.to_ascii_lowercase()));
        }
    }
    if let Some(suffix) = upper.strip_prefix("MODEL_PLACEHOLDER_") {
        let slug: String = suffix
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let clean = slug.trim_matches('-');
        if !clean.is_empty() {
            return Some(format!("model-{}", clean));
        }
    }
    None
}

pub fn format_model_display_name(model_id: &str) -> String {
    let trimmed = model_id.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown") {
        return "Unknown".to_string();
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "gemini-3.8-flash-high" => return "Gemini 3.8 Flash (High)".to_string(),
        "gemini-3.8-flash-tiered" => return "Gemini 3.8 Flash (Tiered)".to_string(),
        "gemini-4-argon-high" => return "Gemini 4 Argon (High)".to_string(),
        "gemini-4-carbon-high" => return "Gemini 4 Carbon (High)".to_string(),
        "claude-opus-5-5-high" => return "Claude Opus 5.5 (High)".to_string(),
        "claude-opus-5-5-medium" => return "Claude Opus 5.5 (Medium)".to_string(),
        "gpt-oss-120b-medium" => return "GPT-OSS 120B (Medium)".to_string(),
        _ => {}
    }

    let parts: Vec<&str> = trimmed.split('-').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return "Unknown".to_string();
    }

    let (body_parts, tier_suffix) = if parts.len() >= 2 {
        match parts[parts.len() - 1].to_ascii_lowercase().as_str() {
            "high" => (&parts[..parts.len() - 1], Some("(High)")),
            "medium" => (&parts[..parts.len() - 1], Some("(Medium)")),
            "low" => (&parts[..parts.len() - 1], Some("(Low)")),
            "tiered" => (&parts[..parts.len() - 1], Some("(Tiered)")),
            "xhigh" => (&parts[..parts.len() - 1], Some("(XHigh)")),
            "minimal" => (&parts[..parts.len() - 1], Some("(Minimal)")),
            _ => (&parts[..], None),
        }
    } else {
        (&parts[..], None)
    };

    let mut formatted_tokens: Vec<String> = Vec::with_capacity(body_parts.len());
    let mut i = 0;
    while i < body_parts.len() {
        let tok = body_parts[i];
        let lower = tok.to_ascii_lowercase();

        if tok.bytes().all(|b| b.is_ascii_digit())
            && i + 1 < body_parts.len()
            && body_parts[i + 1].bytes().all(|b| b.is_ascii_digit())
        {
            formatted_tokens.push(format!("{}.{}", tok, body_parts[i + 1]));
            i += 2;
            continue;
        }

        let formatted = match lower.as_str() {
            "gpt" => "GPT".to_string(),
            "oss" => "OSS".to_string(),
            "ai" => "AI".to_string(),
            "llm" => "LLM".to_string(),
            _ if lower.starts_with('m')
                && lower[1..].bytes().all(|b| b.is_ascii_digit())
                && lower.len() > 1 =>
            {
                lower.to_ascii_uppercase()
            }
            _ if lower.ends_with('b')
                && lower[..lower.len() - 1].bytes().all(|b| b.is_ascii_digit())
                && lower.len() > 1 =>
            {
                lower.to_ascii_uppercase()
            }
            _ => {
                let mut chars = tok.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                    None => String::new(),
                }
            }
        };
        formatted_tokens.push(formatted);
        i += 1;
    }

    let mut out = formatted_tokens.join(" ");
    if let Some(tier) = tier_suffix {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(tier);
    }
    out
}

pub fn resolve_api_provider(raw_provider: Option<&str>, model_id: &str) -> String {
    if let Some(raw) = raw_provider.map(str::trim).filter(|s| !s.is_empty()) {
        match raw.to_ascii_uppercase().as_str() {
            "24" | "API_PROVIDER_GOOGLE_GEMINI" => return "Google Gemini".to_string(),
            "26" | "API_PROVIDER_ANTHROPIC_VERTEX" => return "Anthropic Vertex".to_string(),
            "API_PROVIDER_ANTHROPIC" => return "Anthropic".to_string(),
            "API_PROVIDER_OPENAI" => return "OpenAI".to_string(),
            _ => {}
        }
    }

    let lower = model_id.trim().to_ascii_lowercase();
    if lower.starts_with("gemini-") {
        "Google Gemini".to_string()
    } else if lower.starts_with("claude-") {
        "Anthropic Vertex".to_string()
    } else if lower.starts_with("gpt-") {
        "OpenAI".to_string()
    } else {
        "Antigravity".to_string()
    }
}

pub fn resolve_model_info(input: ModelResolutionInput<'_>) -> ResolvedModelInfo {
    let friendly_trimmed = input
        .friendly_model
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let effective_raw_enum = input
        .raw_model_enum
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| friendly_trimmed.filter(|s| is_placeholder_or_numeric(s)));

    let model_id = if let Some(friendly) =
        friendly_trimmed.filter(|s| !is_placeholder_or_numeric(s))
    {
        canonicalize_model_id(friendly)
    } else if let Some(mapped_dyn) = effective_raw_enum.and_then(|raw| {
        input
            .dynamic_enum_map
            .and_then(|m| lookup_dynamic_enum_map(raw, m))
    }) {
        mapped_dyn
    } else if let Some(mapped_builtin) = effective_raw_enum.and_then(lookup_builtin_enum_model) {
        mapped_builtin.to_string()
    } else if let Some(resp) = input
        .response_model
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(mapped_resp) = lookup_response_model(resp) {
            mapped_resp.to_string()
        } else if !is_placeholder_or_numeric(resp) {
            canonicalize_model_id(resp)
        } else {
            effective_raw_enum
                .and_then(fallback_enum_slug)
                .unwrap_or_else(|| "unknown".to_string())
        }
    } else if let Some(slug) = effective_raw_enum.and_then(fallback_enum_slug) {
        slug
    } else {
        "unknown".to_string()
    };

    let display_name = format_model_display_name(&model_id);
    let api_provider = resolve_api_provider(input.api_provider, &model_id);

    ResolvedModelInfo {
        model_id,
        display_name,
        api_provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_builtin_and_dynamic_enums() {
        let info = resolve_model_info(ModelResolutionInput {
            raw_model_enum: Some("1427"),
            ..Default::default()
        });
        assert_eq!(info.model_id, "gemini-4-carbon-high");
        assert_eq!(info.display_name, "Gemini 4 Carbon (High)");
        assert_eq!(info.api_provider, "Google Gemini");

        let mut dyn_map = HashMap::new();
        dyn_map.insert(
            "MODEL_PLACEHOLDER_M528".to_string(),
            "gemini-4-hydrogen-xhigh".to_string(),
        );
        let dyn_resolved = resolve_model_info(ModelResolutionInput {
            raw_model_enum: Some("1528"),
            api_provider: Some("24"),
            dynamic_enum_map: Some(&dyn_map),
            ..Default::default()
        });
        assert_eq!(dyn_resolved.model_id, "gemini-4-hydrogen-xhigh");
        assert_eq!(dyn_resolved.display_name, "Gemini 4 Hydrogen (XHigh)");
    }
}
