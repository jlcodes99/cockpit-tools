// Exercise the actual packaging guard, without copying its implementation or
// compiling the application's platform-specific GUI dependencies.
#[cfg(test)]
#[path = "../../../../src-tauri/build_frontend.rs"]
mod build_frontend;
