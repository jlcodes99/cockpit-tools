//! Reference API cost estimator for Antigravity local token usage.
//!
//! Computes API-equivalent reference costs (USD) and prompt-cache savings from
//! `ExactTokenTotals` without double-counting `thinking_tokens`
//! (since `ExactTokenTotals.output_tokens` already excludes `thinking_tokens`).

use super::types::ExactTokenTotals;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricingRate {
    pub input_usd_per_million: f64,
    pub output_usd_per_million: f64,
    pub cache_read_usd_per_million: f64,
    pub cache_write_usd_per_million: f64,
}

/// Resolves reference API pricing rates (USD per 1M tokens) by canonical `model_id`.
///
/// Rates follow official Google Gemini, Anthropic Claude, and OpenAI reference pricing:
/// - Cache reads use the 10% discounted prompt-caching rate.
/// - Thinking tokens are billed at the model's output token rate (combined with `output_tokens`).
pub fn resolve_model_pricing(model_id: &str) -> ModelPricingRate {
    let lower = model_id.trim().to_ascii_lowercase();

    if lower.starts_with("claude-opus-") {
        // Claude Opus 4.5 / 4.6 / 5.5 reference API rate
        ModelPricingRate {
            input_usd_per_million: 5.00,
            output_usd_per_million: 25.00,
            cache_read_usd_per_million: 0.50,
            cache_write_usd_per_million: 6.25,
        }
    } else if lower.starts_with("claude-sonnet-") {
        // Claude Sonnet 4.5 / 4.6 / 5.5 reference API rate
        ModelPricingRate {
            input_usd_per_million: 3.00,
            output_usd_per_million: 15.00,
            cache_read_usd_per_million: 0.30,
            cache_write_usd_per_million: 3.75,
        }
    } else if lower.starts_with("claude-haiku-") {
        ModelPricingRate {
            input_usd_per_million: 1.00,
            output_usd_per_million: 5.00,
            cache_read_usd_per_million: 0.10,
            cache_write_usd_per_million: 1.25,
        }
    } else if lower.contains("argon")
        || lower.contains("carbon")
        || lower.contains("pro-orchestrator")
        || lower.contains("3.1-pro")
        || lower.contains("3-pro")
        || lower.contains("2.5-pro")
    {
        // Gemini 4 Argon / Carbon & Gemini Pro tier reference API rate
        ModelPricingRate {
            input_usd_per_million: 1.25,
            output_usd_per_million: 10.00,
            cache_read_usd_per_million: 0.125,
            cache_write_usd_per_million: 1.25,
        }
    } else if lower.contains("hydrogen") || lower.contains("neon") {
        // Gemini 4 Hydrogen / Neon reference API rate
        ModelPricingRate {
            input_usd_per_million: 0.50,
            output_usd_per_million: 3.00,
            cache_read_usd_per_million: 0.05,
            cache_write_usd_per_million: 0.50,
        }
    } else if lower.starts_with("gpt-") {
        ModelPricingRate {
            input_usd_per_million: 0.50,
            output_usd_per_million: 1.50,
            cache_read_usd_per_million: 0.05,
            cache_write_usd_per_million: 0.50,
        }
    } else {
        // Default / Gemini 3.8 Flash / 3.0 Flash / worker tier reference API rate
        ModelPricingRate {
            input_usd_per_million: 0.30,
            output_usd_per_million: 2.50,
            cache_read_usd_per_million: 0.03,
            cache_write_usd_per_million: 0.30,
        }
    }
}

#[inline]
fn round_usd(val: f64) -> f64 {
    if !val.is_finite() || val <= 0.0 {
        0.0
    } else {
        (val * 10_000.0).round() / 10_000.0
    }
}

/// Returns `(estimated_cost_usd, cache_savings_usd)` for exact API-reported token counts.
pub fn estimate_exact_cost_usd(model_id: &str, exact: &ExactTokenTotals) -> (f64, f64) {
    let rate = resolve_model_pricing(model_id);
    let combined_output = exact.output_tokens.saturating_add(exact.thinking_tokens);

    let input_cost = (exact.input_tokens as f64) * rate.input_usd_per_million / 1_000_000.0;
    let output_cost = (combined_output as f64) * rate.output_usd_per_million / 1_000_000.0;
    let cache_read_cost =
        (exact.cache_read_tokens as f64) * rate.cache_read_usd_per_million / 1_000_000.0;
    let cache_write_cost =
        (exact.cache_write_tokens as f64) * rate.cache_write_usd_per_million / 1_000_000.0;

    let total_cost = input_cost + output_cost + cache_read_cost + cache_write_cost;
    let savings = (exact.cache_read_tokens as f64)
        * (rate.input_usd_per_million - rate.cache_read_usd_per_million).max(0.0)
        / 1_000_000.0;

    (round_usd(total_cost), round_usd(savings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_cost_and_cache_savings_without_double_counting_thinking() {
        let exact = ExactTokenTotals {
            input_tokens: 100_000,
            output_tokens: 20_000,
            thinking_tokens: 10_000,
            cache_read_tokens: 400_000,
            cache_write_tokens: 0,
            total_tokens: 530_000,
        };
        let (cost, savings) = estimate_exact_cost_usd("gemini-4-hydrogen-xhigh", &exact);
        // input: 0.1 * 0.50 = 0.05
        // output + thinking: 0.03 * 3.00 = 0.09
        // cache_read: 0.4 * 0.05 = 0.02
        // total = 0.16, savings = 0.4 * 0.45 = 0.18
        assert_eq!(cost, 0.16);
        assert_eq!(savings, 0.18);
    }
}
