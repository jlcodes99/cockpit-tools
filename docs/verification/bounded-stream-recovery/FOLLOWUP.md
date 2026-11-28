# PR3361 后续安全检查与本机发布验证

本文补充首轮报告，保留先前原始结果；不把旧源码的测试回执改成新源码回执。

## CodeQL 检查与测试夹具加固

初次提交 `1cca4f015cf671348abc9a3baa299d7f9bf2291a` 的九项 Build Matrix 工作及两项 CodeQL 分析工作成功，单独的 CodeQL policy check 失败。失败摘要同时包含 main 的 `/language:go` configuration 缺失，以及一项新增 critical 告警，不能仅归为工作流配置。

实际告警为 `rust/unsafe-deserialization`，查询包 `codeql/rust-queries/0.1.48`，指向新增 `stream_recovery_http_tests.rs` 中 Axum Bytes 到 `serde_json::from_slice::<Value>(...).unwrap()`。这是 `cfg(test)` 的环回模拟上游，通用JSON树不是任意类型对象实例化，未发现任意代码执行路径；但解析分配及unwrap仍可受错误输入影响，故未直接忽略告警或关闭扫描器。准确告警数据流图读取权限返回403，本说明的source/sink来自对应源码和公开check annotation，不能冒称获得完整服务器分析图。

提交 `c2e547de88d8a99f19708231a04703d856e73f1a` 仅修改测试文件：并发分支增加16KiB显式限制，仅匹配三种已知精确合成content标识；未知、重复、混合、转义伪装与超限请求在计数前拒绝。分类器只服务本夹具，不是通用JSON验证器。计数与记录共用同一锁，避免并发误计数。取消handler与并发断言里的通用网络JSON反序列化，仍保留三流2/1/1发送、重发原字节一致、池退休一次及permit归还断言。四项新增边界单元覆盖合法、错误/伪装、重复/歧义及体积边界。

不改变生产恢复算法、账户选择、超时、请求字节和内存保持策略，也没有加扫描器豁免或改作者工作流。首轮 `source-files.json` 仍对应当时D6A实测副本；当前测试文件新增差异及对应后续回执单独记录。

后续准确源码库测试编译成功、sourceStable=true；新测试EXE SHA256为`1090723F7F0BD1191714BA0E75787DEE81C7D110D9ABE17FA447584F92C7BF3E`，228,350,976字节。受影响HTTP模块20项（原16项+新4项）全部实际通过，0失败/忽略，21.59秒。10个其他修改源文件与首轮实测字节一致；后续测试名、构建指纹见`followup-results.json`，当前11源码哈希见`source-files-current.json`。原始145项首轮144/1及后续上传配对结果仍保留，不改写成149项全套重跑通过。

## 最终提交的 CI 结果

截至2026-11-28 05:41 UTC，准确提交 `ce26ba83dc6f90f1864f25b9cd7d678c61dbfd3f` 的检查为11项SUCCESS、1项NEUTRAL、0失败、0等待：九项Build Matrix与两项CodeQL语言分析均成功。CodeQL policy仅剩与main比较时缺失的 `/language:go` 配置，当前check输出0条annotation，未再报告先前critical反序列化告警。这是公开check输出证据，不是私有告警dismissal证据，也不等于全仓库零告警或无条件全绿。准确check链接及结果见[发布验证数据](release-results.json)。

## 本机 Release 构建与产物身份

从上述干净提交运行正常Tauri CLI 2.12.6 Release构建，保留 `node scripts/prepare-tauri.cjs && npm run build`，实际完成版本核对、TypeScript及Vite生产编译；`frontendDist` 仍为相对目录 `../dist`。Cargo使用locked/offline和优化Release配置，子进程环境先清空。未创建或分发安装器。构建前后3,350个源码文件的指纹均为 `B327DE3A19D1C08B91EACC66B5496906EA164E9E908F6C91C77C934C6A48589F`，工作树干净。此后追加的本文和JSON属于文档增量，不改变该二进制的源码身份。

生成的1.5.3版EXE为175,255,552字节（167.14MiB），SHA256为 `0468F8AA3EF52FD71AB22893C980F9B641E28B34FC437113E7E18443AA63894D`。生产前端共173文件、19,171,199字节，树哈希为 `4D9FB81A9B31FBC29BB90E614DD81C98B2D1482B21A30DD256E71E6E1497F500`；Rust嵌入守卫核对11个入口/引用资产。批准的部署清单包含EXE和全部21项配置资源，共22项，按清单逐项核验大小和SHA256一致；运行时产生的其他文件不在安装或公开范围。清单收录于`release-results.json`。

Tauri/Cargo编译成功（Release编译23分22秒），原辅助包装脚本随后在前端清单统计中失败：严格模式下 `Measure-Object` 不能直接读取OrderedDictionary的 `bytes` 键。原包装回执exit 1完整保留；其SHA固定的启动脚本只在子进程exit 0分支填入EXE记录，且Tauri日志具有准确产物路径的完成标记。修正统计为显式取数值后，独立后处理重新核源码指纹和同一EXE哈希、复制资源并生成清单，exit 0。没有重新编译、修改EXE或把原失败回执改为通过。编译存在854项库警告、1项EXE链接输出警告及 `proc-macro-error2 2.0.1` 的未来兼容性提示，因此也不称无警告构建。

## 同一 EXE 的真实 WebView2 验收

对上述同一SHA的最终EXE运行其已有隔离前端模式，使用清空的继承环境和专用profile，不注册生产handlers/plugins。第二轮内外层均Passed、exit 0：实际加载嵌入入口脚本和CSS，1280×800下三次有效DOM快照、连续稳定2,000ms；三次均有非空root、可见app/main、`readyState=complete`、入口脚本加载及10份样式表，致命错误和资源错误均为0。`frontendReady=true`，事件循环和测试进程退出。32种业务命令被隔离模式拒绝，未执行生产业务API；按有界轮询观察到的8个子进程均已退出。验收只覆盖实际观察到的宿主及子进程，不能证明未观察到的脱离进程不存在。

两轮 `frontendStages` 实际都仅含 `script_loaded`、`react_mount_start`，没有 `react_mounted`；通过依据是上述真实DOM/资源/ready/退出证据，未补造阶段事件。资源记录包含Google Fonts CSS，因此不称零外网或操作系统网络沙箱。该验收不等于真实账户、生产API或上游推理实测；未使用屏幕采集。

首轮内层已Passed，但外层把运行时实际用户目录追加的 `EBWebView` 当作异常而标Failed，失败处理执行了准确观察对象的清理。随后只修正包装脚本的精确路径校验，验证专用配置目录加上该固定后缀，再对同一EXE重跑通过；未宽泛放行任意子路径，也未重新构建。微软官方文档明确说明该后缀由WebView2 Runtime自动添加，见[WebView2用户数据目录说明](https://learn.microsoft.com/en-us/microsoft-edge/web-platform/devtools-mcp-server#step-2-find-the-webview2-user-data-directory)。此链接只说明目录行为，不代表本次使用DevTools MCP、远程调试端口或注册表覆盖。

## 部署状态

本机安装准备和同EXE前端验收已完成，部署尚未执行，生产EXE尚未替换。后续在用户正常退出应用后执行既定本机部署流程；本次不分享安装器。公开数据仅包含脱敏统计、相对资产名及哈希，不含本机绝对路径、账号、进程ID或原始运行日志。
