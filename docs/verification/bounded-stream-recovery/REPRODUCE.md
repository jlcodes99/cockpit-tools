# 有界流恢复的本地复跑

本说明用于修复分支源码的隔离回归。测试使用合成账号标识、请求体和本机环回服务，不向真实模型账号发请求。库测试并不启动Cockpit前端，不能作为发布安装包的界面验收。不要在生产账号上主动制造TLS故障。

## 常规Rust回归

使用项目正常的Rust/Tauri开发环境。Windows需要匹配的MSVC和Windows SDK。依赖准备完成后，在仓库根目录的新PowerShell会话中执行；下面只将测试数据放进专属临时目录，不修改现有软件配置。

```powershell
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('cockpit-stream-recovery-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$env:COCKPIT_TOOLS_TEST_DATA_DIR = $testRoot
$env:COCKPIT_TEST_DATA_DIR = $testRoot
$env:COCKPIT_TOOLS_DATA_DIR = $testRoot

$modules = @(
  'stream_recovery',
  'stream_recovery_http_tests',
  'stream_recovery_cli_tests::public_wire_decoder::tests',
  'tls_recovery_http_tests',
  'bootstrap::tests',
  'lifecycle::tests',
  'response_wire::tests',
  'tests::request_upload_http_tests',
  'tests::request_upload_nack_tests',
  'http_compression::large_compression_tests',
  'events::tests',
  'execution::layout_tests'
)
foreach ($module in $modules) {
  cargo test --locked -p cockpit-tools --lib "modules::codex_native_api::${module}::" -- --test-threads=1
  if ($LASTEXITCODE -ne 0) { throw "Regression failed: $module" }
}
```

检查每次输出实际匹配到测试且通过，不能把0测试或全部ignored当成功。已有依赖缓存完整时可增加`--offline`。测试涉及真实环回HTTP/TLS、超时和取消，低资源机器可出现时序失败，应保存完整失败日志分析，不能简单重跑直到绿色再抹掉首次失败。

## 显式选择已安装的Codex客户端

`stream_recovery_cli_tests`中的两项真实CLI用例默认ignored，仅Windows支持；其4项wire解码测试由上面的常规列表执行。它不是调用模型的测试：选定本机CLI后，CLI连接本地真实网关，网关只连接合成上游。必须显式提供准确EXE路径、SHA256与一个新的空证据目录；不能依靠PATH猜选或在缺少CLI时默默跳过。

```powershell
# Replace this example path with the executable being validated.
$clientExecutable = 'C:\path\to\codex.exe'
$clientDigest = (Get-FileHash -LiteralPath $clientExecutable -Algorithm SHA256).Hash
$cliEvidence = Join-Path $testRoot 'cli-evidence'
New-Item -ItemType Directory -Path $cliEvidence | Out-Null
$env:COCKPIT_STREAM_RECOVERY_CLI_OPT_IN = '1'
$env:COCKPIT_STREAM_RECOVERY_CODEX_BIN = $clientExecutable
$env:COCKPIT_STREAM_RECOVERY_CODEX_SHA256 = $clientDigest
$env:COCKPIT_STREAM_RECOVERY_CLI_EVIDENCE_DIR = $cliEvidence
cargo test --locked -p cockpit-tools --lib 'modules::codex_native_api::stream_recovery_cli_tests::' -- --ignored --test-threads=1 --nocapture
if ($LASTEXITCODE -ne 0) { throw 'Explicit CLI contracts failed' }
```

必须实际运行两项、2 passed/0 failed/0 ignored。每项会保存`result.json`及合成public wire证据；确认成功场景只有1个客户端POST、2个字节相同的上游POST、两个不同created ID、最终`turn.completed`及唯一期望答案，并人工确认`turnFailed=false`。预算失败场景应1个客户端POST、1个上游POST、真实`network_recovery_exhausted`和`turn.failed`，没有成功结果或客户端补发。观察器须正确解析HTTP分块及Content-Encoding后再读SSE，不将压缩wire直接当作UTF8。

CLI子进程清空继承环境，使用测试自己的应用数据与工作目录、文件凭据后端，关闭Apps/plugins/MCP/项目指令和守护进程，设置request/stream retries为0，并记录本地拒绝代理。断流用例是HTTP body传输错误；真实TLS/H1/H2和纯namespace转换由其他模块验证。没有把这组最小CLI声明等同于Desktop全部插件工具声明。配置隔离与代理记录不等于操作系统网络沙箱。

## 前端发布保护

```powershell
cargo test --locked --manifest-path scripts/release/frontend-guard-tests/Cargo.toml -- --test-threads=1
```

这只验证现有frontend guard。若后续构建Windows发布包，仍须运行项目Tauri构建流程、确认其实际`frontendDist`资源嵌入，最后从最终包提取同一个EXE进行真实WebView渲染验收。不要用库测试、EXE体积或字符串命中代替实际界面验收。
