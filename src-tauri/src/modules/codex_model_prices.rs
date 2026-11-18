use serde::Deserialize;
use std::{collections::HashSet, sync::{Arc, OnceLock}};

pub(super) const BUNDLED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"), "/../resources/codex/model_prices.json"
));

/// Global OpenAI API text prices. Token triples are input, cache read and output.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PriceBook {
    schema_version: u32,
    pub revision: u64,
    source: String,
    pub models: Vec<ModelPrice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelPrice {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub standard: [f64; 3],
    #[serde(default)]
    pub subsequent_prices: Vec<DatedPrice>,
    pub ultrafast_promo_until_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DatedPrice {
    effective_at_ms: i64,
    standard: [f64; 3],
}

impl ModelPrice {
    pub(crate) fn standard_at(&self, timestamp_ms: i64) -> [f64; 3] {
        self.subsequent_prices.iter().rev()
            .find(|price| price.effective_at_ms <= timestamp_ms)
            .map_or(self.standard, |price| price.standard)
    }
}

impl PriceBook {
    pub(crate) fn find(&self, id: &str) -> Option<&ModelPrice> {
        self.models.iter().find(|price| {
            price.id == id || price.aliases.iter().any(|alias| alias == id)
        })
    }
}

pub(super) fn bundled() -> Arc<PriceBook> {
    static BOOK: OnceLock<Arc<PriceBook>> = OnceLock::new();
    BOOK.get_or_init(|| Arc::new(parse(BUNDLED).expect("bundled OpenAI API prices must be valid"))).clone()
}

pub(super) fn parse(bytes: &[u8]) -> Result<PriceBook, String> {
    fn valid_prices(prices: [f64; 3]) -> bool {
        prices.into_iter().all(|value| value.is_finite() && value >= 0.0)
    }
    let book: PriceBook = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if book.schema_version != 1 || book.revision == 0
        || !book.source.starts_with("https://developers.openai.com/api/docs/")
        || book.models.is_empty()
    {
        return Err("Invalid OpenAI price book metadata".into());
    }
    let mut ids = HashSet::new();
    for model in &book.models {
        for id in std::iter::once(&model.id).chain(&model.aliases) {
            if !id.starts_with("gpt-")
                || !id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-'))
                || !ids.insert(id)
            {
                return Err(format!("Invalid or duplicate pricing model {id}"));
            }
        }
        if !valid_prices(model.standard)
            || model.ultrafast_promo_until_ms.is_some_and(|date| date <= 0)
            || model.subsequent_prices.iter().any(|price| {
                price.effective_at_ms <= 0 || !valid_prices(price.standard)
            })
            || model.subsequent_prices.windows(2).any(|p| p[0].effective_at_ms >= p[1].effective_at_ms)
        {
            return Err(format!("Invalid prices for {}", model.id));
        }
    }
    Ok(book)
}

pub(super) fn parse_remote(bytes: &[u8]) -> Result<PriceBook, String> {
    let book = parse(bytes)?;
    let bundled = bundled();
    if book.revision > bundled.revision && bundled.models.iter().any(|expected| {
        !book.models.iter().any(|entry| entry.id == expected.id
            && expected.aliases.iter().all(|alias| entry.aliases.contains(alias)))
    }) {
        return Err("OpenAI price book is missing a bundled model or alias".into());
    }
    Ok(book)
}
