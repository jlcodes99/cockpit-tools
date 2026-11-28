# PR3361 后续安全检查与本机发布验证

本文补充首轮报告，保留先前原始结果；不把旧源码的测试回执改成新源码回执。

## CodeQL 检查与测试夹具加固

初次提交 `1cca4f015cf671348abc9a3baa299d7f9bf2291a` 的九项 Build Matrix 工作及两项 CodeQL 分析工作成功，单独的 CodeQL policy check 失败。失败摘要同时包含 main 的 `/language:go` configuration 缺失，以及一项新增 critical 告警，不能仅归为工作流配置。

实际告警为 `rust/unsafe-deserialization`，查询包 `codeql/rust-queries/0.1.48`，指向新增 `stream_recovery_http_tests.rs` 中 Axum Bytes 到 `serde_json::from_slice::<Value>(...).unwrap()`。这是 `cfg(test)` 的环回模拟上游，通用JSON树不是任意类型对象实例化，未发现任意代码执行路径；但解析分配及unwrap仍可受错误输入影响，故未直接忽略告警或关闭扫描器。准确告警数据流图读取权限返回403，本说明的source/sink来自对应源码和公开check annotation，不能冒称获得完整服务器分析图。

提交 `c2e547de88d8a99f19708231a04703d856e73f1a` 仅修改测试文件：并发分支增加16KiB显式限制，仅匹配三种已知精确合成content标识；未知、重复、混合、转义伪装与超限请求在计数前拒绝。分类器只服务本夹具，不是通用JSON验证器。计数与记录共用同一锁，避免并发误计数。取消handler与并发断言里的通用网络JSON反序列化，仍保留三流2/1/1发送、重发原字节一致、池退休一次及permit归还断言。四项新增边界单元覆盖合法、错误/伪装、重复/歧义及体积边界。

不改变生产恢复算法、账户选择、超时、请求字节和内存保持策略，也没有加扫描器豁免或改作者工作流。首轮 `source-files.json` 仍对应当时D6A实测副本；当前测试文件新增差异及对应后续回执单独记录。

后续准确源码库测试编译成功、sourceStable=true；新测试EXE SHA256为`1090723F7F0BD1191714BA0E75787DEE81C7D110D9ABE17FA447584F92C7BF3E`，228,350,976字节。受影响HTTP模块20项（原16项+新4项）全部实际通过，0失败/忽略，21.59秒。10个其他修改源文件与首轮实测字节一致；后续测试名、构建指纹见`followup-results.json`，当前11源码哈希见`source-files-current.json`。原始145项首轮144/1及后续上传配对结果仍保留，不改写成149项全套重跑通过。
