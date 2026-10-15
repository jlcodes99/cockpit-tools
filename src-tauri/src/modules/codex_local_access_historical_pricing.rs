// Historical log estimates only. These entries are never offered as model presets.
const HISTORICAL_CODEX_MODEL_PRICE_BOOK: &[CodexLocalAccessPriceBookEntry] = &[
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4",
        session_long_context: true,
        standard: codex_price(2.5, 0.25, 15.0),
        priority: Some(codex_price(5.0, 0.5, 30.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4-mini",
        session_long_context: false,
        standard: codex_price(0.75, 0.075, 4.5),
        priority: Some(codex_price(1.5, 0.15, 9.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.4-nano",
        session_long_context: false,
        standard: codex_price(0.2, 0.02, 1.25),
        priority: None,
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.3-codex",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.3-codex-spark",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.2",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.2-codex",
        session_long_context: false,
        standard: codex_price(1.75, 0.175, 14.0),
        priority: Some(codex_price(3.5, 0.35, 28.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        priority: Some(codex_price(2.5, 0.25, 20.0)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex-max",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        // No explicit priority rates -> fall back to x2 at billing time.
        priority: None,
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5.1-codex-mini",
        session_long_context: false,
        standard: codex_price(0.25, 0.025, 2.0),
        priority: Some(codex_price(0.45, 0.045, 3.6)),
    },
    CodexLocalAccessPriceBookEntry {
        model_id: "gpt-5-codex",
        session_long_context: false,
        standard: codex_price(1.25, 0.125, 10.0),
        priority: None,
    },
];
