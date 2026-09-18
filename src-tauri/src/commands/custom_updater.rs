use crate::modules::logger;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Emitter};

const CUSTOM_BRANCH: &str = "my-custom";
const CUSTOM_MERGE_MESSAGE: &str = "merge: sync upstream main into my-custom";

#[cfg(windows)]
const NPM_PROGRAM: &str = "npm.cmd";
#[cfg(not(windows))]
const NPM_PROGRAM: &str = "npm";

struct SyncStep {
    program: &'static str,
    args: &'static [&'static str],
    title: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSyncStepProgress {
    pub step: usize,
    pub total: usize,
    pub title: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomSyncReport {
    pub success: bool,
    pub message: String,
    pub current_step: usize,
    pub total_steps: usize,
    pub logs: String,
}

fn resolve_repo_path(repo_path: Option<String>) -> Result<PathBuf, String> {
    if let Some(ref p) = repo_path {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.join(".git").exists() {
                return Ok(path);
            }
        }
    }

    // Đường dẫn mặc định của dự án
    let default_path = PathBuf::from(r"D:\0-vung-apps\cockpit-tools");
    if default_path.join(".git").exists() {
        return Ok(default_path);
    }

    // Kiểm tra thư mục làm việc hiện tại
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(".git").exists() {
            return Ok(cwd);
        }
        if let Some(parent) = cwd.parent() {
            if parent.join(".git").exists() {
                return Ok(parent.to_path_buf());
            }
        }
    }

    Err("Không tìm thấy thư mục git repo của Cockpit Tools. Vui lòng kiểm tra đường dẫn dự án.".to_string())
}

fn command_label(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .map(|part| {
            if part.contains(char::is_whitespace) {
                format!("\"{}\"", part.replace('"', "\\\""))
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn run_command(repo_dir: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let label = command_label(program, args);
    let mut command = Command::new(program);
    command.current_dir(repo_dir).args(args);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    match command.output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let combined = format!("{}\n{}", stdout.trim(), stderr.trim())
                .trim()
                .to_string();

            if out.status.success() {
                Ok(combined)
            } else {
                Err(format!(
                    "Lệnh '{}' thất bại (mã thoát: {:?}):\n{}",
                    label,
                    out.status.code(),
                    combined
                ))
            }
        }
        Err(err) => Err(format!("Không thể khởi chạy '{}': {}", label, err)),
    }
}

fn run_git(repo_dir: &Path, args: &[&str]) -> Result<String, String> {
    run_command(repo_dir, "git", args)
}

fn merge_in_progress(repo_dir: &Path) -> bool {
    run_git(repo_dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

fn git_path(repo_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let raw = run_git(repo_dir, &["rev-parse", "--git-path", name])?;
    let path = PathBuf::from(raw.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repo_dir.join(path)
    })
}

fn is_custom_updater_merge_message(message: &str) -> bool {
    message.lines().next().map(str::trim) == Some(CUSTOM_MERGE_MESSAGE)
}

fn recover_interrupted_custom_merge(repo_dir: &Path) -> Result<Option<String>, String> {
    if !merge_in_progress(repo_dir) {
        return Ok(None);
    }

    let merge_message_path = git_path(repo_dir, "MERGE_MSG")?;
    let merge_message = std::fs::read_to_string(&merge_message_path).map_err(|err| {
        format!(
            "Không thể đọc trạng thái merge tại '{}': {}",
            merge_message_path.display(),
            err
        )
    })?;

    if !is_custom_updater_merge_message(&merge_message) {
        return Err(
            "Repo đang có một merge thủ công chưa hoàn tất. Hãy hoàn tất hoặc hủy merge đó trước khi cập nhật."
                .to_string(),
        );
    }

    let output = run_git(repo_dir, &["merge", "--abort"])?;
    Ok(Some(format!(
        "Đã khôi phục merge dang dở từ lần cập nhật trước.{}",
        if output.is_empty() {
            String::new()
        } else {
            format!("\n{}", output)
        }
    )))
}

fn validate_custom_branch(repo_dir: &Path) -> Result<(), String> {
    let branch = run_git(repo_dir, &["branch", "--show-current"])?;
    if branch.trim() != CUSTOM_BRANCH {
        return Err(format!(
            "Repo phải đang ở nhánh '{}', nhưng hiện tại là '{}'.",
            CUSTOM_BRANCH,
            if branch.trim().is_empty() {
                "detached HEAD"
            } else {
                branch.trim()
            }
        ));
    }

    run_git(repo_dir, &["remote", "get-url", "upstream"])
        .map_err(|err| format!("Remote 'upstream' chưa sẵn sàng.\n{}", err))?;
    run_git(repo_dir, &["remote", "get-url", "origin"])
        .map_err(|err| format!("Remote 'origin' chưa sẵn sàng.\n{}", err))?;
    Ok(())
}

fn sync_steps() -> [SyncStep; 5] {
    [
        SyncStep {
            program: "git",
            args: &["fetch", "upstream", "main"],
            title: "1/5: Lấy cập nhật mới từ repo gốc (upstream)",
        },
        SyncStep {
            program: "git",
            args: &["push", "origin", "upstream/main:refs/heads/main"],
            title: "2/5: Đồng bộ nhánh main lên fork GitHub",
        },
        SyncStep {
            program: "git",
            args: &[
                "merge",
                "-X",
                "ours",
                "upstream/main",
                "-m",
                CUSTOM_MERGE_MESSAGE,
            ],
            title: "3/5: Gộp cập nhật main vào nhánh my-custom",
        },
        SyncStep {
            program: NPM_PROGRAM,
            args: &["test"],
            title: "4/5: Chạy bộ kiểm thử regression (npm test)",
        },
        SyncStep {
            program: "git",
            args: &["push", "origin", "HEAD:refs/heads/my-custom"],
            title: "5/5: Đẩy lên GitHub để kích hoạt build matrix",
        },
    ]
}

/// Chạy quy trình đồng bộ Git và kích hoạt GitHub Actions build bản tùy biến
#[tauri::command]
pub async fn sync_and_trigger_custom_build(
    app: AppHandle,
    repo_path: Option<String>,
) -> Result<CustomSyncReport, String> {
    logger::log_info("[CustomUpdater] Bắt đầu quy trình đồng bộ repo tùy biến...");

    let repo_dir = match resolve_repo_path(repo_path) {
        Ok(path) => path,
        Err(err) => {
            logger::log_error(&format!("[CustomUpdater] Lỗi đường dẫn: {}", err));
            return Ok(CustomSyncReport {
                success: false,
                message: err,
                current_step: 0,
                total_steps: 5,
                logs: String::new(),
            });
        }
    };

    logger::log_info(&format!(
        "[CustomUpdater] Sử dụng thư mục repo: {:?}",
        repo_dir
    ));

    let mut all_logs = Vec::new();
    match recover_interrupted_custom_merge(&repo_dir) {
        Ok(Some(detail)) => all_logs.push(format!("=== Khôi phục ===\n{}\n", detail)),
        Ok(None) => {}
        Err(err) => {
            logger::log_error(&format!("[CustomUpdater] Preflight thất bại: {}", err));
            return Ok(CustomSyncReport {
                success: false,
                message: err.clone(),
                current_step: 0,
                total_steps: 5,
                logs: err,
            });
        }
    }

    if let Err(err) = validate_custom_branch(&repo_dir) {
        logger::log_error(&format!("[CustomUpdater] Preflight thất bại: {}", err));
        all_logs.push(format!("=== Preflight thất bại ===\n{}\n", err));
        return Ok(CustomSyncReport {
            success: false,
            message: err,
            current_step: 0,
            total_steps: 5,
            logs: all_logs.join("\n"),
        });
    }

    let steps = sync_steps();
    let total_steps = steps.len();

    for (index, step) in steps.iter().enumerate() {
        let step_num = index + 1;
        let command = command_label(step.program, step.args);
        logger::log_info(&format!(
            "[CustomUpdater] Bước {}/{}: {} (Lệnh: {})",
            step_num, total_steps, step.title, command
        ));

        let _ = app.emit(
            "custom-sync-progress",
            CustomSyncStepProgress {
                step: step_num,
                total: total_steps,
                title: step.title.to_string(),
                status: "running".to_string(),
                detail: String::new(),
            },
        );

        match run_command(&repo_dir, step.program, step.args) {
            Ok(output) => {
                all_logs.push(format!(
                    "=== Bước {}/{}: {} ===\n{}\n",
                    step_num, total_steps, step.title, output
                ));
                let _ = app.emit(
                    "custom-sync-progress",
                    CustomSyncStepProgress {
                        step: step_num,
                        total: total_steps,
                        title: step.title.to_string(),
                        status: "success".to_string(),
                        detail: output,
                    },
                );
            }
            Err(err) => {
                logger::log_error(&format!(
                    "[CustomUpdater] Thất bại ở bước {}/{}: {}",
                    step_num, total_steps, err
                ));
                all_logs.push(format!(
                    "=== THẤT BẠI Bước {}/{}: {} ===\n{}\n",
                    step_num, total_steps, step.title, err
                ));

                if step_num == 3 && merge_in_progress(&repo_dir) {
                    match run_git(&repo_dir, &["merge", "--abort"]) {
                        Ok(output) => all_logs.push(format!(
                            "=== Khôi phục sau lỗi merge ===\nRepo đã được đưa về trạng thái trước khi merge.\n{}\n",
                            output
                        )),
                        Err(abort_err) => all_logs.push(format!(
                            "=== Không thể khôi phục sau lỗi merge ===\n{}\n",
                            abort_err
                        )),
                    }
                }

                let _ = app.emit(
                    "custom-sync-progress",
                    CustomSyncStepProgress {
                        step: step_num,
                        total: total_steps,
                        title: step.title.to_string(),
                        status: "failed".to_string(),
                        detail: err.clone(),
                    },
                );

                return Ok(CustomSyncReport {
                    success: false,
                    message: format!("Thất bại: {}", step.title),
                    current_step: step_num,
                    total_steps,
                    logs: all_logs.join("\n"),
                });
            }
        }
    }

    logger::log_info("[CustomUpdater] Hoàn thành toàn bộ quy trình đồng bộ thành công!");

    Ok(CustomSyncReport {
        success: true,
        message: "Đồng bộ và kích hoạt build thành công trên GitHub Actions!".to_string(),
        current_step: total_steps,
        total_steps,
        logs: all_logs.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_merge_is_identified_only_by_its_exact_subject() {
        assert!(is_custom_updater_merge_message(&format!(
            "{}\n\n# Conflicts:\n# file.ts",
            CUSTOM_MERGE_MESSAGE
        )));
        assert!(!is_custom_updater_merge_message(
            "Merge branch 'main' into my-custom"
        ));
    }

    #[test]
    fn sync_uses_upstream_tracking_ref_and_preserves_custom_conflicts() {
        let steps = sync_steps();
        assert_eq!(steps[0].args, ["fetch", "upstream", "main"]);
        assert_eq!(
            steps[1].args,
            ["push", "origin", "upstream/main:refs/heads/main"]
        );
        assert_eq!(
            steps[2].args,
            [
                "merge",
                "-X",
                "ours",
                "upstream/main",
                "-m",
                CUSTOM_MERGE_MESSAGE,
            ]
        );
        assert_eq!(
            steps[4].args,
            ["push", "origin", "HEAD:refs/heads/my-custom"]
        );
    }
}
