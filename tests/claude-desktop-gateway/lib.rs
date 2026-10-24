//! Compile the actual configuration and atomic-write modules without a Tauri GUI.
#![allow(dead_code)]

pub mod modules {
    pub use crate::claude_desktop_gateway;
    // Only UI-specific error decoration and logging are replaced. All file I/O,
    // compare-and-replace, rollback and JSON merging use production code.
    pub mod windows_operation {
        pub fn format_permission_io_error(
            _operation: &str,
            _action: &str,
            _path: &str,
            _error: &std::io::Error,
        ) -> Option<String> {
            None
        }
    }
    pub mod logger {
        pub fn log_warn(_message: &str) {}
    }
    pub mod atomic_write {
        include!("../../src-tauri/src/modules/atomic_write.rs");
    }
}

#[path = "../../src-tauri/src/models/claude.rs"]
pub mod claude;

pub mod models {
    pub use crate::claude;
}

#[path = "../../src-tauri/src/modules/claude_desktop_gateway.rs"]
pub mod claude_desktop_gateway;

#[path = "../../src-tauri/src/modules/claude_desktop_gateway_config.rs"]
pub mod claude_desktop_gateway_config;
