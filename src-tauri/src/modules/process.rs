// Process 模块统一入口。
// 各分片按启动候选、路径解析、进程识别、关闭生命周期和平台启动职责组织，
// 通过 include! 保持原模块作用域，调用方无需改变。
include!("process_launch_candidates.rs");
include!("process_passive_wsl.rs");
include!("process_path_resolution.rs");
include!("process_codex_windows_launch.rs");
#[cfg(any(test, target_os = "windows"))]
#[path = "process_codex_package_launcher.rs"]
pub(crate) mod codex_package_launcher;
include!("process_codex_app_server.rs");
include!("process_detection_matching.rs");
#[cfg(any(test, target_os = "windows"))]
#[path = "process_codex_probe_cache.rs"]
mod codex_probe_cache;
include!("process_close_lifecycle.rs");
include!("process_codex_runtime.rs");
include!("process_codex_proxy_snapshot.rs");
include!("process_editor_launch.rs");

include!("process_tests.rs");
