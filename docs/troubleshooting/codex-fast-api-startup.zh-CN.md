# Cockpit / Codex 原生 Fast 不显示：给 AI 的诊断与修复说明

更新日期：2026-11-17。状态：源码修复草案，尚未完成新构建的实机验收，尚未进入官方发行版。

将本文件下载为 Markdown 文件后，可以直接交给你的 AI 编程助手。本文件末尾附完整源码补丁，不含 API Key、Cookie、登录凭据或个人账号数据。

## 先明确修复对象

本说明针对 Cockpit 启动时注册 Codex 运行时适配过晚的问题。修改对象是 Cockpit 源码，不是 Codex 安装包。它不是仅修改配置即可保证生效的教程。

如果你的要求是“只安装双方官方发布版本，然后按配置教程使用”，应等待 Cockpit 官方接纳、发布修复，并验证支持的版本组合。本文不把本地编译版本称为官方版本，不要求普通用户安装本地构建。

已观察到的问题组合：

- Cockpit Tools：1.4.0。
- Codex Desktop：26.1106.6628.0。
- Windows 11：10.0.26200。
- 本地 API Key 接入，所选模型具有 Fast 元数据。
- 通过 Cockpit 正常重启后，原生 Fast 仍未出现。

证据：一次启动在 22:23:37 确认客户端进程；22:23:43 才出现 `preload-registered-next-document`，随后出现 `restart-required`。只读检查得到 `nativePatchState: pending`、`document-already-started`；资源匹配检查通过。

这证明该次 Cockpit 首文档适配没有及时安装。源码中也存在“客户端启动与首份账号配置读取并行”的竞争，但它是不是全部原因，仍需新构建的 Windows 启动测试确认。

## 可直接复制给 AI 的任务

> 请按本文件诊断 Cockpit / Codex 在本地 API 模式下不显示原生 Fast 的问题。
>
> 先只读核实版本、启动方式、适配日志和所选模型能力。只有符合本文现象时才尝试补丁；其他原因要单独定位。
>
> 保留官方 Codex 安装包、签名、更新程序、登录凭据、正常 OAuth 登录、已有会话和用户自定义设置。不要修改 app.asar，不要伪造 ChatGPT/OAuth 身份、额度、模型能力或服务端响应档位，不要绕过首文档保护，不要强制刷新正在工作的窗口。
>
> 在独立的 Cockpit 源码工作目录修复。先检查工作区变更和适用版本，展示改动。缺少 Rust、Windows 构建工具或其他系统依赖时，说明缺项；未经授权不要安装系统全局依赖或替换正在使用的程序。
>
> 应用本文补丁后，运行相关测试和构建，再在隔离测试配置下验证。明确区分已有 JavaScript 测试、新增 Rust 测试、编译检查和真实客户端验收。任一步未运行或失败，都不能报告已修复。
>
> 最终交付变更清单、测试结果、实际菜单和请求档位证据、回退方法。源码修复未进入官方发行版之前，不得承诺所有人仅按配置就能获得按钮。

## 1. 只读诊断

1. 记录 Cockpit、Codex、操作系统版本和当前使用的模型。确认是本地 API 接入还是正常 ChatGPT/OAuth 登录。
2. 确认 Codex 是从 Cockpit 对应实例启动，Cockpit 在后台运行；不要只重开一个不受管理的快捷方式。
3. 在 Codex 配置中检查 `[features]` 的 `fast_mode = true`。不要覆盖整个配置文件，也不要把添加这一行等同于解锁界面。
4. 检查 Cockpit 当前日志中的 `CodexEnhancements`、`restart-required`、`document-already-started`、`preload` 信息。日志目录以当前版本实际配置为准；不要把旧版数据目录当作所有人的固定路径。
5. 核实所选模型通过当前客户端 `model/list` 返回的速度档位。不支持 Fast 的模型缺少选项是另一类情况，不要添加虚假的能力。
6. 如果资源匹配检查失败，说明客户端资源结构不匹配；本文的启动顺序补丁不能代替版本适配。

需要查看本机调试端口时，可在 PowerShell 执行以下只读命令。端口会在重启后变化，不要硬编码别人的端口，也不要对外开放调试端口。

```powershell
Get-CimInstance Win32_Process |
  Where-Object {
    $_.Name -eq 'ChatGPT.exe' -and
    $_.CommandLine -notmatch '--type=' -and
    $_.CommandLine -match '--remote-debugging-port[= ](\d+)'
  } |
  ForEach-Object {
    [pscustomobject]@{
      ProcessId = $_.ProcessId
      DebugPort = [regex]::Match(
        $_.CommandLine, '--remote-debugging-port[= ](\d+)'
      ).Groups[1].Value
    }
  }
```

只对正确实例的 `127.0.0.1` CDP 端口进行检查。读取 Codex 主页面中以下字段足以记录状态，不要导出 localStorage、账号文件、Cookie 或完整网络请求：

```javascript
({
  nativeFast: window.__cockpitEnhancements?.nativeFast,
  patchState: window.__cockpitEnhancements?.nativePatchState,
  diagnostic: window.__cockpitEnhancements?.nativePatchDiagnostic,
  preload: window.__cockpitStartupPreload,
  localApiOverride: window.__cockpitNativeFast?.allow('local', 'apiKey'),
  oauthOverride: window.__cockpitNativeFast?.allow('local', 'chatgpt'),
  remoteApiOverride: window.__cockpitNativeFast?.allow('remote', 'apiKey')
})
```

## 2. 补丁适用范围与原理

补丁已检查可应用于官方 `v1.4.0`，其提交为 `795e91f52c6d3df54fe16513e451e0d255945490`。制作补丁所用源码提交为 `22eb4df0b05f6395559b41a648da39e566e9e66b`；涉及的原有源码文件与上述发行标签相同。

运行时改动限于三个文件，另新增一个 Rust 测试文件：

- `src-tauri/src/modules/codex_app_enhancements.rs`：首份账号配置读取任务完成并发布结果后，通知启动流程；不再让启动流程仅靠并行任务调度碰运气。
- `src-tauri/src/modules/codex_app_injection.rs`：等待上述通知，最多 10 秒；失败或超时继续原生客户端启动。
- `src-tauri/src/commands/codex_instance.rs`：默认实例、命名实例均等待准备流程；等待结束后重新检查用户是否取消启动。
- `src-tauri/src/modules/codex_app_enhancements/launch_profile_tests.rs`：测试延迟读取、超时、读取任务停止和无可选准备的情况。

本补丁不修改身份资格规则、模型目录、请求计费、Codex 文件或自动更新设置。准备较慢时，启动最多增加 10 秒等待。它处理首份配置晚发布的竞争；CDP 连接是否仍可能晚于首文档，需要实机验证，不能仅因测试通过就省略该项。

## 3. 在独立源码目录应用

先使用仓库官方 README 的开发要求准备独立工作目录。不要在安装目录、已有业务项目或有未保存改动的仓库里直接执行下面的操作。

```powershell
git clone --branch v1.4.0 --single-branch https://github.com/jlcodes99/cockpit-tools.git cockpit-tools-fast-review
Set-Location cockpit-tools-fast-review
git status --short
git rev-parse HEAD
```

提取本文件末尾唯一的 `diff` 代码块，保存为 UTF-8 无 BOM、LF 换行的 `codex-fast-startup.patch`。补丁文件应以一个换行结束；预期 SHA-256：`b12ebe4a6031ee1c0090a79cf042d559ddfb7ac9acc05394daf003597a989b06`。

```powershell
Get-FileHash -Algorithm SHA256 .\codex-fast-startup.patch
git apply --check .\codex-fast-startup.patch
```

只有哈希符合且 `git apply --check` 成功时，才继续应用：

```powershell
git apply .\codex-fast-startup.patch
git diff --check
git diff --stat
git status --short
```

若补丁不适用，先检查官方是否已修复、目标版本是否不同，以及工作区是否有用户修改。不要使用强制覆盖、`git reset --hard`，不要把相似代码随意替换。不要将本补丁重复应用到已包含修复的分支。

## 4. 验证与构建

JavaScript 回归测试：

```powershell
node --experimental-vm-modules --test src-tauri/src/modules/codex_app_enhancements/service_rspack.test.cjs src-tauri/src/modules/codex_app_enhancements/browser_preload_guard.test.cjs src-tauri/src/modules/codex_app_enhancements/startup_preload.test.cjs
```

Rust 新增测试及现有相关测试：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib modules::codex_app_enhancements::launch_profile_tests
cargo test --manifest-path src-tauri/Cargo.toml --lib modules::codex_app_enhancements
```

构建前先阅读当前版本的官方 README 和构建脚本。依赖与系统构建工具齐全、且已获准进行本地构建时，可使用仓库现有入口：

```powershell
npm ci
npm run tauri build
```

构建会下载项目依赖并产生构建文件；不要覆盖正在使用的官方程序。使用独立的测试实例和测试配置，不把用户真实凭据写进测试数据或报告。运行本地测试版本应先获得使用者同意。

截至本文件制作时，已执行结果：20 项现有 JavaScript 测试通过、补丁格式检查通过、补丁可应用于 `v1.4.0`。未执行：新增 Rust 测试、完整编译、新构建的客户端实机验收。原因是制作环境没有 Rust 工具链，Docker 引擎未运行。

## 5. 实机验收，不能省略

验证前记录 Codex 安装包的哈希；验证后再次比较。只比较哈希，不上传安装包或账号数据。

1. 从修复构建的 Cockpit 启动本地 API 实例。检查首文档适配真正进入 `active`，而非反复提示重启。
2. 打开原生模型菜单，确认支持的模型出现 Fast 选项；分别选择普通档与 Fast，检查客户端实际发出的请求档位。
3. 默认实例、至少一个命名实例均测试。慢配置读取、配置读取失败、用户取消启动时，应保留正常启动或取消行为。
4. 正常 ChatGPT/OAuth 登录按原有流程验收；不要伪造账号，不要修改或批量清除用户凭据。远端或不支持的模型不应被强行放开。
5. 检查 Codex 正常更新入口和原有设置，确认安装包哈希、登录配置没有被本次修改改变。
6. 对“上游实际加速”单独验收：查看服务端返回的 `service_tier`。请求携带 `priority` / `fast` 只证明请求了 Fast；响应 `default` 代表普通处理，不能伪装为 Fast 或据此宣称加速。

若新构建仍报告 `document-already-started`，说明本补丁不足以解决该环境的早期 CDP 注册问题。保留脱敏时序证据继续定位，不要再次要求用户无意义地重启，也不要删除首文档保护、暂停或强制重载用户正在工作的窗口来掩盖问题。

## 6. 回退与报告

只在本次独立源码目录、且确认补丁涉及的文件没有新增用户改动时检查反向应用：

```powershell
git apply --reverse --check .\codex-fast-startup.patch
git apply --reverse .\codex-fast-startup.patch
```

停止本次测试实例，重新使用原官方 Cockpit / Codex。保留用户原有数据和会话，不使用批量删除、卸载或重装来代替回退。

交付报告写清：版本、具体改动、每类测试的已通过 / 已失败 / 未执行、真实菜单结果、请求档位与响应档位、是否发布官方版本。补丁和教程不等于官方已经修复。

参考：[Cockpit v1.3.78 速度控件说明](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.3.78)、[Cockpit v1.4.0](https://github.com/jlcodes99/cockpit-tools/releases/tag/v1.4.0)、[OpenAI Fast API 文档](https://developers.openai.com/api/docs/guides/fast-mode)。

## 附录：完整源码补丁

```diff
diff --git a/src-tauri/src/commands/codex_instance.rs b/src-tauri/src/commands/codex_instance.rs
index e10e1ea..c2e6f23 100644
--- a/src-tauri/src/commands/codex_instance.rs
+++ b/src-tauri/src/commands/codex_instance.rs
@@ -3224,7 +3224,12 @@ async fn codex_start_instance_internal(
             default_dir.clone(),
             injection_plan.port,
             default_bind_account_id.clone(),
-        );
+        )
+        .await;
+        if codex_instance_start_cancelled(&instance_id) {
+            modules::codex_app_injection::stop_for_profile(&default_dir);
+        }
+        ensure_codex_instance_start_not_cancelled(&instance_id)?;
         let launch_args = injection_plan.args.clone();
         let prepared_home = default_dir.clone();
         // Package registration, activation and PID confirmation can wait on
@@ -3534,7 +3539,12 @@ async fn codex_start_instance_internal(
         instance_dir.to_path_buf(),
         injection_plan.port,
         instance.bind_account_id.clone(),
-    );
+    )
+    .await;
+    if codex_instance_start_cancelled(&instance_id) {
+        modules::codex_app_injection::stop_for_profile(instance_dir);
+    }
+    ensure_codex_instance_start_not_cancelled(&instance_id)?;
     let launch_home = instance.user_data_dir.clone();
     let launch_args = injection_plan.args.clone();
     let pid = tauri::async_runtime::spawn_blocking(move || {
diff --git a/src-tauri/src/modules/codex_app_enhancements.rs b/src-tauri/src/modules/codex_app_enhancements.rs
index d2a5fe9..3aaab50 100644
--- a/src-tauri/src/modules/codex_app_enhancements.rs
+++ b/src-tauri/src/modules/codex_app_enhancements.rs
@@ -1,5 +1,6 @@
 //! Optional desktop enhancements, scoped to a launched Codex profile and its CDP port.
-//! Slow credential reads and all network work stay outside the launch transaction.
+//! Fresh launches wait briefly for the initial credential snapshot; network work
+//! stays outside the launch transaction and never pauses or reloads the client.
 
 mod fast;
 mod browser_preload;
@@ -11,7 +12,7 @@ use std::collections::{HashMap, HashSet};
 use std::path::{Path, PathBuf};
 use std::sync::{Arc, Mutex, OnceLock};
 use std::time::{Duration, Instant};
-use tokio::sync::watch;
+use tokio::sync::{oneshot, watch};
 use tokio::task::JoinHandle;
 
 const POLL: Duration = Duration::from_secs(3);
@@ -33,16 +34,22 @@ pub fn start_for_profile(
     port: u16,
     fresh_launch: bool,
     api_service: bool,
-) {
+) -> Option<oneshot::Receiver<()>> {
     stop_for_profile(&profile);
     if !enabled() && !api_service {
-        return;
+        return None;
     }
     let (stop, receiver) = watch::channel(false);
     if let Ok(mut active) = runtimes().lock() {
         active.insert(profile.clone(), stop);
     }
     let startup = fresh_launch.then(|| Arc::new(fast::StartupReload::new()));
+    let (launch_ready, prepared) = if fresh_launch {
+        let (ready, prepared) = oneshot::channel();
+        (Some(ready), Some(prepared))
+    } else {
+        (None, None)
+    };
     tauri::async_runtime::spawn(run_profile(
         profile,
         instance,
@@ -50,7 +57,21 @@ pub fn start_for_profile(
         startup,
         api_service,
         receiver,
+        launch_ready,
     ));
+    prepared
+}
+
+// A fresh client must not race the first credential read. Failure only skips
+// this optional preparation; it must never prevent the native client starting.
+pub(crate) async fn wait_for_launch_profile(
+    prepared: Option<oneshot::Receiver<()>>,
+    wait: Duration,
+) -> bool {
+    match prepared {
+        Some(prepared) => matches!(tokio::time::timeout(wait, prepared).await, Ok(Ok(()))),
+        None => true,
+    }
 }
 
 pub fn stop_for_profile(profile: &Path) {
@@ -274,7 +295,7 @@ impl Child {
 }
 
 // API Service quota adaptation uses the persisted instance binding. Optional
-// Fast/prediction features still wait for their verified profile snapshot.
+// Fast/prediction features still require their verified profile snapshot.
 fn profile_or_api_fallback(profile: Option<Profile>, api_service: bool) -> Option<Profile> {
     profile.or_else(|| api_service.then(Profile::default))
 }
@@ -308,6 +329,7 @@ async fn run_profile(
     startup: Option<Arc<fast::StartupReload>>,
     api_service: bool,
     mut stop: watch::Receiver<bool>,
+    mut launch_ready: Option<oneshot::Sender<()>>,
 ) {
     // The launch transaction starts this worker before starting the process.
     // Browser discovery must race neither a blocking keyring read nor a second
@@ -359,6 +381,11 @@ async fn run_profile(
         }
         if read.as_ref().is_some_and(JoinHandle::is_finished) {
             let verified = read.take().unwrap().await.ok().and_then(Result::ok);
+            // The read worker has already published the complete trusted
+            // preload source. Only now may the launch transaction start Codex.
+            if let Some(ready) = launch_ready.take() {
+                let _ = ready.send(());
+            }
             initial_snapshot_ready = true;
             current_verified = verified.is_some();
             let next = profile_or_api_fallback(verified, api_service);
@@ -387,6 +414,9 @@ async fn run_profile(
                 tick.reset();
             }
             logger::log_warn("[Codex enhancements] profile snapshot timed out; optional adaptations suspended");
+            if let Some(ready) = launch_ready.take() {
+                let _ = ready.send(());
+            }
             let safe = profile_or_api_fallback(None, api_service);
             if current != safe {
                 for (_, child) in children.drain() {
@@ -478,6 +508,10 @@ async fn run_profile(
     }
 }
 
+#[cfg(test)]
+#[path = "codex_app_enhancements/launch_profile_tests.rs"]
+mod launch_profile_tests;
+
 #[cfg(test)]
 mod tests {
     use super::*;
diff --git a/src-tauri/src/modules/codex_app_enhancements/launch_profile_tests.rs b/src-tauri/src/modules/codex_app_enhancements/launch_profile_tests.rs
new file mode 100644
index 0000000..0140c2d
--- /dev/null
+++ b/src-tauri/src/modules/codex_app_enhancements/launch_profile_tests.rs
@@ -0,0 +1,51 @@
+use super::*;
+
+#[tokio::test]
+async fn client_launch_waits_for_the_initial_profile_publication() {
+    let (published, prepared) = oneshot::channel();
+    let (started, starting) = oneshot::channel();
+    let launched = Arc::new(std::sync::atomic::AtomicBool::new(false));
+    let observed = launched.clone();
+    let launch = tokio::spawn(async move {
+        started.send(()).unwrap();
+        assert!(wait_for_launch_profile(Some(prepared), Duration::from_secs(2)).await);
+        observed.store(true, std::sync::atomic::Ordering::SeqCst);
+    });
+    starting.await.unwrap();
+    tokio::task::yield_now().await;
+    assert!(!launched.load(std::sync::atomic::Ordering::SeqCst));
+    // run_profile signals only after the read worker published its result.
+    published.send(()).unwrap();
+    launch.await.unwrap();
+    assert!(launched.load(std::sync::atomic::Ordering::SeqCst));
+}
+
+#[tokio::test]
+async fn stalled_profile_read_does_not_prevent_native_startup() {
+    let (_published, prepared) = oneshot::channel();
+    let ready = tokio::time::timeout(
+        Duration::from_secs(1),
+        wait_for_launch_profile(Some(prepared), Duration::from_millis(20)),
+    )
+    .await
+    .expect("optional preparation must have a bounded wait");
+    assert!(!ready);
+}
+
+#[tokio::test]
+async fn stopped_profile_worker_does_not_hold_up_native_startup() {
+    let (published, prepared) = oneshot::channel();
+    drop(published);
+    let ready = tokio::time::timeout(
+        Duration::from_secs(1),
+        wait_for_launch_profile(Some(prepared), Duration::from_secs(10)),
+    )
+    .await
+    .expect("worker failure must release the launch immediately");
+    assert!(!ready);
+}
+
+#[tokio::test]
+async fn profiles_without_optional_preparation_start_normally() {
+    assert!(wait_for_launch_profile(None, Duration::ZERO).await);
+}
diff --git a/src-tauri/src/modules/codex_app_injection.rs b/src-tauri/src/modules/codex_app_injection.rs
index d2d2452..745a6ee 100644
--- a/src-tauri/src/modules/codex_app_injection.rs
+++ b/src-tauri/src/modules/codex_app_injection.rs
@@ -628,18 +628,28 @@ pub fn start_for_profile(
     port: Option<u16>,
     bind_account_id: Option<String>,
 ) {
-    start_profile(app, instance_id, profile_dir, port, bind_account_id, false);
+    let _ = start_profile(app, instance_id, profile_dir, port, bind_account_id, false);
 }
 
 /// Only the unified launch transaction may arm first-document adaptation.
-pub fn start_for_launched_profile(
+pub async fn start_for_launched_profile(
     app: AppHandle,
     instance_id: String,
     profile_dir: PathBuf,
     port: Option<u16>,
     bind_account_id: Option<String>,
 ) {
-    start_profile(app, instance_id, profile_dir, port, bind_account_id, true);
+    let prepared = start_profile(app, instance_id, profile_dir, port, bind_account_id, true);
+    if !crate::modules::codex_app_enhancements::wait_for_launch_profile(
+        prepared,
+        Duration::from_secs(10),
+    )
+    .await
+    {
+        crate::modules::logger::log_warn(
+            "[CodexEnhancements] launch profile preparation unavailable; continuing native client startup",
+        );
+    }
 }
 
 fn start_profile(
@@ -649,9 +659,9 @@ fn start_profile(
     port: Option<u16>,
     bind_account_id: Option<String>,
     fresh_launch: bool,
-) {
-    let Some(port) = port else { return };
-    crate::modules::codex_app_enhancements::start_for_profile(
+) -> Option<tokio::sync::oneshot::Receiver<()>> {
+    let port = port?;
+    let prepared = crate::modules::codex_app_enhancements::start_for_profile(
         profile_dir.clone(),
         instance_id.clone(),
         port,
@@ -670,7 +680,7 @@ fn start_profile(
         );
     }
     if !should_enable_profile_injection(&profile_dir, bind_account_id.as_deref()) {
-        return;
+        return prepared;
     }
     stop_injection_for_profile(&profile_dir);
     let key = profile_key(&profile_dir);
@@ -682,6 +692,7 @@ fn start_profile(
     if let Ok(mut items) = runtimes().lock() {
         items.insert(key, InjectionRuntime { task });
     }
+    prepared
 }
 
 #[derive(Debug, Clone)]
```

