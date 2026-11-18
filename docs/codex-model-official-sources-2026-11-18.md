# Codex 模型支持与 OpenAI API 价格核对

核对日期为 2026 年 11 月 18 日（北京时间）。OpenAI 于 11 月 17 日公布 GPT-6.2 Astra 和 Sol，两个精确 ID 均已用于官方 Codex 并发布至公有 API。Codex 登录的账号权限、客户使用自备 API Key 的项目权限和两类计费规则分别由官方产品文档说明。 [发布公告](https://openai.com/index/introducing-gpt-6-2-astra-sol/) · [Codex 模型](https://learn.chatgpt.com/docs/models) · [API 迁移指南](https://developers.openai.com/api/docs/guides/latest-model)

## Codex 官方客户端

- 官方 CLI 支持 `codex -m gpt-6.2-sol`，非交互入口同样支持 `-m`。`rust-v0.169.1` 随包目录已包含 `gpt-6.2-sol` 和 `gpt-6.2-astra`。两者目录均为 `visibility: list`、`supported_in_api: true`、`minimal_client_version: 0.166.0`，默认 `low`，支持 `low`、`medium`、`high`、`xhigh`、`max`、`ultra`。Sol 默认上下文 500,000，Astra 272,000。两者随包目录的 `max_context_window` 均为 872,000，产品文档描述可配置至约 100 万，公有 API 独立标注 1,050,000 总窗口、922,000 最大输入及 128,000 最大输出。 [Codex 模型](https://learn.chatgpt.com/docs/models) · [官方 CLI 发布](https://github.com/openai/codex/releases/tag/rust-v0.169.1) · [Sol 随包目录](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/models-manager/models.json#L4) · [Astra 随包目录](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/models-manager/models.json#L184) · [Sol API 模型页](https://developers.openai.com/api/docs/models/gpt-6.2-sol)
- 随包两条记录都提供 `service_tiers` 的 `priority`（显示名 Fast），也声明了 `additional_speed_tiers: ["fast"]`。官方产品文档另确认 Ultrafast 资格。Pro $500 可对两个 6.2 模型使用，Pro $200 可在 Codex 对 6.2 Sol 使用，符合条件的 Enterprise/Edu 由管理员开启。公开模型页还把 Business 列入基础 6.2 模型适用计划。订阅内 6.2 Ultrafast 按 Standard 的 6 倍扣额度，符合条件的购入 credits 促销价按 3 倍计至 2027 年 2 月 28 日。 [官方 CLI 随包目录](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/models-manager/models.json#L4) · [Work 与 Codex 计划帮助](https://help.openai.com/en/articles/20001275-chatgpt-work-and-codex) · [Codex 定价](https://learn.chatgpt.com/docs/pricing)
- `ultra` 属于 Codex 的推理和任务委派选项，Ultrafast 属于服务档位。公有 API 可用 `gpt-6.2-sol` 和 `gpt-6.2-astra` 调用 Responses 与 Chat Completions，两者的工具调用使用 Responses。API 指南建议新接入从 `medium` 开始，由 `none` 或 `minimal` 迁移至 `low`。模型页明确 `none` 会被拒绝，Codex 随包目录的推理选项和默认值只说明客户端行为。 [Codex 模型](https://learn.chatgpt.com/docs/models) · [API 迁移指南](https://developers.openai.com/api/docs/guides/latest-model) · [Sol API 模型页](https://developers.openai.com/api/docs/models/gpt-6.2-sol)

## 公有 API 费率

下表按美元／百万文本 token 计，采用 Standard 短上下文价格。官方 Prompt Caching 文档给出 GPT-5.6 及后续模型在 Responses API 的 30 分钟和 12 小时写入总单价分别为未缓存输入的 1.25 倍和 2.5 倍。Codex 订阅 credits 费率没有独立缓存写入收费。 [API 定价表](https://developers.openai.com/api/docs/pricing) · [API 缓存规则](https://developers.openai.com/api/docs/guides/prompt-caching) · [Codex 定价](https://learn.chatgpt.com/docs/pricing) · [6.1 Terra](https://developers.openai.com/api/docs/models/gpt-6.1-terra) · [6 Luna](https://developers.openai.com/api/docs/models/gpt-6-luna) · [6 Sol](https://developers.openai.com/api/docs/models/gpt-6-sol) · [5.6 Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol) · [5.6 Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra) · [5.6 Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna) · [5.3 Codex](https://developers.openai.com/api/docs/models/gpt-5.3-codex)

| 模型 | Standard 输入 | 缓存读取 | 30 分钟写入 | 12 小时写入 | 输出 |
| --- | ---: | ---: | ---: | ---: | ---: |
| `gpt-6.2-sol`、`gpt-6.1-sol` | 2 | 0.10 | 2.50 | 5 | 10 |
| `gpt-6.2-astra` | 8 | 0.40 | 10 | 20 | 40 |
| `gpt-6-astra` | 10 | 1 | 12.50 | 25 | 50 |
| `gpt-6.1-terra` | 1 | 0.05 | 1.25 | 2.50 | 5 |
| `gpt-6-luna` | 0.10 | 0.01 | 0.125 | 0.25 | 0.50 |
| `gpt-6-sol` | 2 | 0.20 | 2.50 | 5 | 10 |
| `gpt-5.6-sol`，促销期 | 4 | 0.40 | 5 | 10 | 20 |
| `gpt-5.6-terra` | 2 | 0.20 | 2.50 | 5 | 12 |
| `gpt-5.6-luna` | 0.20 | 0.02 | 0.25 | 0.50 | 1.20 |
| `gpt-5.3-codex`，标注弃用 | 1.75 | 0.175 | — | — | 14 |

GPT-5.6 的 12 小时写入单价按官方缓存指南的 Responses API 规则计算，使用未缓存输入价格的 2.5 倍。 [OpenAI 缓存规则](https://developers.openai.com/api/docs/guides/prompt-caching#summary-of-model-differences)

- GPT-6.2 两款的标准长上下文费率从输入超过 272,000 token 起按整个请求计算，输入及各缓存项乘 2，输出乘 1.5。API Fast 为相同上下文长度 Standard 的 2 倍，`priority` 和 `fast` 均可指 Fast。API Batch 为 0.5 倍。API Ultrafast 使用 `service_tier: "ultrafast"`，截至太平洋时间 2027 年 2 月 28 日为 3 倍，自 3 月 1 日起为 6 倍。旧 `gpt-6-astra` 和 `gpt-6.1-sol` 的 Ultrafast 仍为 6 倍。符合条件的后台请求另有按交付时限折扣，2 小时为 Standard 的 70%，8 小时为 60%。 [API 定价表](https://developers.openai.com/api/docs/pricing) · [Sol API 模型页](https://developers.openai.com/api/docs/models/gpt-6.2-sol) · [Astra API 模型页](https://developers.openai.com/api/docs/models/gpt-6.2-astra) · [API Fast 指南](https://developers.openai.com/api/docs/guides/fast-mode) · [API Ultrafast 指南](https://developers.openai.com/api/docs/guides/ultrafast-mode)
- API 定价表和 5.6 Sol 模型页均把促销截至日写为 2026 年 11 月 30 日。较早的 Fast 帮助页写的是至少持续至 11 月 21 日。现价以定价表和模型页为准，促销后的生效价和具体切换时刻尚无可核实的公开值。 [API 定价表](https://developers.openai.com/api/docs/pricing) · [5.6 Sol API 模型页](https://developers.openai.com/api/docs/models/gpt-5.6-sol) · [较早的 Fast 指南页面](https://developers.openai.com/api/docs/guides/fast-mode?trk=article-ssr-frontend-pulse_little-text-block)
- `codex-auto-review` 在官方 CLI 随包目录为 `visibility: hide` 且 `supported_in_api: true`。OpenAI 公开 API 价格表未列这个 ID，独立模型页返回 404，因此常规 API 项目的可调用性与单独费率未核实。ChatGPT 登录时，审批安全检查免费且不占订阅额度。GitHub 代码审查按适用 Codex 用量计费，费率卡同时指明它使用 `gpt-5.3-codex`。`gpt-5.3-codex-spark` 公告列为研究预览，API 仅说明曾向少数设计合作伙伴提供访问，未找到公开单价。 [随包 Auto Review 记录](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/models-manager/models.json#L1986) · [官方 API 定价表](https://developers.openai.com/api/docs/pricing) · [ChatGPT credits 费率卡](https://help.openai.com/en/articles/11481834-chatgpt-rate-card-business-enterpriseedu-credit-based-pricing) · [Codex 定价](https://learn.chatgpt.com/docs/pricing) · [Codex-Spark 公告](https://openai.com/index/introducing-gpt-5-3-codex-spark/)

## Cockpit 本地覆盖与缺口

| 入口 | 当前实现 | 拟调整 |
| --- | --- | --- |
| 模型管理、默认与多开实例 | `default_experimental_model_definitions` 从 `SHIPPED_VISIBLE_CODEX_MODEL_IDS` 取 ID，落盘到受管 `cockpit-model-catalog.json`。旧版本迁移按单模型补项，生成戳目前以代码版本和目录文件哈希为准。 | 默认模型清单和恢复默认读取统一目录。生成戳带默认目录内容版本，默认实例和多开在现有生成入口更新宿主管理的目录。已有手动选定模型、排序和第三方供应商目录照原有归属处理。 |
| API 服务与模型路由 | 默认广告清单和可路由清单分别从 `recommended_models()`、`managed_codex_model_ids()` 派生，后者还有写死的旧型号排序。账号与供应商有各自的发现目录、模型映射和 Key 范围。 | 统一消费模型元数据和推荐顺序。检查模型选择器、`/v1/models`、Codex 协议目录、Responses HTTP/WS、兼容入口、Key 白名单和实际路由。账号可用性继续按已有的账号和供应商能力判定。 |
| 辅助预设 | 模型名、能力提示、唤醒和测试候选还有独立列表。 | 改用相同元数据，保留现有省额度的唤醒默认和用户已保存的任务。 |
| API 服务定价 | 代码常量存 Standard/Fast 档，本地费用估算和预算共用。Ultrafast 全局按 6 倍，缓存写入的 1.25 倍只匹配固定的模型名单。 | 补 6.2 两款和可核实的当前官方价格。按模型、档位、生效时间和长上下文规则选择费率。优先保留用户自定义价，预算检查和估算共用价格解析。 |

[模型管理默认清单](../src-tauri/src/modules/codex_account_model_catalog.rs#L666) · [受管目录版本戳](../src-tauri/src/modules/codex_managed_model_catalog_version.rs) · [API 服务默认清单](../src-tauri/src/modules/codex_local_access_foundation.rs#L2395) · [原生 API 目录与路由](../src-tauri/src/modules/codex_local_access_native_routes.rs) · [价格引擎](../src-tauri/src/modules/codex_local_access_routing_pricing.rs#L1214) · [API Key 预算价格检查](../src-tauri/src/modules/codex_local_access_budget.rs#L148)

2026-11-18 直接读取当前代码配置的 Router for Me GitHub 来源，返回了 `gpt-6.2-sol` 和 `gpt-6.2-astra`，均标为可列出、可用 API、走 Responses Lite。该来源的 Sol 宣告 Ultrafast，Astra 仅宣告 Fast。官方产品文档确认两款均有 Ultrafast，具体是否对账号和接入方式开放仍按对应能力判断。当前 Cockpit 内置资源和本地价表缺少两款，启动时加载的第三方目录由 `OnceLock` 固定，运行期间没有刷新。这解释了在线时 API 派生清单可能已有 6.2、离线默认与模型管理清单仍缺、价格页也没有对应预设的情况。上述为对代码和公开数据的核查，未发起真实账号请求。 [第三方目录](https://github.com/router-for-me/models/blob/main/codex_client_models.json) · [加载实现](../src-tauri/src/modules/codex_model_catalog.rs#L17) · [本地内置目录](../resources/codex/codex_client_models.json) · [官方速度规则](https://help.openai.com/en/articles/20001275-chatgpt-work-and-codex)

## 建议实现

1. **先统一默认目录与价格数据。** 首版在宿主内置同一份可离线读取的数据，模型项记录稳定 ID、显示名、推荐顺序、上下文、推理与速度能力。费率项记录来源、适用模型或明确的快照别名、计价档位和生效时间。把 6.2 Sol 放在推荐首位，同批加入 6.2 Astra。新建或恢复默认时，目标账号可用才选 6.2 Sol。已有显式模型选择继续优先。订阅的可用性和公有 API 的可用性由各自上游目录标明。
2. **通过现有远端配置发布数据。** 给项目自有的 `remote-config.json` 加一个可选的 Codex 数据快照引用，快照带版本和来源链接。Rust 后台沿用默认一小时的刷新节奏并使用 ETag，单飞限时下载。先用上次有效快照或内置数据提供服务，校验成功后原子保存并在内存切换。通过同一快照更新默认候选与费率，避免带预算的 Key 看见新模型却没有价格。模型目录真正变化才刷新受管 profile；费率更新时只刷新计价。远端字段限于结构化元数据和价格，客户端指令模板、协议代码和路由配置留在宿主。
3. **把公开 API 价格按请求发生时间应用。** 优先使用可信的实际上游模型、实际服务档位和已采集的 token 明细。6.2 Ultrafast 的 3 倍到 6 倍切换预先录入，离线也会按请求时间生效。30 分钟与 12 小时缓存写入需要相应的用量证据，支持混用时按可区分的明细计算。未能区分的用量保留估算标记。Codex 订阅额度和 Credits 属于另一套口径，价格页继续说明这是 API 美元参考估算。官方涨降价仅影响对应生效时间内的请求。若修正了曾经错算或漏算的历史区间，复用现有按模型分批重算能力，按日志时间选择价格并保存使用的价格版本。
4. **维护以官方来源为准。** 首批将两款 6.2 及本地现有 Codex/GPT 文本价表逐条对照，保留 Grok 的独立价表。OpenAI 公布的 API 价目是定价来源，`/v1/models` 是账号可见模型来源。发布可由维护者直接更新版本化数据，后续可用定时 CI 生成官方差异供维护。支持该快照的宿主一次升级后，新模型和价目可随下次后台拉取生效。 [项目远端配置](../remote-config.json) · [现有远端配置加载](../src-tauri/src/modules/remote_config.rs) · [历史估算任务](../src-tauri/src/modules/codex_local_access_request_logs.rs#L1509) · [OpenAI API 定价](https://developers.openai.com/api/docs/pricing) · [OpenAI 模型列表接口](https://developers.openai.com/api/reference/resources/models/methods/list)

回归覆盖离线安装、运行中刷新、服务和模型管理看到同一版本、默认与自定义 profile、供应商和 Key 范围、别名与快照 ID、272K 输入边界、缓存读写、Fast、Ultrafast 切换前后和历史时间估算。

## 更新与排查

- 安装包自带两款 6.2 的模型定义和 OpenAI API 价表。后续费率写到 `resources/codex/model_prices.json`，递增 `revision`。已知未来费率可写入同一模型的 `subsequentPrices`，时间使用 UTC 毫秒时间戳。旧费率仍可用于历史请求。 [随包目录](../resources/codex/codex_client_models.json) · [官方费率数据](../resources/codex/model_prices.json)
- 模型目录默认跟随现有上游。项目需要自行发布目录时，在远端 `remote-config.json` 将 `codexModelCatalogSource` 设为 `project`，并更新项目的 `resources/codex/codex_client_models.json`。Rust 宿主在后台每小时刷新，API 服务页面也可手动触发。下载失败时继续使用最后一次有效数据。 [远端配置](../remote-config.json) · [目录加载](../src-tauri/src/modules/codex_model_catalog.rs)
- 符合条件的 API 服务受管桌面实例在正常启动时切到 `model_catalog_url`。客户端凭既有 Key 从同一网关读取完整目录。低版本、CLI 启动与用户管理的模型清单使用原有文件。已运行的官方 App Server 按自己的周期拉取动态目录，新列表何时出现在桌面下拉选择器取决于客户端刷新时机。 [官方供应商目录实现](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/model-provider/src/models_endpoint.rs#L214-L221) · [官方定期刷新](https://github.com/openai/codex/blob/rust-v0.169.1/codex-rs/app-server/src/models_refresh_worker.rs#L10)
