use super::*;

const GOOGLE_MESSAGE: &str = "Invalid JSON payload received. Unknown name \"client_metadata\": Cannot find field.\nInvalid JSON payload received. Unknown name \"prompt_cache_key\": Cannot find field.\nInvalid JSON payload received. Unknown name \"verbosity\": Cannot find field.";

fn google_error(message: &str) -> Value {
    json!([{"error":{"code":400,"message":message,"status":"INVALID_ARGUMENT"}}])
}

#[test]
fn google_unknown_name_error_removes_every_named_optional_field_at_once() {
    let body = json!({
        "model": "m",
        "messages": [],
        "client_metadata": {"session_id": "s"},
        "prompt_cache_key": "k",
        "verbosity": "low",
        "reasoning_effort": "high"
    });
    for error in [
        google_error(GOOGLE_MESSAGE),
        json!({"error":{"code":400,"message":GOOGLE_MESSAGE}}),
    ] {
        let repair = repair(&body, &error).expect("repair");
        let mut repaired = body.clone();
        assert!(repair.apply(&mut repaired));
        assert_eq!(
            repaired,
            json!({"model":"m","messages":[],"reasoning_effort":"high"})
        );
    }
}

#[test]
fn google_unknown_name_error_never_removes_fields_that_change_the_request() {
    let body = json!({"model":"m","messages":[],"tools":[],"temperature":1,"verbosity":"low"});
    let message = "Invalid JSON payload received. Unknown name \"tools\": Cannot find field.\nInvalid JSON payload received. Unknown name \"temperature\": Cannot find field.";
    assert_eq!(repair(&body, &google_error(message)), None);
    // Only the optional hint is removed when it is named next to others.
    let message = format!(
        "{message}\nInvalid JSON payload received. Unknown name \"verbosity\": Cannot find field."
    );
    let repair = repair(&body, &google_error(&message)).expect("repair");
    let mut repaired = body.clone();
    assert!(repair.apply(&mut repaired));
    assert!(repaired.get("verbosity").is_none());
    assert!(repaired.get("tools").is_some() && repaired.get("temperature").is_some());
}

#[test]
fn google_unknown_name_error_ignores_nested_fields_and_fields_not_sent() {
    let body = json!({"model":"m","messages":[],"verbosity":"low"});
    let nested = "Invalid JSON payload received. Unknown name \"verbosity\" at 'generation_config': Cannot find field.";
    assert_eq!(repair(&body, &google_error(nested)), None);
    let absent =
        "Invalid JSON payload received. Unknown name \"prompt_cache_key\": Cannot find field.";
    assert_eq!(repair(&body, &google_error(absent)), None);
}

#[test]
fn removed_fields_are_not_removed_twice() {
    let body = json!({"model":"m","prompt_cache_key":"k"});
    let message =
        "Invalid JSON payload received. Unknown name \"prompt_cache_key\": Cannot find field.";
    let repair = repair(&body, &google_error(message)).expect("repair");
    let mut repaired = body.clone();
    assert!(repair.apply(&mut repaired));
    assert!(!repair.apply(&mut repaired));
}
