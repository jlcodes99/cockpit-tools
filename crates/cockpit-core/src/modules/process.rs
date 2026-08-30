// cockpit-core Process 统一入口。
// 按发现、匹配、生命周期和编辑器启动职责拆分，保持原模块作用域不变。
include!("process_core_discovery.rs");
include!("process_core_matching.rs");
include!("process_core_lifecycle.rs");
include!("process_core_editor_launch.rs");
