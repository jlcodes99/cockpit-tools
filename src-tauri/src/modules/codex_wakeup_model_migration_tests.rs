use super::*;
use serde_json::{json, Value};

fn saved_task(model: Option<&str>) -> CodexWakeupTask {
    serde_json::from_value(json!({
        "id": "saved-task",
        "name": "My wakeup",
        "enabled": false,
        "accountIds": ["account-b", "account-a"],
        "prompt": "Keep my prompt",
        "model": model,
        "modelDisplayName": "My saved model",
        "modelReasoningEffort": "high",
        "schedule": {
            "kind": "weekly",
            "weeklyDays": [1, 3, 5],
            "weeklyTime": "11:30"
        },
        "createdAt": 123,
        "updatedAt": 456,
        "lastRunAt": 789,
        "lastStatus": "failed",
        "lastMessage": "Original failure",
        "lastSuccessCount": 2,
        "lastFailureCount": 1,
        "lastDurationMs": 1250,
        "nextRunAt": 12345,
        "executionMode": "confirm",
        "confirmTimeoutMinutes": 7
    }))
    .expect("deserialize historical wakeup task")
}

fn preset(id: &str, model: &str) -> CodexWakeupModelPreset {
    CodexWakeupModelPreset {
        id: id.to_string(),
        name: "User label".to_string(),
        model: model.to_string(),
        allowed_reasoning_efforts: vec!["high".to_string()],
        default_reasoning_effort: "high".to_string(),
    }
}

fn without_model_labels(task: &CodexWakeupTask) -> Value {
    let mut value = serde_json::to_value(task).expect("serialize wakeup task");
    value.as_object_mut().unwrap().remove("model");
    value.as_object_mut().unwrap().remove("modelDisplayName");
    value
}

#[test]
fn retired_model_migration_preserves_task_configuration_and_run_metadata() {
    for model in ["gpt-5.5", " GPT-5.5 ", "gpt-5.5-codex", "gpt-5.5-pro"] {
        let task = saved_task(Some(model));
        let expected_other_fields = without_model_labels(&task);
        let mut state = CodexWakeupState {
            tasks: vec![task],
            ..CodexWakeupState::default()
        };
        // The removal still applies when all migration markers have already been saved.
        assert!(retire_gpt_5_5_wakeup_model(&mut state));
        assert!(!state.enabled);
        assert_eq!(state.tasks[0].model.as_deref(), Some(DEFAULT_WAKEUP_MODEL));
        assert_eq!(
            state.tasks[0].model_display_name.as_deref(),
            Some(DEFAULT_WAKEUP_MODEL_NAME)
        );
        assert_eq!(without_model_labels(&state.tasks[0]), expected_other_fields);
        assert!(!retire_gpt_5_5_wakeup_model(&mut state));
    }
}

#[test]
fn retired_model_migration_keeps_custom_models_and_existing_newer_presets() {
    let mut state = CodexWakeupState {
        tasks: vec![saved_task(Some("vendor/gpt-5.5")), saved_task(None)],
        model_presets: vec![
            preset("preset-gpt-5-5", "custom-model"),
            preset("custom-provider", "vendor/gpt-5.5"),
            preset("custom-luna", DEFAULT_WAKEUP_MODEL),
            preset("old-builtin", "gpt-5.5"),
            preset("old-custom-id", " GPT-5.5-CODEX "),
        ],
        ..CodexWakeupState::default()
    };
    let expected_tasks = serde_json::to_value(&state.tasks).unwrap();
    let expected_presets = serde_json::to_value(&state.model_presets[..3]).unwrap();
    assert!(retire_gpt_5_5_wakeup_model(&mut state));
    assert_eq!(serde_json::to_value(&state.tasks).unwrap(), expected_tasks);
    assert_eq!(
        serde_json::to_value(&state.model_presets).unwrap(),
        expected_presets
    );
    assert!(!retire_gpt_5_5_wakeup_model(&mut state));
}

#[test]
fn retired_model_full_migration_is_idempotent_and_cleans_stale_saved_snapshots() {
    let mut state = CodexWakeupState {
        tasks: vec![saved_task(Some("gpt-5.5"))],
        model_presets: vec![preset("preset-gpt-5-5", "gpt-5.5")],
        model_preset_migrations: vec![],
        enabled: false,
    };
    assert!(apply_model_preset_migrations(&mut state));
    assert_eq!(state.tasks[0].model.as_deref(), Some(DEFAULT_WAKEUP_MODEL));
    assert!(state
        .model_presets
        .iter()
        .all(|item| !is_retired_gpt_5_5_wakeup_model(&item.model)));
    for marker in [
        GPT_5_5_MODEL_PRESET_MIGRATION_ID,
        RETIRE_GPT_5_5_WAKEUP_MODEL_MIGRATION_ID,
    ] {
        assert!(state
            .model_preset_migrations
            .iter()
            .any(|item| item == marker));
    }
    let migrated = serde_json::to_value(&state).unwrap();
    state = serde_json::from_value(migrated.clone()).expect("reload migrated saved state");
    assert!(!apply_model_preset_migrations(&mut state));
    assert_eq!(serde_json::to_value(&state).unwrap(), migrated);

    // A stale UI save must not reintroduce a retired preset or execution target.
    state
        .model_presets
        .push(preset("reintroduced-preset", "gpt-5.5"));
    state.tasks[0] = saved_task(Some("gpt-5.5"));
    assert!(apply_model_preset_migrations(&mut state));
    assert_eq!(serde_json::to_value(&state).unwrap(), migrated);
    assert!(!apply_model_preset_migrations(&mut state));
}

#[test]
fn retired_model_execution_fallback_preserves_reasoning_and_custom_targets() {
    for model in [Some("gpt-5.5"), Some("custom-model"), None] {
        let mut config = CodexWakeupExecutionConfig {
            model: model.map(str::to_string),
            model_display_name: Some("Original name".to_string()),
            model_reasoning_effort: Some("high".to_string()),
        };
        let changed =
            retarget_retired_wakeup_model(&mut config.model, &mut config.model_display_name);
        assert_eq!(changed, model == Some("gpt-5.5"));
        assert_eq!(config.model_reasoning_effort.as_deref(), Some("high"));
        if changed {
            assert_eq!(config.model.as_deref(), Some(DEFAULT_WAKEUP_MODEL));
            assert_eq!(
                config.model_display_name.as_deref(),
                Some(DEFAULT_WAKEUP_MODEL_NAME)
            );
        } else {
            assert_eq!(config.model.as_deref(), model);
            assert_eq!(config.model_display_name.as_deref(), Some("Original name"));
        }
    }
}
