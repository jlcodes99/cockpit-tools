use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value as JsonValue};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "macos")]
const CODEX_APP_SERVER_MACOS_EXECUTABLES: &[&str] = &[
    "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
    "/Applications/ChatGPT.app/Contents/Resources/codex",
    "/Applications/Codex.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
    "/Applications/Codex.app/Contents/Resources/codex",
];
const CODEX_APP_SERVER_EXECUTABLE_ENV: &str = "CODEX_APP_SERVER_EXECUTABLE";
const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

pub fn rebuild_thread_metadata(codex_home: &Path) -> Result<(), String> {
    rebuild_imported_thread_metadata(codex_home, &[])
}

pub fn rebuild_imported_thread_metadata(
    codex_home: &Path,
    mapped_threads: &[(String, String)],
) -> Result<(), String> {
    let flow_started = Instant::now();
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] rebuild_thread_metadata flow started: codex_home={}",
        codex_home.display()
    ));
    let sanitize_started = Instant::now();
    crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &codex_home.join("config.toml"),
    )?;
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] sanitize config finished: codex_home={}, elapsed_ms={}, total_ms={}",
        codex_home.display(),
        sanitize_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let executable_started = Instant::now();
    let executable = official_app_server_executable()?;
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] executable resolved: executable={}, elapsed_ms={}, total_ms={}",
        executable.display(),
        executable_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] starting rebuild_thread_metadata: executable={}, codex_home={}",
        executable.display(),
        codex_home.display()
    ));
    let spawn_started = Instant::now();
    let mut child = build_app_server_command(&executable, codex_home)
        .spawn()
        .map_err(|error| {
            format!(
                "启动官方 Codex app-server 失败 ({} / CODEX_HOME={}): {}",
                executable.display(),
                codex_home.display(),
                error
            )
        })?;
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] child spawned: codex_home={}, pid={:?}, elapsed_ms={}, total_ms={}",
        codex_home.display(),
        child.id(),
        spawn_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let stdout = child
        .stdout
        .take()
        .ok_or("无法读取官方 app-server stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("无法读取官方 app-server stderr")?;
    let mut stdin = child.stdin.take().ok_or("无法写入官方 app-server stdin")?;
    let (sender, receiver) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Official AppServer][stderr] {}",
                line
            ));
        }
    });

    let result = (|| {
        let initialize_started = Instant::now();
        send_request(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "cockpit-tools",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": if mapped_threads.is_empty() {
                        JsonValue::Null
                    } else {
                        json!({ "experimentalApi": true })
                    },
                },
            }),
        )?;
        wait_for_response(&receiver, 1)?;
        crate::modules::logger::log_info(&format!(
            "[Codex Official AppServer] initialize finished: codex_home={}, elapsed_ms={}, total_ms={}",
            codex_home.display(),
            initialize_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));

        let thread_list_started = Instant::now();
        send_request(
            &mut stdin,
            json!({
                "method": "thread/list",
                "id": 2,
                "params": {
                    "cursor": null,
                    "limit": 1,
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                    "modelProviders": null,
                    "sourceKinds": [],
                    "archived": false,
                },
            }),
        )?;
        wait_for_response(&receiver, 2)?;
        if !mapped_threads.is_empty() {
            assign_imported_threads_to_projects(&mut stdin, &receiver, mapped_threads)?;
        }
        crate::modules::logger::log_info(&format!(
            "[Codex Official AppServer] thread/list finished: codex_home={}, elapsed_ms={}, total_ms={}",
            codex_home.display(),
            thread_list_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        Ok::<(), String>(())
    })();

    let finish_started = Instant::now();
    finish_child(&mut child);
    let _ = reader.join();
    let _ = stderr_reader.join();
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] child finished: codex_home={}, elapsed_ms={}, total_ms={}",
        codex_home.display(),
        finish_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let result = result.and_then(|()| {
        let normalized_count =
            crate::modules::codex_session_visibility::normalize_official_thread_cwds(codex_home)?;
        if normalized_count > 0 {
            crate::modules::logger::log_info(&format!(
                "[Codex Official AppServer] normalized {} Desktop thread cwd row(s): codex_home={}",
                normalized_count,
                codex_home.display()
            ));
        }
        Ok(())
    });
    if let Err(error) = &result {
        crate::modules::logger::log_warn(&format!(
            "[Codex Official AppServer] rebuild_thread_metadata failed: codex_home={}, elapsed_ms={}, error={}",
            codex_home.display(),
            flow_started.elapsed().as_millis(),
            error
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[Codex Official AppServer] rebuild_thread_metadata completed: codex_home={}, elapsed_ms={}",
            codex_home.display(),
            flow_started.elapsed().as_millis()
        ));
    }
    result
}

fn assign_imported_threads_to_projects(
    stdin: &mut impl Write,
    receiver: &mpsc::Receiver<String>,
    mapped_threads: &[(String, String)],
) -> Result<(), String> {
    let mut request_id = 3;
    let mut cursor = JsonValue::Null;
    let mut projects = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen_cursors = std::collections::HashSet::new();
    loop {
        if Instant::now() >= deadline {
            return Err("更新项目归属超时，已导入的会话将保留".into());
        }
        send_request(
            stdin,
            json!({
                "method": "project/list", "id": request_id,
                "params": { "cursor": cursor, "limit": 100 }
            }),
        )?;
        let response = match wait_for_response_value_until(receiver, request_id, deadline) {
            Ok(response) => response,
            Err(error) => {
                let message = error.message();
                // Older Codex versions group by cwd and do not expose project identities.
                if message.contains("-32601") || message.contains("unknown variant `project/list`")
                {
                    return Ok(());
                }
                return Err(message.to_string());
            }
        };
        let result = response
            .get("result")
            .ok_or("project/list 响应缺少 result")?;
        projects.extend(
            result
                .get("data")
                .and_then(JsonValue::as_array)
                .ok_or("project/list 响应缺少项目列表")?
                .iter()
                .cloned(),
        );
        request_id += 1;
        cursor = result.get("nextCursor").cloned().unwrap_or(JsonValue::Null);
        if cursor.is_null() {
            break;
        }
        if !seen_cursors.insert(cursor.to_string()) || projects.len() >= 10_000 {
            return Err("官方项目列表分页异常，已导入的会话将保留".into());
        }
    }
    let mut warnings = Vec::new();
    for (thread_id, cwd) in mapped_threads {
        if Instant::now() >= deadline {
            return Err("更新项目归属超时，已导入的会话将保留".into());
        }
        let project_id =
            match crate::modules::codex_session_import_paths::project_id_for_cwd(&projects, cwd) {
                Ok(Some(project_id)) => project_id,
                Ok(None) => {
                    warnings.push(format!(
                        "未找到目标目录对应的 Codex 项目，请先在 Codex 中添加该项目: {}",
                        cwd
                    ));
                    continue;
                }
                Err(error) => {
                    warnings.push(error);
                    continue;
                }
            };
        send_request(
            stdin,
            json!({
                "method": "thread/metadata/update", "id": request_id,
                "params": { "threadId": thread_id, "projectId": project_id }
            }),
        )?;
        if let Err(error) = wait_for_response_value_until(receiver, request_id, deadline) {
            if error.is_timeout() {
                return Err(error.message().to_string());
            }
            warnings.push(error.message().to_string());
        }
        request_id += 1;
    }
    warnings.sort();
    warnings.dedup();
    if warnings.is_empty() {
        Ok(())
    } else {
        Err(warnings.join("；"))
    }
}

/// 通过官方 app-server 的 `thread/delete` 删除会话线程（与官方客户端一致），
/// 返回成功删除的条数。官方删除会同时清理 state DB、会话目录与 rollout 文件，
/// 因此客户端不需要重启或重新扫描即可同步。
///
/// 单个会话删除失败只记录日志并继续，调用方可根据返回条数决定是否回退到
/// 文件方式删除；只有 app-server 无法启动这类整体性错误才返回 `Err`。
pub fn delete_threads(codex_home: &Path, session_ids: &[String]) -> Result<usize, String> {
    let unique_session_ids = dedupe_session_ids(session_ids);
    if unique_session_ids.is_empty() {
        return Ok(0);
    }

    let flow_started = Instant::now();
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] delete_threads flow started: codex_home={}, requested={}",
        codex_home.display(),
        unique_session_ids.len()
    ));
    if let Err(error) = crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &codex_home.join("config.toml"),
    ) {
        crate::modules::logger::log_warn(&format!(
            "[Codex Official AppServer] sanitize config before delete_threads failed, continuing: codex_home={}, error={}",
            codex_home.display(),
            error
        ));
    }
    let executable = official_app_server_executable()?;
    let mut child = build_app_server_command(&executable, codex_home)
        .spawn()
        .map_err(|error| {
            format!(
                "启动官方 Codex app-server 失败 ({} / CODEX_HOME={}): {}",
                executable.display(),
                codex_home.display(),
                error
            )
        })?;
    crate::modules::logger::log_info(&format!(
        "[Codex Official AppServer] delete_threads child spawned: codex_home={}, pid={:?}, requested={}",
        codex_home.display(),
        child.id(),
        unique_session_ids.len()
    ));

    let stdout = child
        .stdout
        .take()
        .ok_or("无法读取官方 app-server stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("无法读取官方 app-server stderr")?;
    let mut stdin = child.stdin.take().ok_or("无法写入官方 app-server stdin")?;
    let (sender, receiver) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Official AppServer][stderr] {}",
                line
            ));
        }
    });

    let result = (|| {
        send_request(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "cockpit-tools",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": null,
                },
            }),
        )?;
        wait_for_response(&receiver, 1)?;

        let mut deleted_count = 0usize;
        for (index, session_id) in unique_session_ids.iter().enumerate() {
            let request_id = 2 + index as i64;
            if let Err(error) = send_request(
                &mut stdin,
                json!({
                    "method": "thread/delete",
                    "id": request_id,
                    "params": { "threadId": session_id },
                }),
            ) {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Official AppServer] delete_threads write failed: codex_home={}, thread_id={}, error={}",
                    codex_home.display(),
                    session_id,
                    error
                ));
                break;
            }
            match wait_for_response_value(&receiver, request_id) {
                Ok(_) => deleted_count += 1,
                Err(error) if error.is_timeout() => {
                    // app-server 已无响应：继续逐条等待会让批量删除长时间卡住，
                    // 剩余会话交给文件方式删除兜底。
                    crate::modules::logger::log_warn(&format!(
                        "[Codex Official AppServer] delete_threads 超时，停止继续删除并回退: codex_home={}, remaining={}, error={}",
                        codex_home.display(),
                        unique_session_ids.len() - index,
                        error.message()
                    ));
                    break;
                }
                Err(error) => {
                    crate::modules::logger::log_warn(&format!(
                        "[Codex Official AppServer] delete_threads failed for thread: codex_home={}, thread_id={}, error={}",
                        codex_home.display(),
                        session_id,
                        error.message()
                    ));
                }
            }
        }
        Ok::<usize, String>(deleted_count)
    })();

    finish_child(&mut child);
    let _ = reader.join();
    let _ = stderr_reader.join();
    match &result {
        Ok(deleted_count) => crate::modules::logger::log_info(&format!(
            "[Codex Official AppServer] delete_threads completed: codex_home={}, requested={}, deleted={}, elapsed_ms={}",
            codex_home.display(),
            unique_session_ids.len(),
            deleted_count,
            flow_started.elapsed().as_millis()
        )),
        Err(error) => crate::modules::logger::log_warn(&format!(
            "[Codex Official AppServer] delete_threads failed: codex_home={}, requested={}, elapsed_ms={}, error={}",
            codex_home.display(),
            unique_session_ids.len(),
            flow_started.elapsed().as_millis(),
            error
        )),
    }
    result
}

fn dedupe_session_ids(session_ids: &[String]) -> Vec<String> {
    let mut unique = Vec::new();
    for session_id in session_ids {
        let trimmed = session_id.trim();
        if trimmed.is_empty() || unique.iter().any(|existing| existing == trimmed) {
            continue;
        }
        unique.push(trimmed.to_string());
    }
    unique
}

pub(crate) fn official_app_server_executable() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(executable) = std::env::var_os(CODEX_APP_SERVER_EXECUTABLE_ENV) {
        if !executable.as_os_str().is_empty() {
            push_candidate(&mut candidates, PathBuf::from(executable));
        }
    }
    add_codex_app_server_candidates(&mut candidates);

    for executable in &candidates {
        if executable.exists() {
            return Ok(executable.clone());
        }
    }

    let searched_paths = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let error = format!("未找到官方 Codex app-server 可执行文件: {}", searched_paths);
    crate::modules::logger::log_warn(&format!("[Codex Official AppServer] {}", error));
    Err(error)
}

fn add_codex_app_server_candidates(candidates: &mut Vec<PathBuf>) {
    let configured_path = crate::modules::config::get_user_config().codex_app_path;
    if !configured_path.trim().is_empty() {
        push_candidate_from_codex_launch_path(candidates, Path::new(configured_path.trim()));
    }

    if let Some(detected_path) = crate::modules::process::detect_codex_exec_path() {
        push_candidate_from_codex_launch_path(candidates, &detected_path);
    }

    #[cfg(target_os = "macos")]
    for executable in CODEX_APP_SERVER_MACOS_EXECUTABLES {
        push_candidate(candidates, PathBuf::from(executable));
    }
}

fn push_candidate_from_codex_launch_path(candidates: &mut Vec<PathBuf>, launch_path: &Path) {
    if let Some(app_server_path) = app_server_executable_from_codex_launch_path(launch_path) {
        // Current macOS bundles contain a signed CLI helper. Preserve the old
        // executable as a fallback for installations that have not upgraded.
        if parent_file_name_eq(&app_server_path, "resources")
            && path_file_name_eq(&app_server_path, "codex")
            && app_server_path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .is_some_and(|root| {
                    path_file_name_eq(root, "chatgpt.app") || path_file_name_eq(root, "codex.app")
                })
        {
            push_candidate(
                candidates,
                app_server_path
                    .parent()
                    .unwrap()
                    .join("codex-cli/CodexCLI.app/Contents/MacOS/codex"),
            );
        }
        push_candidate(candidates, app_server_path);
    }
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() || candidates.iter().any(|candidate| candidate == &path) {
        return;
    }
    candidates.push(path);
}

fn app_server_executable_from_codex_launch_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }

    if is_existing_app_server_path_shape(path) {
        return Some(path.to_path_buf());
    }

    if path_file_name_eq(path, "codex.app") {
        return Some(path.join("Contents").join("Resources").join("codex"));
    }

    if path_file_name_eq(path, "chatgpt.app") {
        return Some(path.join("Contents").join("Resources").join("codex"));
    }

    if path_file_name_eq(path, "codex") && parent_file_name_eq(path, "macos") {
        let contents_dir = path.parent()?.parent()?;
        return Some(contents_dir.join("Resources").join("codex"));
    }

    if path_file_name_eq(path, "chatgpt") && parent_file_name_eq(path, "macos") {
        let contents_dir = path.parent()?.parent()?;
        return Some(contents_dir.join("Resources").join("codex"));
    }

    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if path_file_name_eq(&resolved, "chatgpt") && parent_file_name_eq(&resolved, "chatgpt") {
        return Some(resolved.parent()?.join("resources").join("codex"));
    }

    if path_file_name_eq(path, "codex.exe") {
        return Some(path.parent()?.join("resources").join("codex.exe"));
    }

    if path_file_name_eq(path, "chatgpt.exe") {
        return Some(path.parent()?.join("resources").join("codex.exe"));
    }

    None
}

fn is_existing_app_server_path_shape(path: &Path) -> bool {
    if path_file_name_eq(path, "codex") && parent_file_name_eq(path, "macos") {
        if path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .is_some_and(|root| path_file_name_eq(root, "codexcli.app"))
        {
            return true;
        }
    }
    if path_file_name_eq(path, "codex") && parent_file_name_eq(path, "resources") {
        return true;
    }
    path_file_name_eq(path, "codex.exe") && parent_file_name_eq(path, "resources")
}

fn path_file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn parent_file_name_eq(path: &Path, expected: &str) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn build_app_server_command(executable: &Path, codex_home: &Path) -> Command {
    let mut command = Command::new(executable);
    crate::modules::process::apply_managed_proxy_env_to_command(&mut command);
    command
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
}

fn send_request(stdin: &mut impl Write, request: JsonValue) -> Result<(), String> {
    let line = serde_json::to_string(&request)
        .map_err(|error| format!("序列化官方 app-server 请求失败: {}", error))?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("写入官方 app-server 请求失败: {}", error))
}

/// 官方 app-server 响应等待失败：区分「整体无响应」和「单条请求返回错误」。
///
/// 批量删除需要据此决定是否继续发送后续请求：app-server 超时说明它已经不可用，
/// 继续逐条等待只会让整个删除流程长时间卡住，此时应立刻回退到文件方式删除。
enum AppServerWaitFailure {
    Timeout(String),
    Response(String),
}

impl AppServerWaitFailure {
    fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout(_))
    }

    fn message(&self) -> &str {
        match self {
            Self::Timeout(message) | Self::Response(message) => message,
        }
    }
}

fn wait_for_response(receiver: &mpsc::Receiver<String>, request_id: i64) -> Result<(), String> {
    wait_for_response_value(receiver, request_id)
        .map(|_| ())
        .map_err(|error| error.message().to_string())
}

fn wait_for_response_value(
    receiver: &mpsc::Receiver<String>,
    request_id: i64,
) -> Result<JsonValue, AppServerWaitFailure> {
    wait_for_response_value_until(
        receiver,
        request_id,
        Instant::now() + APP_SERVER_RESPONSE_TIMEOUT,
    )
}

fn wait_for_response_value_until(
    receiver: &mpsc::Receiver<String>,
    request_id: i64,
    deadline: Instant,
) -> Result<JsonValue, AppServerWaitFailure> {
    let deadline = deadline.min(Instant::now() + APP_SERVER_RESPONSE_TIMEOUT);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppServerWaitFailure::Timeout(format!(
                "等待官方 app-server 响应超时 (id={})",
                request_id
            )));
        }
        let line = receiver.recv_timeout(remaining).map_err(|_| {
            AppServerWaitFailure::Timeout(format!(
                "等待官方 app-server 响应超时 (id={})",
                request_id
            ))
        })?;
        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        if value.get("id").and_then(JsonValue::as_i64) != Some(request_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            crate::modules::logger::log_warn(&format!(
                "[Codex Official AppServer] response error: id={}, error={}",
                request_id, error
            ));
            return Err(AppServerWaitFailure::Response(format!(
                "官方 app-server 返回错误 (id={}): {}",
                request_id, error
            )));
        }
        if value.get("result").is_some() {
            return Ok(value);
        }
        return Err(AppServerWaitFailure::Response(format!(
            "官方 app-server 响应缺少 result (id={}): {}",
            request_id, value
        )));
    }
}

fn finish_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_project_binding_rejects_repeated_pagination_cursor() {
        let (sender, receiver) = mpsc::channel();
        for id in [3, 4] {
            sender
                .send(json!({"id":id,"result":{"data":[],"nextCursor":"repeat"}}).to_string())
                .unwrap();
        }
        let mut stdin = Vec::new();
        assert!(assign_imported_threads_to_projects(
            &mut stdin,
            &receiver,
            &[("thread".into(), "/new/project".into())]
        )
        .unwrap_err()
        .contains("分页异常"));
        assert_eq!(String::from_utf8(stdin).unwrap().lines().count(), 2);
    }

    #[test]
    fn unrelated_notifications_do_not_extend_the_response_deadline() {
        let (sender, receiver) = mpsc::channel();
        for _ in 0..100 {
            sender
                .send(json!({"method":"notification"}).to_string())
                .unwrap();
        }
        let error =
            wait_for_response_value_until(&receiver, 3, Instant::now() + Duration::from_millis(10))
                .err()
                .unwrap();
        assert!(error.is_timeout());
    }

    #[test]
    fn imported_project_binding_paginates_and_updates_matching_project() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(json!({"id":3,"result":{"data":[],"nextCursor":"page-2"}}).to_string())
            .unwrap();
        sender.send(json!({"id":4,"result":{"data":[{"id":"destination","roots":[{"path":"/b/project"}]}],"nextCursor":null}}).to_string()).unwrap();
        sender
            .send(json!({"id":5,"result":{}}).to_string())
            .unwrap();
        let mut stdin = Vec::new();
        assign_imported_threads_to_projects(
            &mut stdin,
            &receiver,
            &[("thread-1".into(), "/b/project".into())],
        )
        .unwrap();
        let requests = String::from_utf8(stdin)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<JsonValue>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(requests[1]["params"]["cursor"], "page-2");
        assert_eq!(requests[2]["method"], "thread/metadata/update");
        assert_eq!(requests[2]["params"]["projectId"], "destination");
        assert_eq!(requests[2]["params"]["threadId"], "thread-1");
    }

    #[test]
    fn older_servers_without_project_api_keep_cwd_fallback() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(
                json!({"id":3,"error":{"code":-32600,"message":"unknown variant `project/list`"}})
                    .to_string(),
            )
            .unwrap();
        let mut stdin = Vec::new();
        assign_imported_threads_to_projects(
            &mut stdin,
            &receiver,
            &[("thread-1".into(), "/b/project".into())],
        )
        .unwrap();
        assert_eq!(String::from_utf8(stdin).unwrap().lines().count(), 1);
    }

    #[test]
    fn unmatched_project_returns_warning_without_assigning_arbitrary_project() {
        let (sender, receiver) = mpsc::channel();
        sender.send(json!({"id":3,"result":{"data":[{"id":"other","roots":[{"path":"/other"}]}],"nextCursor":null}}).to_string()).unwrap();
        let mut stdin = Vec::new();
        let result = assign_imported_threads_to_projects(
            &mut stdin,
            &receiver,
            &[("thread-1".into(), "/b/project".into())],
        );
        assert!(result
            .unwrap_err()
            .contains("未找到目标目录对应的 Codex 项目"));
        assert_eq!(String::from_utf8(stdin).unwrap().lines().count(), 1);
    }

    #[test]
    fn maps_macos_launch_binary_to_resources_app_server() {
        let launch_path = PathBuf::from("/Applications/Codex.app/Contents/MacOS/Codex");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn prioritizes_nested_macos_cli_and_preserves_legacy_fallback() {
        for root in ["/Applications/ChatGPT.app", "/Volumes/Apps Disk/Codex.app"] {
            for launch in [
                PathBuf::from(root),
                PathBuf::from(root).join("Contents/MacOS/ChatGPT"),
            ] {
                let mut candidates = Vec::new();
                push_candidate_from_codex_launch_path(&mut candidates, &launch);
                assert_eq!(
                    candidates,
                    vec![
                        PathBuf::from(root)
                            .join("Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex"),
                        PathBuf::from(root).join("Contents/Resources/codex"),
                    ]
                );
            }
        }
    }

    #[test]
    fn preserves_direct_nested_macos_cli_executable() {
        let path = PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex");
        let mut candidates = Vec::new();
        push_candidate_from_codex_launch_path(&mut candidates, &path);
        assert_eq!(candidates, vec![path]);
    }

    #[test]
    fn maps_chatgpt_macos_launch_binary_to_resources_app_server() {
        let launch_path = PathBuf::from("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn maps_macos_app_root_to_resources_app_server() {
        let launch_path = PathBuf::from("/Applications/Codex.app");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from("/Applications/Codex.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn maps_chatgpt_macos_app_root_to_resources_app_server() {
        let launch_path = PathBuf::from("/Applications/ChatGPT.app");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex")
        );
    }

    #[test]
    fn maps_windows_launch_binary_to_resources_app_server() {
        let launch_path =
            PathBuf::from("C:/Program Files/WindowsApps/OpenAI.Codex_1.2.3/app/Codex.exe");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from(
                "C:/Program Files/WindowsApps/OpenAI.Codex_1.2.3/app/resources/codex.exe"
            )
        );
    }

    #[test]
    fn maps_chatgpt_windows_launch_binary_to_resources_app_server() {
        let launch_path =
            PathBuf::from("C:/Program Files/WindowsApps/OpenAI.ChatGPT_1.2.3/app/ChatGPT.exe");
        let app_server_path = app_server_executable_from_codex_launch_path(&launch_path)
            .expect("resolve app-server path");

        assert_eq!(
            app_server_path,
            PathBuf::from(
                "C:/Program Files/WindowsApps/OpenAI.ChatGPT_1.2.3/app/resources/codex.exe"
            )
        );
    }

    #[test]
    fn keeps_existing_resources_app_server_path() {
        let app_server_path = PathBuf::from(
            "C:/Program Files/WindowsApps/OpenAI.Codex_1.2.3/app/resources/codex.exe",
        );

        assert_eq!(
            app_server_executable_from_codex_launch_path(&app_server_path),
            Some(app_server_path)
        );
    }

    #[test]
    fn maps_linux_chatgpt_binary_to_resources_app_server() {
        let launch_path = PathBuf::from("/usr/lib/chatgpt/ChatGPT");
        assert_eq!(
            app_server_executable_from_codex_launch_path(&launch_path),
            Some(PathBuf::from("/usr/lib/chatgpt/resources/codex"))
        );
    }
}
