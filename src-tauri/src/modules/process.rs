// Process 模块统一入口。
// 各分片按启动候选、路径解析、进程识别、关闭生命周期和平台启动职责组织，
// 通过 include! 保持原模块作用域，调用方无需改变。
include!("process_launch_candidates.rs");
include!("process_path_resolution.rs");
include!("process_detection_matching.rs");
include!("process_close_lifecycle.rs");
include!("process_codex_runtime.rs");
include!("process_editor_launch.rs");

include!("process_tests.rs");
