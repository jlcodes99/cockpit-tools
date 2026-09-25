// Compile the production modules directly, without Tauri, account services, or startup hooks.
#[path = "../../../../src-tauri/src/modules/codex_history_health.rs"]
pub mod history_health;
#[path = "../../../../src-tauri/src/modules/codex_rollout_byte_layout.rs"]
pub mod byte_layout;
