use std::{fs, path::PathBuf};

use codex_task_supervisor::{
    classify_codex_event, CodexEventSource, CodexEvidenceConfidence, CodexEvidenceKind,
    CodexFailureClass,
};
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/upstream-contracts/fixtures")
        .join(name)
}

fn jsonl(name: &str) -> Vec<Value> {
    fs::read_to_string(fixture(name))
        .expect("read upstream contract fixture")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("valid JSONL fixture"))
        .collect()
}

#[test]
fn official_exec_contract_waits_for_terminal_and_allows_success_after_warning() {
    let evidence: Vec<_> = jsonl("openai-exec-json.jsonl")
        .iter()
        .map(|event| classify_codex_event(CodexEventSource::ExecJson, event))
        .collect();

    assert_eq!(evidence[2].kind, CodexEvidenceKind::QuotaWarning);
    assert_eq!(evidence[2].confidence, CodexEvidenceConfidence::Suspected);
    assert!(!evidence[2].terminal);
    assert_eq!(evidence[3].kind, CodexEvidenceKind::TurnCompleted);
    assert!(evidence[3].terminal);
    assert_eq!(evidence[4].kind, CodexEvidenceKind::TurnFailed);
    assert_eq!(
        evidence[4].failure_class,
        Some(CodexFailureClass::QuotaExhausted)
    );
    assert!(evidence[4].confirms_quota_exhaustion());
}

#[test]
fn official_app_server_contract_accepts_string_and_object_quota_variants() {
    let evidence: Vec<_> = jsonl("openai-app-server.jsonl")
        .iter()
        .map(|event| classify_codex_event(CodexEventSource::AppServer, event))
        .collect();

    for item in &evidence[0..2] {
        assert_eq!(item.kind, CodexEvidenceKind::TurnFailed);
        assert_eq!(item.failure_class, Some(CodexFailureClass::QuotaExhausted));
        assert!(item.confirms_quota_exhaustion());
    }
    assert_eq!(evidence[2].kind, CodexEvidenceKind::TurnInterrupted);
    assert_eq!(evidence[3].kind, CodexEvidenceKind::TurnCompleted);
}

#[test]
fn passive_observer_contracts_never_produce_authoritative_terminal_evidence() {
    let fixture: Value = serde_json::from_str(
        &fs::read_to_string(fixture("observer-boundaries.json")).expect("read observer fixture"),
    )
    .expect("valid observer fixture");
    for sample in fixture["sampleEvents"].as_array().expect("sample events") {
        let source = CodexEventSource::parse(sample["source"].as_str().expect("source"))
            .expect("known source");
        let evidence = classify_codex_event(source, &sample["payload"]);
        assert!(!evidence.terminal);
        assert_ne!(evidence.confidence, CodexEvidenceConfidence::Confirmed);
        assert!(!evidence.confirms_quota_exhaustion());
    }
}
