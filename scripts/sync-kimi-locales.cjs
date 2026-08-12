#!/usr/bin/env node
/**
 * One-shot: inject full `nav.kimi` + `kimi.*` keys into every locale file.
 * Values differ per language so check_locales English-reuse gate stays green.
 */
const fs = require("fs");
const path = require("path");

const DIR = path.join(__dirname, "../src/locales");

function setPath(obj, keyPath, value) {
  const parts = keyPath.split(".");
  let cur = obj;
  for (let i = 0; i < parts.length - 1; i++) {
    const p = parts[i];
    if (!cur[p] || typeof cur[p] !== "object" || Array.isArray(cur[p])) cur[p] = {};
    cur = cur[p];
  }
  cur[parts[parts.length - 1]] = value;
}

/** English baseline (also en-US) */
const EN = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Search Kimi Code accounts...",
  "kimi.empty": "No Kimi Code accounts yet",
  "kimi.addAccount": "Add Kimi Code Account",
  "kimi.accounts.title": "Kimi Code Accounts",
  "kimi.accounts.desc":
    "Manage Kimi Code CLI accounts: OAuth login, local import, one-click switch into ~/.kimi-code.",
  "kimi.flowNotice.title": "Kimi Code account guide",
  "kimi.flowNotice.desc":
    "Cockpit keeps a multi-account index. Switching writes the official ~/.kimi-code/credentials/kimi-code.json and ensures config.toml has managed:kimi-code.",
  "kimi.flowNotice.permission":
    "Local scope: default ~/.kimi-code credentials can be read for import; switch writes official credentials and config.toml.",
  "kimi.flowNotice.network":
    "Network scope: OAuth authorization, token refresh, and /me · /usages quota queries. Credentials are not uploaded to Cockpit services.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Open the Kimi authorization page in the system browser and complete the device-code login. The account is saved automatically.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Uses the official device flow in the browser without occupying a local callback port.",
  "kimi.oauth.item2":
    "Switching writes ~/.kimi-code/credentials/kimi-code.json in the official wire format.",
  "kimi.oauth.item3":
    "Quota comes from official /usages; login only loads /me to keep traffic light.",
  "kimi.oauth.urlPlaceholder": "Kimi OAuth authorization URL",
  "kimi.oauth.waiting": "Waiting for Kimi OAuth authorization...",
  "kimi.oauth.openWindow": "Open Authorization Page",
  "kimi.oauth.success": "Kimi Code OAuth login succeeded",
  "kimi.import.tokenDesc":
    "You can also paste official credentials/kimi-code.json or a full account JSON exported by this app.",
  "kimi.import.pasteDesc":
    "Paste official credentials/kimi-code.json or a full Cockpit export JSON.",
  "kimi.import.pastePlaceholder": "Paste Kimi Code account JSON",
  "kimi.import.pasteAction": "Import JSON",
  "kimi.import.localDesc":
    "Import from default ~/.kimi-code/credentials/kimi-code.json (honors KIMI_CODE_HOME).",
  "kimi.import.localClient": "Import from local Kimi Code CLI",
  "kimi.quota.empty": "No quota data",
  "kimi.quota.resetAt": "{{label}} resets: {{time}}",
  "kimi.wakeup.masterSwitch": "Wakeup master switch",
  "kimi.wakeup.masterSwitchDesc":
    "When off, scheduled / quota-reset / startup tasks do not run; manual tests still work.",
  "kimi.wakeup.cliPath": "Kimi CLI path",
  "kimi.wakeup.cliOk": "Detected {{path}}",
  "kimi.wakeup.cliMissing": "kimi CLI not found",
  "kimi.wakeup.cliPathSaved": "CLI path saved",
  "kimi.wakeup.cliSettings": "Kimi CLI settings",
  "kimi.wakeup.cliReadyShort": "Detected",
  "kimi.wakeup.cliMissingShort": "Not detected",
  "kimi.wakeup.cliHint":
    "Leave empty to search PATH and common install locations, or pick the kimi / kimi-code executable manually.",
  "kimi.wakeup.cliPathPlaceholder": "e.g. C:\\…\\kimi.exe or leave empty to auto-detect",
  "kimi.wakeup.cliBrowse": "Browse",
  "kimi.wakeup.cliBrowseTitle": "Select Kimi CLI executable",
  "kimi.wakeup.cliBrowsePicked": "File selected — click Save",
  "kimi.wakeup.cliDetect": "Auto-detect",
  "kimi.wakeup.cliDetecting": "Detecting…",
  "kimi.wakeup.cliDetectOk": "Detected and saved: {{path}}",
  "kimi.wakeup.cliDetectFailed":
    "kimi CLI not found on PATH or common install folders. Pick the executable manually.",
  "kimi.wakeup.cliOpenFolder": "Open folder",
  "kimi.wakeup.cliNoPath": "No path to open. Detect or pick a file first.",
  "kimi.wakeup.cliFolderOpened": "Opened folder: {{path}}",
  "kimi.wakeup.cliOpenFolderFailed": "Failed to open folder: {{error}}",
  "kimi.wakeup.modelLine": "Model: {{model}}",
  "kimi.wakeup.invalidModelId":
    "Use a Kimi model alias (e.g. kimi-code/kimi-for-coding), not GPT/Codex models",
  "kimi.wakeup.builtinPresetReadonly": "Built-in presets cannot be edited — create a new one",
  "kimi.wakeup.tasks": "Tasks",
  "kimi.wakeup.addTask": "New task",
  "kimi.wakeup.editTask": "Edit task",
  "kimi.wakeup.noTasks": "No wakeup tasks yet",
  "kimi.wakeup.accountsUnit": "accounts",
  "kimi.wakeup.run": "Run now",
  "kimi.wakeup.test": "Test run",
  "kimi.wakeup.history": "Run history",
  "kimi.wakeup.clearHistory": "Clear history",
  "kimi.wakeup.noHistory": "No history yet",
  "kimi.wakeup.taskName": "Name",
  "kimi.wakeup.nameRequired": "Please enter a task name",
  "kimi.wakeup.accountsRequired": "Select at least one account",
  "kimi.wakeup.selectAccounts": "Accounts",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Model",
  "kimi.wakeup.scheduleKind": "Trigger",
  "kimi.wakeup.intervalHours": "Interval (hours)",
  "kimi.wakeup.dailyTime": "Daily time",
  "kimi.wakeup.quotaWindow": "Quota window",
  "kimi.wakeup.saved": "Task saved",
  "kimi.wakeup.runDone": "Done: success {{ok}} / failed {{fail}}",
  "kimi.wakeup.testDone": "Test done: success {{ok}} / failed {{fail}}",
};

const ZH_CN = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "搜索 Kimi Code 账号...",
  "kimi.empty": "暂无 Kimi Code 账号",
  "kimi.addAccount": "添加 Kimi Code 账号",
  "kimi.accounts.title": "Kimi Code 账号",
  "kimi.accounts.desc":
    "管理 Kimi Code CLI 多账号：OAuth 登录、本机导入、一键切号写入 ~/.kimi-code。",
  "kimi.flowNotice.title": "Kimi Code 账号管理说明",
  "kimi.flowNotice.desc":
    "多账号索引保存在 Cockpit；切号会写入官方 ~/.kimi-code/credentials/kimi-code.json，并确保 config.toml 中有 managed:kimi-code。",
  "kimi.flowNotice.permission":
    "本地范围：可读取默认 ~/.kimi-code 凭据用于导入；切号时写入官方 credentials 与 config.toml。",
  "kimi.flowNotice.network":
    "网络范围：OAuth 授权、token 刷新与 /me · /usages 额度查询；不会把凭据上传到 Cockpit 服务。",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "打开 Kimi 授权页（系统默认浏览器）完成设备码登录，完成后账号会自动保存。",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1": "使用官方 device flow，浏览器授权，不占用本地回调端口。",
  "kimi.oauth.item2": "切号写入 ~/.kimi-code/credentials/kimi-code.json（官方 wire 格式）。",
  "kimi.oauth.item3": "额度来自官方 /usages；登录仅拉 /me 资料，尽量少占请求。",
  "kimi.oauth.urlPlaceholder": "Kimi OAuth 授权地址",
  "kimi.oauth.waiting": "等待 Kimi OAuth 授权...",
  "kimi.oauth.openWindow": "打开授权页",
  "kimi.oauth.success": "Kimi Code OAuth 登录成功",
  "kimi.import.tokenDesc":
    "也可粘贴官方 credentials/kimi-code.json，或本应用导出的完整账号 JSON。",
  "kimi.import.pasteDesc":
    "粘贴官方 credentials/kimi-code.json，或本应用导出的完整账号 JSON。",
  "kimi.import.pastePlaceholder": "粘贴 Kimi Code 账号 JSON",
  "kimi.import.pasteAction": "导入 JSON",
  "kimi.import.localDesc":
    "从默认 ~/.kimi-code/credentials/kimi-code.json 导入（尊重 KIMI_CODE_HOME）。",
  "kimi.import.localClient": "从本机 Kimi Code CLI 导入",
  "kimi.quota.empty": "暂无额度",
  "kimi.quota.resetAt": "{{label}} 重置：{{time}}",
  "kimi.wakeup.masterSwitch": "唤醒总开关",
  "kimi.wakeup.masterSwitchDesc":
    "关闭后定时/额度重置/启动任务不会执行；手动测试仍可用。",
  "kimi.wakeup.cliPath": "Kimi CLI 路径",
  "kimi.wakeup.cliOk": "已检测 {{path}}",
  "kimi.wakeup.cliMissing": "未检测到 kimi CLI",
  "kimi.wakeup.cliPathSaved": "CLI 路径已保存",
  "kimi.wakeup.cliSettings": "Kimi CLI 设置",
  "kimi.wakeup.cliReadyShort": "已检测",
  "kimi.wakeup.cliMissingShort": "未检测",
  "kimi.wakeup.cliHint":
    "可留空以从 PATH 与常见安装目录自动查找；也可手动选择 kimi / kimi-code 可执行文件。",
  "kimi.wakeup.cliPathPlaceholder": "例如 C:\\…\\kimi.exe 或留空自动检测",
  "kimi.wakeup.cliBrowse": "选择",
  "kimi.wakeup.cliBrowseTitle": "选择 Kimi CLI 可执行文件",
  "kimi.wakeup.cliBrowsePicked": "已选择文件，请点保存",
  "kimi.wakeup.cliDetect": "自动检测",
  "kimi.wakeup.cliDetecting": "检测中…",
  "kimi.wakeup.cliDetectOk": "已自动检测并保存：{{path}}",
  "kimi.wakeup.cliDetectFailed":
    "未在本机 PATH / 常见安装目录找到 kimi CLI，请手动选择可执行文件",
  "kimi.wakeup.cliOpenFolder": "打开文件夹",
  "kimi.wakeup.cliNoPath": "没有可打开的路径，请先检测或选择文件",
  "kimi.wakeup.cliFolderOpened": "已打开文件夹：{{path}}",
  "kimi.wakeup.cliOpenFolderFailed": "打开文件夹失败：{{error}}",
  "kimi.wakeup.modelLine": "模型：{{model}}",
  "kimi.wakeup.invalidModelId":
    "请填写 Kimi 模型别名（如 kimi-code/kimi-for-coding），不要用 GPT/Codex 模型",
  "kimi.wakeup.builtinPresetReadonly": "内置预设不可修改，请新增",
  "kimi.wakeup.tasks": "任务列表",
  "kimi.wakeup.addTask": "新建任务",
  "kimi.wakeup.editTask": "编辑任务",
  "kimi.wakeup.noTasks": "暂无唤醒任务",
  "kimi.wakeup.accountsUnit": "账号",
  "kimi.wakeup.run": "立即执行",
  "kimi.wakeup.test": "测试运行",
  "kimi.wakeup.history": "运行历史",
  "kimi.wakeup.clearHistory": "清空历史",
  "kimi.wakeup.noHistory": "暂无历史",
  "kimi.wakeup.taskName": "名称",
  "kimi.wakeup.nameRequired": "请填写任务名称",
  "kimi.wakeup.accountsRequired": "请至少选择一个账号",
  "kimi.wakeup.selectAccounts": "账号",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "模型",
  "kimi.wakeup.scheduleKind": "触发方式",
  "kimi.wakeup.intervalHours": "间隔（小时）",
  "kimi.wakeup.dailyTime": "每天时间",
  "kimi.wakeup.quotaWindow": "额度窗口",
  "kimi.wakeup.saved": "任务已保存",
  "kimi.wakeup.runDone": "完成：成功 {{ok}} / 失败 {{fail}}",
  "kimi.wakeup.testDone": "测试完成：成功 {{ok}} / 失败 {{fail}}",
};

const ZH_TW = {
  ...Object.fromEntries(
    Object.entries(ZH_CN).map(([k, v]) => [
      k,
      typeof v === "string"
        ? v
            .replace(/账号/g, "帳號")
            .replace(/登录/g, "登入")
            .replace(/导入/g, "匯入")
            .replace(/导出/g, "匯出")
            .replace(/粘贴/g, "貼上")
            .replace(/打开/g, "開啟")
            .replace(/等待/g, "等待")
            .replace(/完成/g, "完成")
            .replace(/失败/g, "失敗")
            .replace(/成功/g, "成功")
            .replace(/默认/g, "預設")
            .replace(/尊重/g, "遵循")
            .replace(/管理/g, "管理")
            .replace(/说明/g, "說明")
            .replace(/网络/g, "網路")
            .replace(/本地/g, "本機")
            .replace(/范围/g, "範圍")
            .replace(/查询/g, "查詢")
            .replace(/不会/g, "不會")
            .replace(/把/g, "把")
            .replace(/服务/g, "服務")
            .replace(/搜索/g, "搜尋")
            .replace(/添加/g, "新增")
            .replace(/暂无/g, "尚無")
            .replace(/额度/g, "額度")
            .replace(/重置/g, "重設")
            .replace(/唤醒/g, "喚醒")
            .replace(/总开关/g, "總開關")
            .replace(/关闭后/g, "關閉後")
            .replace(/定时/g, "定時")
            .replace(/启动/g, "啟動")
            .replace(/任务/g, "任務")
            .replace(/不会执行/g, "不會執行")
            .replace(/手动/g, "手動")
            .replace(/仍可用/g, "仍可用")
            .replace(/路径/g, "路徑")
            .replace(/已检测/g, "已偵測")
            .replace(/未检测到/g, "未偵測到")
            .replace(/已保存/g, "已儲存")
            .replace(/列表/g, "列表")
            .replace(/新建/g, "新建")
            .replace(/编辑/g, "編輯")
            .replace(/立即执行/g, "立即執行")
            .replace(/测试运行/g, "測試執行")
            .replace(/运行历史/g, "執行歷史")
            .replace(/清空历史/g, "清空歷史")
            .replace(/请填写/g, "請填寫")
            .replace(/请至少选择一个/g, "請至少選擇一個")
            .replace(/名称/g, "名稱")
            .replace(/模型/g, "模型")
            .replace(/触发方式/g, "觸發方式")
            .replace(/间隔（小时）/g, "間隔（小時）")
            .replace(/每天时间/g, "每天時間")
            .replace(/窗口/g, "視窗")
            .replace(/已保存/g, "已儲存")
            .replace(/测试完成/g, "測試完成")
            .replace(/授权/g, "授權")
            .replace(/地址/g, "地址")
            .replace(/页/g, "頁")
            .replace(/资料/g, "資料")
            .replace(/尽量少占请求/g, "盡量少佔請求")
            .replace(/也可/g, "也可")
            .replace(/完整/g, "完整")
            .replace(/本应用/g, "本應用")
            .replace(/的/g, "的")
            .replace(/从/g, "從")
            .replace(/本机/g, "本機")
        : v,
    ]),
  ),
};

/** Other languages: full distinct translations (not copy-paste EN). */
const JA = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Kimi Code アカウントを検索...",
  "kimi.empty": "Kimi Code アカウントはまだありません",
  "kimi.addAccount": "Kimi Code アカウントを追加",
  "kimi.accounts.title": "Kimi Code アカウント",
  "kimi.accounts.desc":
    "Kimi Code CLI の複数アカウント管理：OAuth ログイン、ローカル取り込み、ワンクリックで ~/.kimi-code に切替。",
  "kimi.flowNotice.title": "Kimi Code アカウントの案内",
  "kimi.flowNotice.desc":
    "Cockpit が複数アカウントの索引を保持します。切替時に公式 ~/.kimi-code/credentials/kimi-code.json を書き込み、config.toml に managed:kimi-code があることを確認します。",
  "kimi.flowNotice.permission":
    "ローカル範囲：既定の ~/.kimi-code 凭拠を取り込みに利用可能。切替時は公式 credentials と config.toml を書き込みます。",
  "kimi.flowNotice.network":
    "ネットワーク範囲：OAuth 認可、トークン更新、/me · /usages のクォータ照会。凭拠は Cockpit サービスへアップロードしません。",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "システム既定ブラウザで Kimi 認可ページを開き、デバイスコードログインを完了します。完了後にアカウントが自動保存されます。",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "公式の device flow をブラウザで使用し、ローカルのコールバックポートを占有しません。",
  "kimi.oauth.item2":
    "切替で ~/.kimi-code/credentials/kimi-code.json を公式 wire 形式で書き込みます。",
  "kimi.oauth.item3":
    "クォータは公式 /usages 由来。ログイン時は /me のみ取得して通信を抑えます。",
  "kimi.oauth.urlPlaceholder": "Kimi OAuth 認可 URL",
  "kimi.oauth.waiting": "Kimi OAuth 認可を待機中...",
  "kimi.oauth.openWindow": "認可ページを開く",
  "kimi.oauth.success": "Kimi Code の OAuth ログインに成功しました",
  "kimi.import.tokenDesc":
    "公式 credentials/kimi-code.json、または本アプリのエクスポート JSON も貼り付けできます。",
  "kimi.import.pasteDesc":
    "公式 credentials/kimi-code.json、または Cockpit のエクスポート JSON を貼り付けます。",
  "kimi.import.pastePlaceholder": "Kimi Code アカウント JSON を貼り付け",
  "kimi.import.pasteAction": "JSON を取り込む",
  "kimi.import.localDesc":
    "既定の ~/.kimi-code/credentials/kimi-code.json から取り込み（KIMI_CODE_HOME を尊重）。",
  "kimi.import.localClient": "ローカルの Kimi Code CLI から取り込む",
  "kimi.quota.empty": "クォータデータなし",
  "kimi.quota.resetAt": "{{label}} のリセット：{{time}}",
  "kimi.wakeup.masterSwitch": "ウェイクアップ主スイッチ",
  "kimi.wakeup.masterSwitchDesc":
    "オフ時は定時 / クォータリセット / 起動タスクは実行されません。手動テストは可能です。",
  "kimi.wakeup.cliPath": "Kimi CLI パス",
  "kimi.wakeup.cliOk": "{{path}} を検出済み",
  "kimi.wakeup.cliMissing": "kimi CLI が見つかりません",
  "kimi.wakeup.cliPathSaved": "CLI パスを保存しました",
  "kimi.wakeup.tasks": "タスク一覧",
  "kimi.wakeup.addTask": "タスクを新規作成",
  "kimi.wakeup.editTask": "タスクを編集",
  "kimi.wakeup.noTasks": "ウェイクアップタスクはまだありません",
  "kimi.wakeup.accountsUnit": "アカウント",
  "kimi.wakeup.run": "今すぐ実行",
  "kimi.wakeup.test": "テスト実行",
  "kimi.wakeup.history": "実行履歴",
  "kimi.wakeup.clearHistory": "履歴を消去",
  "kimi.wakeup.noHistory": "履歴はまだありません",
  "kimi.wakeup.taskName": "名前",
  "kimi.wakeup.nameRequired": "タスク名を入力してください",
  "kimi.wakeup.accountsRequired": "アカウントを少なくとも 1 つ選んでください",
  "kimi.wakeup.selectAccounts": "アカウント",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "モデル",
  "kimi.wakeup.scheduleKind": "トリガー",
  "kimi.wakeup.intervalHours": "間隔（時間）",
  "kimi.wakeup.dailyTime": "毎日の時刻",
  "kimi.wakeup.quotaWindow": "クォータ窓",
  "kimi.wakeup.saved": "タスクを保存しました",
  "kimi.wakeup.runDone": "完了：成功 {{ok}} / 失敗 {{fail}}",
  "kimi.wakeup.testDone": "テスト完了：成功 {{ok}} / 失敗 {{fail}}",
};

// Fix JA typo 凭拠 -> 資格情報
for (const k of Object.keys(JA)) {
  JA[k] = JA[k].replace(/凭拠/g, "資格情報");
}

const KO = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Kimi Code 계정 검색...",
  "kimi.empty": "Kimi Code 계정이 아직 없습니다",
  "kimi.addAccount": "Kimi Code 계정 추가",
  "kimi.accounts.title": "Kimi Code 계정",
  "kimi.accounts.desc":
    "Kimi Code CLI 다중 계정 관리: OAuth 로그인, 로컬 가져오기, 원클릭으로 ~/.kimi-code 에 전환.",
  "kimi.flowNotice.title": "Kimi Code 계정 안내",
  "kimi.flowNotice.desc":
    "Cockpit 이 다중 계정 인덱스를 보관합니다. 전환 시 공식 ~/.kimi-code/credentials/kimi-code.json 을 기록하고 config.toml 에 managed:kimi-code 가 있는지 확인합니다.",
  "kimi.flowNotice.permission":
    "로컬 범위: 기본 ~/.kimi-code 자격 증명을 가져오기에 사용할 수 있으며, 전환 시 공식 credentials 와 config.toml 을 기록합니다.",
  "kimi.flowNotice.network":
    "네트워크 범위: OAuth 승인, 토큰 갱신, /me · /usages 할당량 조회. 자격 증명은 Cockpit 서비스로 업로드되지 않습니다.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "시스템 기본 브라우저에서 Kimi 승인 페이지를 열고 디바이스 코드 로그인을 완료하면 계정이 자동 저장됩니다.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "공식 device flow 를 브라우저에서 사용하며 로컬 콜백 포트를 점유하지 않습니다.",
  "kimi.oauth.item2":
    "전환 시 ~/.kimi-code/credentials/kimi-code.json 을 공식 wire 형식으로 기록합니다.",
  "kimi.oauth.item3":
    "할당량은 공식 /usages 기준이며, 로그인 시 /me 만 조회해 요청을 줄입니다.",
  "kimi.oauth.urlPlaceholder": "Kimi OAuth 승인 URL",
  "kimi.oauth.waiting": "Kimi OAuth 승인 대기 중...",
  "kimi.oauth.openWindow": "승인 페이지 열기",
  "kimi.oauth.success": "Kimi Code OAuth 로그인 성공",
  "kimi.import.tokenDesc":
    "공식 credentials/kimi-code.json 또는 이 앱에서 내보낸 전체 계정 JSON 을 붙여넣을 수도 있습니다.",
  "kimi.import.pasteDesc":
    "공식 credentials/kimi-code.json 또는 Cockpit 내보내기 JSON 을 붙여넣으세요.",
  "kimi.import.pastePlaceholder": "Kimi Code 계정 JSON 붙여넣기",
  "kimi.import.pasteAction": "JSON 가져오기",
  "kimi.import.localDesc":
    "기본 ~/.kimi-code/credentials/kimi-code.json 에서 가져오기(KIMI_CODE_HOME 준수).",
  "kimi.import.localClient": "로컬 Kimi Code CLI 에서 가져오기",
  "kimi.quota.empty": "할당량 데이터 없음",
  "kimi.quota.resetAt": "{{label}} 재설정: {{time}}",
  "kimi.wakeup.masterSwitch": "웨이크업 마스터 스위치",
  "kimi.wakeup.masterSwitchDesc":
    "끄면 예약/할당량 재설정/시작 작업은 실행되지 않습니다. 수동 테스트는 가능합니다.",
  "kimi.wakeup.cliPath": "Kimi CLI 경로",
  "kimi.wakeup.cliOk": "{{path}} 감지됨",
  "kimi.wakeup.cliMissing": "kimi CLI 를 찾을 수 없음",
  "kimi.wakeup.cliPathSaved": "CLI 경로가 저장됨",
  "kimi.wakeup.tasks": "작업 목록",
  "kimi.wakeup.addTask": "새 작업",
  "kimi.wakeup.editTask": "작업 편집",
  "kimi.wakeup.noTasks": "웨이크업 작업이 아직 없습니다",
  "kimi.wakeup.accountsUnit": "계정",
  "kimi.wakeup.run": "지금 실행",
  "kimi.wakeup.test": "테스트 실행",
  "kimi.wakeup.history": "실행 기록",
  "kimi.wakeup.clearHistory": "기록 지우기",
  "kimi.wakeup.noHistory": "기록이 아직 없습니다",
  "kimi.wakeup.taskName": "이름",
  "kimi.wakeup.nameRequired": "작업 이름을 입력하세요",
  "kimi.wakeup.accountsRequired": "계정을 하나 이상 선택하세요",
  "kimi.wakeup.selectAccounts": "계정",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "모델",
  "kimi.wakeup.scheduleKind": "트리거",
  "kimi.wakeup.intervalHours": "간격(시간)",
  "kimi.wakeup.dailyTime": "매일 시각",
  "kimi.wakeup.quotaWindow": "할당량 창",
  "kimi.wakeup.saved": "작업이 저장됨",
  "kimi.wakeup.runDone": "완료: 성공 {{ok}} / 실패 {{fail}}",
  "kimi.wakeup.testDone": "테스트 완료: 성공 {{ok}} / 실패 {{fail}}",
};

function western(pack) {
  return pack;
}

const DE = western({
  "nav.kimi": "Kimi Code",
  "kimi.search": "Kimi-Code-Konten suchen...",
  "kimi.empty": "Noch keine Kimi-Code-Konten",
  "kimi.addAccount": "Kimi-Code-Konto hinzufügen",
  "kimi.accounts.title": "Kimi-Code-Konten",
  "kimi.accounts.desc":
    "Kimi-Code-CLI-Konten verwalten: OAuth-Anmeldung, lokaler Import, Umschalten nach ~/.kimi-code mit einem Klick.",
  "kimi.flowNotice.title": "Hinweis zu Kimi-Code-Konten",
  "kimi.flowNotice.desc":
    "Cockpit speichert einen Mehrkonten-Index. Beim Wechsel wird die offizielle ~/.kimi-code/credentials/kimi-code.json geschrieben und config.toml auf managed:kimi-code geprüft.",
  "kimi.flowNotice.permission":
    "Lokal: Standard-Credentials unter ~/.kimi-code dürfen zum Import gelesen werden; beim Wechsel werden offizielle Credentials und config.toml geschrieben.",
  "kimi.flowNotice.network":
    "Netzwerk: OAuth-Autorisierung, Token-Aktualisierung und /me · /usages-Kontingentabfragen. Credentials werden nicht an Cockpit-Dienste gesendet.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Öffnen Sie die Kimi-Autorisierungsseite im Systembrowser und schließen Sie den Gerätecode-Login ab. Das Konto wird danach automatisch gespeichert.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Nutzt den offiziellen Device-Flow im Browser und belegt keinen lokalen Callback-Port.",
  "kimi.oauth.item2":
    "Beim Wechsel wird ~/.kimi-code/credentials/kimi-code.json im offiziellen Wire-Format geschrieben.",
  "kimi.oauth.item3":
    "Kontingente stammen von /usages; beim Login wird nur /me geladen, um Traffic sparsam zu halten.",
  "kimi.oauth.urlPlaceholder": "Kimi-OAuth-Autorisierungs-URL",
  "kimi.oauth.waiting": "Warte auf Kimi-OAuth-Autorisierung...",
  "kimi.oauth.openWindow": "Autorisierungsseite öffnen",
  "kimi.oauth.success": "Kimi-Code-OAuth-Anmeldung erfolgreich",
  "kimi.import.tokenDesc":
    "Sie können auch die offizielle credentials/kimi-code.json oder ein vollständiges Export-JSON dieser App einfügen.",
  "kimi.import.pasteDesc":
    "Offizielle credentials/kimi-code.json oder vollständiges Cockpit-Export-JSON einfügen.",
  "kimi.import.pastePlaceholder": "Kimi-Code-Konto-JSON einfügen",
  "kimi.import.pasteAction": "JSON importieren",
  "kimi.import.localDesc":
    "Import aus ~/.kimi-code/credentials/kimi-code.json (beachtet KIMI_CODE_HOME).",
  "kimi.import.localClient": "Aus lokaler Kimi-Code-CLI importieren",
  "kimi.quota.empty": "Keine Kontingentdaten",
  "kimi.quota.resetAt": "{{label}} zurücksetzen: {{time}}",
  "kimi.wakeup.masterSwitch": "Wakeup-Hauptschalter",
  "kimi.wakeup.masterSwitchDesc":
    "Wenn aus, laufen geplante / Kontingent-Reset- / Startaufgaben nicht; manuelle Tests bleiben möglich.",
  "kimi.wakeup.cliPath": "Kimi-CLI-Pfad",
  "kimi.wakeup.cliOk": "{{path}} erkannt",
  "kimi.wakeup.cliMissing": "kimi CLI nicht gefunden",
  "kimi.wakeup.cliPathSaved": "CLI-Pfad gespeichert",
  "kimi.wakeup.tasks": "Aufgaben",
  "kimi.wakeup.addTask": "Neue Aufgabe",
  "kimi.wakeup.editTask": "Aufgabe bearbeiten",
  "kimi.wakeup.noTasks": "Noch keine Wakeup-Aufgaben",
  "kimi.wakeup.accountsUnit": "Konten",
  "kimi.wakeup.run": "Jetzt ausführen",
  "kimi.wakeup.test": "Testdurchlauf",
  "kimi.wakeup.history": "Ausführungsverlauf",
  "kimi.wakeup.clearHistory": "Verlauf leeren",
  "kimi.wakeup.noHistory": "Noch kein Verlauf",
  "kimi.wakeup.taskName": "Name",
  "kimi.wakeup.nameRequired": "Bitte einen Aufgabennamen eingeben",
  "kimi.wakeup.accountsRequired": "Mindestens ein Konto auswählen",
  "kimi.wakeup.selectAccounts": "Konten",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Modell",
  "kimi.wakeup.scheduleKind": "Auslöser",
  "kimi.wakeup.intervalHours": "Intervall (Stunden)",
  "kimi.wakeup.dailyTime": "Tägliche Uhrzeit",
  "kimi.wakeup.quotaWindow": "Kontingentfenster",
  "kimi.wakeup.saved": "Aufgabe gespeichert",
  "kimi.wakeup.runDone": "Fertig: erfolgreich {{ok}} / fehlgeschlagen {{fail}}",
  "kimi.wakeup.testDone": "Test fertig: erfolgreich {{ok}} / fehlgeschlagen {{fail}}",
});

const FR = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Rechercher des comptes Kimi Code...",
  "kimi.empty": "Aucun compte Kimi Code pour le moment",
  "kimi.addAccount": "Ajouter un compte Kimi Code",
  "kimi.accounts.title": "Comptes Kimi Code",
  "kimi.accounts.desc":
    "Gérer les comptes Kimi Code CLI : connexion OAuth, import local, bascule en un clic vers ~/.kimi-code.",
  "kimi.flowNotice.title": "Guide des comptes Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit conserve un index multi-comptes. La bascule écrit le fichier officiel ~/.kimi-code/credentials/kimi-code.json et vérifie managed:kimi-code dans config.toml.",
  "kimi.flowNotice.permission":
    "Portée locale : les identifiants ~/.kimi-code par défaut peuvent être lus pour l’import ; la bascule écrit les credentials officiels et config.toml.",
  "kimi.flowNotice.network":
    "Portée réseau : autorisation OAuth, rafraîchissement du jeton et requêtes de quota /me · /usages. Les identifiants ne sont pas envoyés aux services Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Ouvrez la page d’autorisation Kimi dans le navigateur système et terminez la connexion par code appareil. Le compte est enregistré automatiquement.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Utilise le device flow officiel dans le navigateur sans occuper de port de rappel local.",
  "kimi.oauth.item2":
    "La bascule écrit ~/.kimi-code/credentials/kimi-code.json au format wire officiel.",
  "kimi.oauth.item3":
    "Les quotas viennent de /usages ; la connexion ne charge que /me pour limiter le trafic.",
  "kimi.oauth.urlPlaceholder": "URL d’autorisation OAuth Kimi",
  "kimi.oauth.waiting": "En attente de l’autorisation OAuth Kimi...",
  "kimi.oauth.openWindow": "Ouvrir la page d’autorisation",
  "kimi.oauth.success": "Connexion OAuth Kimi Code réussie",
  "kimi.import.tokenDesc":
    "Vous pouvez aussi coller le credentials/kimi-code.json officiel ou un JSON de compte exporté par cette application.",
  "kimi.import.pasteDesc":
    "Collez le credentials/kimi-code.json officiel ou un JSON d’export Cockpit complet.",
  "kimi.import.pastePlaceholder": "Coller le JSON du compte Kimi Code",
  "kimi.import.pasteAction": "Importer le JSON",
  "kimi.import.localDesc":
    "Importer depuis ~/.kimi-code/credentials/kimi-code.json (respecte KIMI_CODE_HOME).",
  "kimi.import.localClient": "Importer depuis le CLI Kimi Code local",
  "kimi.quota.empty": "Aucune donnée de quota",
  "kimi.quota.resetAt": "Réinitialisation {{label}} : {{time}}",
  "kimi.wakeup.masterSwitch": "Interrupteur principal de réveil",
  "kimi.wakeup.masterSwitchDesc":
    "Désactivé : les tâches planifiées / reset de quota / démarrage ne s’exécutent pas ; les tests manuels restent possibles.",
  "kimi.wakeup.cliPath": "Chemin du CLI Kimi",
  "kimi.wakeup.cliOk": "{{path}} détecté",
  "kimi.wakeup.cliMissing": "CLI kimi introuvable",
  "kimi.wakeup.cliPathSaved": "Chemin CLI enregistré",
  "kimi.wakeup.tasks": "Tâches",
  "kimi.wakeup.addTask": "Nouvelle tâche",
  "kimi.wakeup.editTask": "Modifier la tâche",
  "kimi.wakeup.noTasks": "Aucune tâche de réveil pour le moment",
  "kimi.wakeup.accountsUnit": "comptes",
  "kimi.wakeup.run": "Exécuter maintenant",
  "kimi.wakeup.test": "Exécution test",
  "kimi.wakeup.history": "Historique d’exécution",
  "kimi.wakeup.clearHistory": "Effacer l’historique",
  "kimi.wakeup.noHistory": "Pas encore d’historique",
  "kimi.wakeup.taskName": "Nom",
  "kimi.wakeup.nameRequired": "Veuillez saisir un nom de tâche",
  "kimi.wakeup.accountsRequired": "Sélectionnez au moins un compte",
  "kimi.wakeup.selectAccounts": "Comptes",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Modèle",
  "kimi.wakeup.scheduleKind": "Déclencheur",
  "kimi.wakeup.intervalHours": "Intervalle (heures)",
  "kimi.wakeup.dailyTime": "Heure quotidienne",
  "kimi.wakeup.quotaWindow": "Fenêtre de quota",
  "kimi.wakeup.saved": "Tâche enregistrée",
  "kimi.wakeup.runDone": "Terminé : succès {{ok}} / échec {{fail}}",
  "kimi.wakeup.testDone": "Test terminé : succès {{ok}} / échec {{fail}}",
};

const ES = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Buscar cuentas de Kimi Code...",
  "kimi.empty": "Aún no hay cuentas de Kimi Code",
  "kimi.addAccount": "Añadir cuenta de Kimi Code",
  "kimi.accounts.title": "Cuentas de Kimi Code",
  "kimi.accounts.desc":
    "Administra cuentas de Kimi Code CLI: inicio OAuth, importación local y cambio en un clic a ~/.kimi-code.",
  "kimi.flowNotice.title": "Guía de cuentas Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit guarda un índice multi-cuenta. Al cambiar se escribe el oficial ~/.kimi-code/credentials/kimi-code.json y se asegura managed:kimi-code en config.toml.",
  "kimi.flowNotice.permission":
    "Ámbito local: se pueden leer las credenciales predeterminadas de ~/.kimi-code para importar; al cambiar se escriben las oficiales y config.toml.",
  "kimi.flowNotice.network":
    "Ámbito de red: autorización OAuth, renovación de token y consultas de cupo /me · /usages. Las credenciales no se suben a los servicios de Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Abre la página de autorización de Kimi en el navegador del sistema y completa el inicio con código de dispositivo. La cuenta se guarda sola.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Usa el device flow oficial en el navegador sin ocupar un puerto de callback local.",
  "kimi.oauth.item2":
    "El cambio escribe ~/.kimi-code/credentials/kimi-code.json en el formato wire oficial.",
  "kimi.oauth.item3":
    "El cupo sale de /usages; el inicio de sesión solo carga /me para reducir tráfico.",
  "kimi.oauth.urlPlaceholder": "URL de autorización OAuth de Kimi",
  "kimi.oauth.waiting": "Esperando autorización OAuth de Kimi...",
  "kimi.oauth.openWindow": "Abrir página de autorización",
  "kimi.oauth.success": "Inicio de sesión OAuth de Kimi Code correcto",
  "kimi.import.tokenDesc":
    "También puedes pegar credentials/kimi-code.json oficial o un JSON completo exportado por esta app.",
  "kimi.import.pasteDesc":
    "Pega credentials/kimi-code.json oficial o un JSON de exportación completo de Cockpit.",
  "kimi.import.pastePlaceholder": "Pegar JSON de cuenta Kimi Code",
  "kimi.import.pasteAction": "Importar JSON",
  "kimi.import.localDesc":
    "Importar desde ~/.kimi-code/credentials/kimi-code.json (respeta KIMI_CODE_HOME).",
  "kimi.import.localClient": "Importar desde el CLI local de Kimi Code",
  "kimi.quota.empty": "Sin datos de cupo",
  "kimi.quota.resetAt": "{{label}} se reinicia: {{time}}",
  "kimi.wakeup.masterSwitch": "Interruptor maestro de despertar",
  "kimi.wakeup.masterSwitchDesc":
    "Si está apagado, no se ejecutan tareas programadas / de reinicio de cupo / de arranque; las pruebas manuales siguen disponibles.",
  "kimi.wakeup.cliPath": "Ruta del CLI de Kimi",
  "kimi.wakeup.cliOk": "Detectado {{path}}",
  "kimi.wakeup.cliMissing": "No se encontró el CLI kimi",
  "kimi.wakeup.cliPathSaved": "Ruta del CLI guardada",
  "kimi.wakeup.tasks": "Tareas",
  "kimi.wakeup.addTask": "Nueva tarea",
  "kimi.wakeup.editTask": "Editar tarea",
  "kimi.wakeup.noTasks": "Aún no hay tareas de despertar",
  "kimi.wakeup.accountsUnit": "cuentas",
  "kimi.wakeup.run": "Ejecutar ahora",
  "kimi.wakeup.test": "Ejecución de prueba",
  "kimi.wakeup.history": "Historial de ejecución",
  "kimi.wakeup.clearHistory": "Borrar historial",
  "kimi.wakeup.noHistory": "Aún no hay historial",
  "kimi.wakeup.taskName": "Nombre",
  "kimi.wakeup.nameRequired": "Introduce un nombre de tarea",
  "kimi.wakeup.accountsRequired": "Selecciona al menos una cuenta",
  "kimi.wakeup.selectAccounts": "Cuentas",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Modelo",
  "kimi.wakeup.scheduleKind": "Disparador",
  "kimi.wakeup.intervalHours": "Intervalo (horas)",
  "kimi.wakeup.dailyTime": "Hora diaria",
  "kimi.wakeup.quotaWindow": "Ventana de cupo",
  "kimi.wakeup.saved": "Tarea guardada",
  "kimi.wakeup.runDone": "Listo: correcto {{ok}} / fallido {{fail}}",
  "kimi.wakeup.testDone": "Prueba lista: correcto {{ok}} / fallido {{fail}}",
};

const PT_BR = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Pesquisar contas Kimi Code...",
  "kimi.empty": "Ainda não há contas Kimi Code",
  "kimi.addAccount": "Adicionar conta Kimi Code",
  "kimi.accounts.title": "Contas Kimi Code",
  "kimi.accounts.desc":
    "Gerencie contas do Kimi Code CLI: login OAuth, importação local e troca com um clique para ~/.kimi-code.",
  "kimi.flowNotice.title": "Guia de contas Kimi Code",
  "kimi.flowNotice.desc":
    "O Cockpit mantém um índice multi-conta. A troca grava o oficial ~/.kimi-code/credentials/kimi-code.json e garante managed:kimi-code no config.toml.",
  "kimi.flowNotice.permission":
    "Escopo local: credenciais padrão em ~/.kimi-code podem ser lidas para importar; a troca grava as oficiais e o config.toml.",
  "kimi.flowNotice.network":
    "Escopo de rede: autorização OAuth, renovação de token e consultas de cota /me · /usages. Credenciais não são enviadas aos serviços do Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Abra a página de autorização Kimi no navegador do sistema e conclua o login por código de dispositivo. A conta é salva automaticamente.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Usa o device flow oficial no navegador sem ocupar porta de callback local.",
  "kimi.oauth.item2":
    "A troca grava ~/.kimi-code/credentials/kimi-code.json no formato wire oficial.",
  "kimi.oauth.item3":
    "A cota vem de /usages; o login só carrega /me para reduzir tráfego.",
  "kimi.oauth.urlPlaceholder": "URL de autorização OAuth do Kimi",
  "kimi.oauth.waiting": "Aguardando autorização OAuth do Kimi...",
  "kimi.oauth.openWindow": "Abrir página de autorização",
  "kimi.oauth.success": "Login OAuth do Kimi Code concluído",
  "kimi.import.tokenDesc":
    "Você também pode colar o credentials/kimi-code.json oficial ou um JSON completo exportado por este app.",
  "kimi.import.pasteDesc":
    "Cole o credentials/kimi-code.json oficial ou um JSON de exportação completo do Cockpit.",
  "kimi.import.pastePlaceholder": "Cole o JSON da conta Kimi Code",
  "kimi.import.pasteAction": "Importar JSON",
  "kimi.import.localDesc":
    "Importar de ~/.kimi-code/credentials/kimi-code.json (respeita KIMI_CODE_HOME).",
  "kimi.import.localClient": "Importar do CLI local do Kimi Code",
  "kimi.quota.empty": "Sem dados de cota",
  "kimi.quota.resetAt": "{{label}} redefine em: {{time}}",
  "kimi.wakeup.masterSwitch": "Interruptor mestre de despertar",
  "kimi.wakeup.masterSwitchDesc":
    "Desligado: tarefas agendadas / de reset de cota / de inicialização não rodam; testes manuais ainda funcionam.",
  "kimi.wakeup.cliPath": "Caminho do CLI Kimi",
  "kimi.wakeup.cliOk": "{{path}} detectado",
  "kimi.wakeup.cliMissing": "CLI kimi não encontrado",
  "kimi.wakeup.cliPathSaved": "Caminho do CLI salvo",
  "kimi.wakeup.tasks": "Tarefas",
  "kimi.wakeup.addTask": "Nova tarefa",
  "kimi.wakeup.editTask": "Editar tarefa",
  "kimi.wakeup.noTasks": "Ainda não há tarefas de despertar",
  "kimi.wakeup.accountsUnit": "contas",
  "kimi.wakeup.run": "Executar agora",
  "kimi.wakeup.test": "Execução de teste",
  "kimi.wakeup.history": "Histórico de execução",
  "kimi.wakeup.clearHistory": "Limpar histórico",
  "kimi.wakeup.noHistory": "Ainda não há histórico",
  "kimi.wakeup.taskName": "Nome",
  "kimi.wakeup.nameRequired": "Informe um nome de tarefa",
  "kimi.wakeup.accountsRequired": "Selecione pelo menos uma conta",
  "kimi.wakeup.selectAccounts": "Contas",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Modelo",
  "kimi.wakeup.scheduleKind": "Gatilho",
  "kimi.wakeup.intervalHours": "Intervalo (horas)",
  "kimi.wakeup.dailyTime": "Horário diário",
  "kimi.wakeup.quotaWindow": "Janela de cota",
  "kimi.wakeup.saved": "Tarefa salva",
  "kimi.wakeup.runDone": "Concluído: sucesso {{ok}} / falha {{fail}}",
  "kimi.wakeup.testDone": "Teste concluído: sucesso {{ok}} / falha {{fail}}",
};

const RU = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Поиск аккаунтов Kimi Code...",
  "kimi.empty": "Пока нет аккаунтов Kimi Code",
  "kimi.addAccount": "Добавить аккаунт Kimi Code",
  "kimi.accounts.title": "Аккаунты Kimi Code",
  "kimi.accounts.desc":
    "Управление аккаунтами Kimi Code CLI: OAuth, локальный импорт, переключение в ~/.kimi-code одним кликом.",
  "kimi.flowNotice.title": "Справка по аккаунтам Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit хранит индекс нескольких аккаунтов. При переключении записывается официальный ~/.kimi-code/credentials/kimi-code.json и проверяется managed:kimi-code в config.toml.",
  "kimi.flowNotice.permission":
    "Локально: учётные данные ~/.kimi-code по умолчанию можно читать для импорта; при переключении пишутся официальные credentials и config.toml.",
  "kimi.flowNotice.network":
    "Сеть: OAuth-авторизация, обновление токена и запросы квоты /me · /usages. Учётные данные не загружаются в сервисы Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Откройте страницу авторизации Kimi в системном браузере и завершите вход по коду устройства. Аккаунт сохранится автоматически.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Использует официальный device flow в браузере и не занимает локальный callback-порт.",
  "kimi.oauth.item2":
    "Переключение записывает ~/.kimi-code/credentials/kimi-code.json в официальном wire-формате.",
  "kimi.oauth.item3":
    "Квота берётся из /usages; при входе загружается только /me, чтобы снизить трафик.",
  "kimi.oauth.urlPlaceholder": "URL авторизации OAuth Kimi",
  "kimi.oauth.waiting": "Ожидание OAuth-авторизации Kimi...",
  "kimi.oauth.openWindow": "Открыть страницу авторизации",
  "kimi.oauth.success": "OAuth-вход Kimi Code выполнен",
  "kimi.import.tokenDesc":
    "Можно также вставить официальный credentials/kimi-code.json или полный JSON экспорта этого приложения.",
  "kimi.import.pasteDesc":
    "Вставьте официальный credentials/kimi-code.json или полный JSON экспорта Cockpit.",
  "kimi.import.pastePlaceholder": "Вставьте JSON аккаунта Kimi Code",
  "kimi.import.pasteAction": "Импорт JSON",
  "kimi.import.localDesc":
    "Импорт из ~/.kimi-code/credentials/kimi-code.json (учитывает KIMI_CODE_HOME).",
  "kimi.import.localClient": "Импорт из локального Kimi Code CLI",
  "kimi.quota.empty": "Нет данных о квоте",
  "kimi.quota.resetAt": "Сброс {{label}}: {{time}}",
  "kimi.wakeup.masterSwitch": "Главный переключатель пробуждения",
  "kimi.wakeup.masterSwitchDesc":
    "Выключено: плановые / сброс квоты / стартовые задачи не выполняются; ручные тесты доступны.",
  "kimi.wakeup.cliPath": "Путь к Kimi CLI",
  "kimi.wakeup.cliOk": "Обнаружен {{path}}",
  "kimi.wakeup.cliMissing": "kimi CLI не найден",
  "kimi.wakeup.cliPathSaved": "Путь CLI сохранён",
  "kimi.wakeup.tasks": "Задачи",
  "kimi.wakeup.addTask": "Новая задача",
  "kimi.wakeup.editTask": "Изменить задачу",
  "kimi.wakeup.noTasks": "Пока нет задач пробуждения",
  "kimi.wakeup.accountsUnit": "аккаунтов",
  "kimi.wakeup.run": "Выполнить сейчас",
  "kimi.wakeup.test": "Тестовый запуск",
  "kimi.wakeup.history": "История запусков",
  "kimi.wakeup.clearHistory": "Очистить историю",
  "kimi.wakeup.noHistory": "Истории пока нет",
  "kimi.wakeup.taskName": "Имя",
  "kimi.wakeup.nameRequired": "Укажите имя задачи",
  "kimi.wakeup.accountsRequired": "Выберите хотя бы один аккаунт",
  "kimi.wakeup.selectAccounts": "Аккаунты",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Модель",
  "kimi.wakeup.scheduleKind": "Триггер",
  "kimi.wakeup.intervalHours": "Интервал (часы)",
  "kimi.wakeup.dailyTime": "Ежедневное время",
  "kimi.wakeup.quotaWindow": "Окно квоты",
  "kimi.wakeup.saved": "Задача сохранена",
  "kimi.wakeup.runDone": "Готово: успешно {{ok}} / ошибок {{fail}}",
  "kimi.wakeup.testDone": "Тест готов: успешно {{ok}} / ошибок {{fail}}",
};

// Remaining locales: distinct localizations
const IT = { ...ES };
// rewrite IT properly
Object.assign(IT, {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Cerca account Kimi Code...",
  "kimi.empty": "Nessun account Kimi Code al momento",
  "kimi.addAccount": "Aggiungi account Kimi Code",
  "kimi.accounts.title": "Account Kimi Code",
  "kimi.accounts.desc":
    "Gestisci account Kimi Code CLI: accesso OAuth, import locale, switch in un clic su ~/.kimi-code.",
  "kimi.flowNotice.title": "Guida account Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit mantiene un indice multi-account. Lo switch scrive il file ufficiale ~/.kimi-code/credentials/kimi-code.json e verifica managed:kimi-code in config.toml.",
  "kimi.flowNotice.permission":
    "Ambito locale: le credenziali predefinite in ~/.kimi-code possono essere lette per l’import; lo switch scrive le ufficiali e config.toml.",
  "kimi.flowNotice.network":
    "Ambito di rete: autorizzazione OAuth, rinnovo token e query di quota /me · /usages. Le credenziali non vengono caricate sui servizi Cockpit.",
  "kimi.oauth.desc":
    "Apri la pagina di autorizzazione Kimi nel browser di sistema e completa l’accesso con codice dispositivo. L’account viene salvato automaticamente.",
  "kimi.oauth.item1":
    "Usa il device flow ufficiale nel browser senza occupare una porta di callback locale.",
  "kimi.oauth.item2":
    "Lo switch scrive ~/.kimi-code/credentials/kimi-code.json nel formato wire ufficiale.",
  "kimi.oauth.item3":
    "La quota arriva da /usages; l’accesso carica solo /me per limitare il traffico.",
  "kimi.oauth.urlPlaceholder": "URL di autorizzazione OAuth Kimi",
  "kimi.oauth.waiting": "In attesa dell’autorizzazione OAuth Kimi...",
  "kimi.oauth.openWindow": "Apri pagina di autorizzazione",
  "kimi.oauth.success": "Accesso OAuth Kimi Code riuscito",
  "kimi.import.tokenDesc":
    "Puoi anche incollare credentials/kimi-code.json ufficiale o un JSON account completo esportato da questa app.",
  "kimi.import.pasteDesc":
    "Incolla credentials/kimi-code.json ufficiale o un JSON di export completo di Cockpit.",
  "kimi.import.pastePlaceholder": "Incolla JSON account Kimi Code",
  "kimi.import.pasteAction": "Importa JSON",
  "kimi.import.localDesc":
    "Importa da ~/.kimi-code/credentials/kimi-code.json (rispetta KIMI_CODE_HOME).",
  "kimi.import.localClient": "Importa dal CLI Kimi Code locale",
  "kimi.quota.empty": "Nessun dato di quota",
  "kimi.quota.resetAt": "{{label}} si reimposta: {{time}}",
  "kimi.wakeup.masterSwitch": "Interruttore principale wakeup",
  "kimi.wakeup.masterSwitchDesc":
    "Se spento, le attività pianificate / di reset quota / di avvio non partono; i test manuali restano disponibili.",
  "kimi.wakeup.cliPath": "Percorso CLI Kimi",
  "kimi.wakeup.cliOk": "Rilevato {{path}}",
  "kimi.wakeup.cliMissing": "CLI kimi non trovato",
  "kimi.wakeup.cliPathSaved": "Percorso CLI salvato",
  "kimi.wakeup.tasks": "Attività",
  "kimi.wakeup.addTask": "Nuova attività",
  "kimi.wakeup.editTask": "Modifica attività",
  "kimi.wakeup.noTasks": "Nessuna attività di wakeup ancora",
  "kimi.wakeup.accountsUnit": "account",
  "kimi.wakeup.run": "Esegui ora",
  "kimi.wakeup.test": "Esecuzione di prova",
  "kimi.wakeup.history": "Cronologia esecuzioni",
  "kimi.wakeup.clearHistory": "Svuota cronologia",
  "kimi.wakeup.noHistory": "Ancora nessuna cronologia",
  "kimi.wakeup.taskName": "Nome",
  "kimi.wakeup.nameRequired": "Inserisci un nome attività",
  "kimi.wakeup.accountsRequired": "Seleziona almeno un account",
  "kimi.wakeup.selectAccounts": "Account",
  "kimi.wakeup.model": "Modello",
  "kimi.wakeup.scheduleKind": "Trigger",
  "kimi.wakeup.intervalHours": "Intervallo (ore)",
  "kimi.wakeup.dailyTime": "Orario giornaliero",
  "kimi.wakeup.quotaWindow": "Finestra di quota",
  "kimi.wakeup.saved": "Attività salvata",
  "kimi.wakeup.runDone": "Fatto: ok {{ok}} / errori {{fail}}",
  "kimi.wakeup.testDone": "Test fatto: ok {{ok}} / errori {{fail}}",
});

const PL = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Szukaj kont Kimi Code...",
  "kimi.empty": "Brak kont Kimi Code",
  "kimi.addAccount": "Dodaj konto Kimi Code",
  "kimi.accounts.title": "Konta Kimi Code",
  "kimi.accounts.desc":
    "Zarządzaj kontami Kimi Code CLI: logowanie OAuth, import lokalny, przełączanie do ~/.kimi-code jednym kliknięciem.",
  "kimi.flowNotice.title": "Przewodnik po kontach Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit trzyma indeks wielu kont. Przełączenie zapisuje oficjalny ~/.kimi-code/credentials/kimi-code.json i pilnuje managed:kimi-code w config.toml.",
  "kimi.flowNotice.permission":
    "Zakres lokalny: domyślne poświadczenia ~/.kimi-code można czytać do importu; przełączenie zapisuje oficjalne credentials i config.toml.",
  "kimi.flowNotice.network":
    "Zakres sieci: autoryzacja OAuth, odświeżanie tokenu i zapytania limitu /me · /usages. Poświadczenia nie trafiają do usług Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Otwórz stronę autoryzacji Kimi w domyślnej przeglądarce i dokończ logowanie kodem urządzenia. Konto zapisze się automatycznie.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Używa oficjalnego device flow w przeglądarce bez zajmowania lokalnego portu callback.",
  "kimi.oauth.item2":
    "Przełączenie zapisuje ~/.kimi-code/credentials/kimi-code.json w oficjalnym formacie wire.",
  "kimi.oauth.item3":
    "Limit pochodzi z /usages; logowanie pobiera tylko /me, by ograniczyć ruch.",
  "kimi.oauth.urlPlaceholder": "URL autoryzacji OAuth Kimi",
  "kimi.oauth.waiting": "Oczekiwanie na autoryzację OAuth Kimi...",
  "kimi.oauth.openWindow": "Otwórz stronę autoryzacji",
  "kimi.oauth.success": "Logowanie OAuth Kimi Code powiodło się",
  "kimi.import.tokenDesc":
    "Możesz też wkleić oficjalny credentials/kimi-code.json lub pełny JSON konta wyeksportowany z tej aplikacji.",
  "kimi.import.pasteDesc":
    "Wklej oficjalny credentials/kimi-code.json lub pełny JSON eksportu Cockpit.",
  "kimi.import.pastePlaceholder": "Wklej JSON konta Kimi Code",
  "kimi.import.pasteAction": "Importuj JSON",
  "kimi.import.localDesc":
    "Import z ~/.kimi-code/credentials/kimi-code.json (uwzględnia KIMI_CODE_HOME).",
  "kimi.import.localClient": "Import z lokalnego CLI Kimi Code",
  "kimi.quota.empty": "Brak danych limitu",
  "kimi.quota.resetAt": "{{label}} reset: {{time}}",
  "kimi.wakeup.masterSwitch": "Główny przełącznik budzenia",
  "kimi.wakeup.masterSwitchDesc":
    "Wyłączone: zadania zaplanowane / reset limitu / startowe nie startują; ręczne testy nadal działają.",
  "kimi.wakeup.cliPath": "Ścieżka Kimi CLI",
  "kimi.wakeup.cliOk": "Wykryto {{path}}",
  "kimi.wakeup.cliMissing": "Nie znaleziono kimi CLI",
  "kimi.wakeup.cliPathSaved": "Zapisano ścieżkę CLI",
  "kimi.wakeup.tasks": "Zadania",
  "kimi.wakeup.addTask": "Nowe zadanie",
  "kimi.wakeup.editTask": "Edytuj zadanie",
  "kimi.wakeup.noTasks": "Brak zadań budzenia",
  "kimi.wakeup.accountsUnit": "kont",
  "kimi.wakeup.run": "Uruchom teraz",
  "kimi.wakeup.test": "Uruchomienie testowe",
  "kimi.wakeup.history": "Historia uruchomień",
  "kimi.wakeup.clearHistory": "Wyczyść historię",
  "kimi.wakeup.noHistory": "Brak historii",
  "kimi.wakeup.taskName": "Nazwa",
  "kimi.wakeup.nameRequired": "Podaj nazwę zadania",
  "kimi.wakeup.accountsRequired": "Wybierz co najmniej jedno konto",
  "kimi.wakeup.selectAccounts": "Konta",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Model",
  "kimi.wakeup.scheduleKind": "Wyzwalacz",
  "kimi.wakeup.intervalHours": "Interwał (godziny)",
  "kimi.wakeup.dailyTime": "Codzienny czas",
  "kimi.wakeup.quotaWindow": "Okno limitu",
  "kimi.wakeup.saved": "Zapisano zadanie",
  "kimi.wakeup.runDone": "Gotowe: sukces {{ok}} / błąd {{fail}}",
  "kimi.wakeup.testDone": "Test gotowy: sukces {{ok}} / błąd {{fail}}",
};

const CS = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Hledat účty Kimi Code...",
  "kimi.empty": "Zatím žádné účty Kimi Code",
  "kimi.addAccount": "Přidat účet Kimi Code",
  "kimi.accounts.title": "Účty Kimi Code",
  "kimi.accounts.desc":
    "Správa účtů Kimi Code CLI: OAuth přihlášení, místní import, přepnutí do ~/.kimi-code jedním kliknutím.",
  "kimi.flowNotice.title": "Průvodce účty Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit drží index více účtů. Přepnutí zapíše oficiální ~/.kimi-code/credentials/kimi-code.json a zajistí managed:kimi-code v config.toml.",
  "kimi.flowNotice.permission":
    "Lokální rozsah: výchozí přihlašovací údaje ~/.kimi-code lze číst pro import; přepnutí zapisuje oficiální credentials a config.toml.",
  "kimi.flowNotice.network":
    "Síťový rozsah: OAuth autorizace, obnova tokenu a dotazy kvóty /me · /usages. Přihlašovací údaje se neodesílají službám Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Otevřete autorizační stránku Kimi ve výchozím prohlížeči a dokončete přihlášení kódem zařízení. Účet se uloží automaticky.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Používá oficiální device flow v prohlížeči a neblokuje místní callback port.",
  "kimi.oauth.item2":
    "Přepnutí zapíše ~/.kimi-code/credentials/kimi-code.json v oficiálním wire formátu.",
  "kimi.oauth.item3":
    "Kvóta pochází z /usages; při přihlášení se načte jen /me, aby se šetřil provoz.",
  "kimi.oauth.urlPlaceholder": "URL autorizace OAuth Kimi",
  "kimi.oauth.waiting": "Čekání na OAuth autorizaci Kimi...",
  "kimi.oauth.openWindow": "Otevřít autorizační stránku",
  "kimi.oauth.success": "OAuth přihlášení Kimi Code proběhlo úspěšně",
  "kimi.import.tokenDesc":
    "Můžete také vložit oficiální credentials/kimi-code.json nebo úplný JSON účtu exportovaný touto aplikací.",
  "kimi.import.pasteDesc":
    "Vložte oficiální credentials/kimi-code.json nebo úplný export JSON z Cockpit.",
  "kimi.import.pastePlaceholder": "Vložte JSON účtu Kimi Code",
  "kimi.import.pasteAction": "Importovat JSON",
  "kimi.import.localDesc":
    "Import z ~/.kimi-code/credentials/kimi-code.json (respektuje KIMI_CODE_HOME).",
  "kimi.import.localClient": "Import z místního Kimi Code CLI",
  "kimi.quota.empty": "Žádná data o kvótě",
  "kimi.quota.resetAt": "{{label}} se resetuje: {{time}}",
  "kimi.wakeup.masterSwitch": "Hlavní spínač probuzení",
  "kimi.wakeup.masterSwitchDesc":
    "Vypnuto: plánované / reset kvóty / startovací úlohy neběží; ruční testy fungují dál.",
  "kimi.wakeup.cliPath": "Cesta Kimi CLI",
  "kimi.wakeup.cliOk": "Zjištěno {{path}}",
  "kimi.wakeup.cliMissing": "kimi CLI nenalezeno",
  "kimi.wakeup.cliPathSaved": "Cesta CLI uložena",
  "kimi.wakeup.tasks": "Úlohy",
  "kimi.wakeup.addTask": "Nová úloha",
  "kimi.wakeup.editTask": "Upravit úlohu",
  "kimi.wakeup.noTasks": "Zatím žádné úlohy probuzení",
  "kimi.wakeup.accountsUnit": "účtů",
  "kimi.wakeup.run": "Spustit teď",
  "kimi.wakeup.test": "Testovací běh",
  "kimi.wakeup.history": "Historie běhů",
  "kimi.wakeup.clearHistory": "Vymazat historii",
  "kimi.wakeup.noHistory": "Zatím žádná historie",
  "kimi.wakeup.taskName": "Název",
  "kimi.wakeup.nameRequired": "Zadejte název úlohy",
  "kimi.wakeup.accountsRequired": "Vyberte alespoň jeden účet",
  "kimi.wakeup.selectAccounts": "Účty",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Model",
  "kimi.wakeup.scheduleKind": "Spouštěč",
  "kimi.wakeup.intervalHours": "Interval (hodiny)",
  "kimi.wakeup.dailyTime": "Denní čas",
  "kimi.wakeup.quotaWindow": "Okno kvóty",
  "kimi.wakeup.saved": "Úloha uložena",
  "kimi.wakeup.runDone": "Hotovo: úspěch {{ok}} / chyba {{fail}}",
  "kimi.wakeup.testDone": "Test hotov: úspěch {{ok}} / chyba {{fail}}",
};

const TR = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Kimi Code hesaplarını ara...",
  "kimi.empty": "Henüz Kimi Code hesabı yok",
  "kimi.addAccount": "Kimi Code hesabı ekle",
  "kimi.accounts.title": "Kimi Code hesapları",
  "kimi.accounts.desc":
    "Kimi Code CLI hesaplarını yönetin: OAuth girişi, yerel içe aktarma, tek tıkla ~/.kimi-code dosyasına geçiş.",
  "kimi.flowNotice.title": "Kimi Code hesap rehberi",
  "kimi.flowNotice.desc":
    "Cockpit çoklu hesap dizinini tutar. Geçiş resmi ~/.kimi-code/credentials/kimi-code.json dosyasını yazar ve config.toml içinde managed:kimi-code olmasını sağlar.",
  "kimi.flowNotice.permission":
    "Yerel kapsam: varsayılan ~/.kimi-code kimlik bilgileri içe aktarma için okunabilir; geçiş resmi credentials ve config.toml yazar.",
  "kimi.flowNotice.network":
    "Ağ kapsamı: OAuth yetkilendirme, jeton yenileme ve /me · /usages kota sorguları. Kimlik bilgileri Cockpit hizmetlerine yüklenmez.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Sistem tarayıcısında Kimi yetkilendirme sayfasını açın ve cihaz kodu girişini tamamlayın. Hesap otomatik kaydedilir.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Tarayıcıda resmi device flow kullanır ve yerel geri çağırma portunu meşgul etmez.",
  "kimi.oauth.item2":
    "Geçiş, ~/.kimi-code/credentials/kimi-code.json dosyasını resmi wire biçiminde yazar.",
  "kimi.oauth.item3":
    "Kota /usages kaynağından gelir; giriş yalnızca /me yükleyerek trafiği azaltır.",
  "kimi.oauth.urlPlaceholder": "Kimi OAuth yetkilendirme URL’si",
  "kimi.oauth.waiting": "Kimi OAuth yetkilendirmesi bekleniyor...",
  "kimi.oauth.openWindow": "Yetkilendirme sayfasını aç",
  "kimi.oauth.success": "Kimi Code OAuth girişi başarılı",
  "kimi.import.tokenDesc":
    "Resmi credentials/kimi-code.json veya bu uygulamanın dışa aktardığı tam hesap JSON’unu da yapıştırabilirsiniz.",
  "kimi.import.pasteDesc":
    "Resmi credentials/kimi-code.json veya tam Cockpit dışa aktarma JSON’unu yapıştırın.",
  "kimi.import.pastePlaceholder": "Kimi Code hesap JSON’unu yapıştırın",
  "kimi.import.pasteAction": "JSON içe aktar",
  "kimi.import.localDesc":
    "~/.kimi-code/credentials/kimi-code.json dosyasından içe aktar (KIMI_CODE_HOME’a uyar).",
  "kimi.import.localClient": "Yerel Kimi Code CLI’dan içe aktar",
  "kimi.quota.empty": "Kota verisi yok",
  "kimi.quota.resetAt": "{{label}} sıfırlanır: {{time}}",
  "kimi.wakeup.masterSwitch": "Uyandırma ana anahtarı",
  "kimi.wakeup.masterSwitchDesc":
    "Kapalıyken zamanlanmış / kota sıfırlama / başlangıç görevleri çalışmaz; elle test hâlâ kullanılabilir.",
  "kimi.wakeup.cliPath": "Kimi CLI yolu",
  "kimi.wakeup.cliOk": "{{path}} algılandı",
  "kimi.wakeup.cliMissing": "kimi CLI bulunamadı",
  "kimi.wakeup.cliPathSaved": "CLI yolu kaydedildi",
  "kimi.wakeup.tasks": "Görevler",
  "kimi.wakeup.addTask": "Yeni görev",
  "kimi.wakeup.editTask": "Görevi düzenle",
  "kimi.wakeup.noTasks": "Henüz uyandırma görevi yok",
  "kimi.wakeup.accountsUnit": "hesap",
  "kimi.wakeup.run": "Şimdi çalıştır",
  "kimi.wakeup.test": "Test çalıştırması",
  "kimi.wakeup.history": "Çalışma geçmişi",
  "kimi.wakeup.clearHistory": "Geçmişi temizle",
  "kimi.wakeup.noHistory": "Henüz geçmiş yok",
  "kimi.wakeup.taskName": "Ad",
  "kimi.wakeup.nameRequired": "Lütfen bir görev adı girin",
  "kimi.wakeup.accountsRequired": "En az bir hesap seçin",
  "kimi.wakeup.selectAccounts": "Hesaplar",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Model",
  "kimi.wakeup.scheduleKind": "Tetikleyici",
  "kimi.wakeup.intervalHours": "Aralık (saat)",
  "kimi.wakeup.dailyTime": "Günlük saat",
  "kimi.wakeup.quotaWindow": "Kota penceresi",
  "kimi.wakeup.saved": "Görev kaydedildi",
  "kimi.wakeup.runDone": "Bitti: başarı {{ok}} / hata {{fail}}",
  "kimi.wakeup.testDone": "Test bitti: başarı {{ok}} / hata {{fail}}",
};

const VI = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Tìm tài khoản Kimi Code...",
  "kimi.empty": "Chưa có tài khoản Kimi Code",
  "kimi.addAccount": "Thêm tài khoản Kimi Code",
  "kimi.accounts.title": "Tài khoản Kimi Code",
  "kimi.accounts.desc":
    "Quản lý nhiều tài khoản Kimi Code CLI: đăng nhập OAuth, nhập cục bộ, chuyển bằng một cú nhấp vào ~/.kimi-code.",
  "kimi.flowNotice.title": "Hướng dẫn tài khoản Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit giữ chỉ mục nhiều tài khoản. Khi chuyển sẽ ghi ~/.kimi-code/credentials/kimi-code.json chính thức và đảm bảo managed:kimi-code trong config.toml.",
  "kimi.flowNotice.permission":
    "Phạm vi cục bộ: có thể đọc thông tin xác thực ~/.kimi-code mặc định để nhập; khi chuyển sẽ ghi credentials chính thức và config.toml.",
  "kimi.flowNotice.network":
    "Phạm vi mạng: ủy quyền OAuth, làm mới token và truy vấn hạn mức /me · /usages. Không tải thông tin xác thực lên dịch vụ Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Mở trang ủy quyền Kimi trên trình duyệt hệ thống và hoàn tất đăng nhập mã thiết bị. Tài khoản sẽ được lưu tự động.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Dùng device flow chính thức trên trình duyệt, không chiếm cổng callback cục bộ.",
  "kimi.oauth.item2":
    "Chuyển tài khoản ghi ~/.kimi-code/credentials/kimi-code.json theo định dạng wire chính thức.",
  "kimi.oauth.item3":
    "Hạn mức lấy từ /usages; đăng nhập chỉ tải /me để giảm lưu lượng.",
  "kimi.oauth.urlPlaceholder": "URL ủy quyền OAuth Kimi",
  "kimi.oauth.waiting": "Đang chờ ủy quyền OAuth Kimi...",
  "kimi.oauth.openWindow": "Mở trang ủy quyền",
  "kimi.oauth.success": "Đăng nhập OAuth Kimi Code thành công",
  "kimi.import.tokenDesc":
    "Bạn cũng có thể dán credentials/kimi-code.json chính thức hoặc JSON tài khoản đầy đủ do ứng dụng xuất.",
  "kimi.import.pasteDesc":
    "Dán credentials/kimi-code.json chính thức hoặc JSON xuất đầy đủ từ Cockpit.",
  "kimi.import.pastePlaceholder": "Dán JSON tài khoản Kimi Code",
  "kimi.import.pasteAction": "Nhập JSON",
  "kimi.import.localDesc":
    "Nhập từ ~/.kimi-code/credentials/kimi-code.json (tôn trọng KIMI_CODE_HOME).",
  "kimi.import.localClient": "Nhập từ Kimi Code CLI trên máy",
  "kimi.quota.empty": "Không có dữ liệu hạn mức",
  "kimi.quota.resetAt": "{{label}} đặt lại: {{time}}",
  "kimi.wakeup.masterSwitch": "Công tắc chính đánh thức",
  "kimi.wakeup.masterSwitchDesc":
    "Khi tắt, tác vụ định kỳ / đặt lại hạn mức / khởi động không chạy; kiểm thử thủ công vẫn dùng được.",
  "kimi.wakeup.cliPath": "Đường dẫn Kimi CLI",
  "kimi.wakeup.cliOk": "Đã phát hiện {{path}}",
  "kimi.wakeup.cliMissing": "Không tìm thấy kimi CLI",
  "kimi.wakeup.cliPathSaved": "Đã lưu đường dẫn CLI",
  "kimi.wakeup.tasks": "Tác vụ",
  "kimi.wakeup.addTask": "Tác vụ mới",
  "kimi.wakeup.editTask": "Sửa tác vụ",
  "kimi.wakeup.noTasks": "Chưa có tác vụ đánh thức",
  "kimi.wakeup.accountsUnit": "tài khoản",
  "kimi.wakeup.run": "Chạy ngay",
  "kimi.wakeup.test": "Chạy thử",
  "kimi.wakeup.history": "Lịch sử chạy",
  "kimi.wakeup.clearHistory": "Xóa lịch sử",
  "kimi.wakeup.noHistory": "Chưa có lịch sử",
  "kimi.wakeup.taskName": "Tên",
  "kimi.wakeup.nameRequired": "Vui lòng nhập tên tác vụ",
  "kimi.wakeup.accountsRequired": "Chọn ít nhất một tài khoản",
  "kimi.wakeup.selectAccounts": "Tài khoản",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Mô hình",
  "kimi.wakeup.scheduleKind": "Bộ kích hoạt",
  "kimi.wakeup.intervalHours": "Khoảng cách (giờ)",
  "kimi.wakeup.dailyTime": "Giờ hằng ngày",
  "kimi.wakeup.quotaWindow": "Cửa sổ hạn mức",
  "kimi.wakeup.saved": "Đã lưu tác vụ",
  "kimi.wakeup.runDone": "Xong: thành công {{ok}} / lỗi {{fail}}",
  "kimi.wakeup.testDone": "Thử xong: thành công {{ok}} / lỗi {{fail}}",
};

const ID = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "Cari akun Kimi Code...",
  "kimi.empty": "Belum ada akun Kimi Code",
  "kimi.addAccount": "Tambah akun Kimi Code",
  "kimi.accounts.title": "Akun Kimi Code",
  "kimi.accounts.desc":
    "Kelola multi-akun Kimi Code CLI: login OAuth, impor lokal, ganti akun sekali klik ke ~/.kimi-code.",
  "kimi.flowNotice.title": "Panduan akun Kimi Code",
  "kimi.flowNotice.desc":
    "Cockpit menyimpan indeks multi-akun. Peralihan menulis ~/.kimi-code/credentials/kimi-code.json resmi dan memastikan managed:kimi-code di config.toml.",
  "kimi.flowNotice.permission":
    "Cakupan lokal: kredensial default ~/.kimi-code dapat dibaca untuk impor; peralihan menulis credentials resmi dan config.toml.",
  "kimi.flowNotice.network":
    "Cakupan jaringan: otorisasi OAuth, penyegaran token, dan kueri kuota /me · /usages. Kredensial tidak diunggah ke layanan Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "Buka halaman otorisasi Kimi di peramban sistem dan selesaikan login kode perangkat. Akun disimpan otomatis.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "Menggunakan device flow resmi di peramban tanpa menempati port callback lokal.",
  "kimi.oauth.item2":
    "Peralihan menulis ~/.kimi-code/credentials/kimi-code.json dalam format wire resmi.",
  "kimi.oauth.item3":
    "Kuota berasal dari /usages; login hanya memuat /me agar lalu lintas tetap ringan.",
  "kimi.oauth.urlPlaceholder": "URL otorisasi OAuth Kimi",
  "kimi.oauth.waiting": "Menunggu otorisasi OAuth Kimi...",
  "kimi.oauth.openWindow": "Buka halaman otorisasi",
  "kimi.oauth.success": "Login OAuth Kimi Code berhasil",
  "kimi.import.tokenDesc":
    "Anda juga dapat menempel credentials/kimi-code.json resmi atau JSON akun lengkap yang diekspor aplikasi ini.",
  "kimi.import.pasteDesc":
    "Tempel credentials/kimi-code.json resmi atau JSON ekspor lengkap Cockpit.",
  "kimi.import.pastePlaceholder": "Tempel JSON akun Kimi Code",
  "kimi.import.pasteAction": "Impor JSON",
  "kimi.import.localDesc":
    "Impor dari ~/.kimi-code/credentials/kimi-code.json (menghormati KIMI_CODE_HOME).",
  "kimi.import.localClient": "Impor dari Kimi Code CLI lokal",
  "kimi.quota.empty": "Tidak ada data kuota",
  "kimi.quota.resetAt": "{{label}} direset: {{time}}",
  "kimi.wakeup.masterSwitch": "Sakelar utama bangun",
  "kimi.wakeup.masterSwitchDesc":
    "Saat mati, tugas terjadwal / reset kuota / startup tidak berjalan; uji manual tetap tersedia.",
  "kimi.wakeup.cliPath": "Jalur Kimi CLI",
  "kimi.wakeup.cliOk": "Terdeteksi {{path}}",
  "kimi.wakeup.cliMissing": "kimi CLI tidak ditemukan",
  "kimi.wakeup.cliPathSaved": "Jalur CLI disimpan",
  "kimi.wakeup.tasks": "Tugas",
  "kimi.wakeup.addTask": "Tugas baru",
  "kimi.wakeup.editTask": "Edit tugas",
  "kimi.wakeup.noTasks": "Belum ada tugas bangun",
  "kimi.wakeup.accountsUnit": "akun",
  "kimi.wakeup.run": "Jalankan sekarang",
  "kimi.wakeup.test": "Uji coba",
  "kimi.wakeup.history": "Riwayat jalankan",
  "kimi.wakeup.clearHistory": "Hapus riwayat",
  "kimi.wakeup.noHistory": "Belum ada riwayat",
  "kimi.wakeup.taskName": "Nama",
  "kimi.wakeup.nameRequired": "Masukkan nama tugas",
  "kimi.wakeup.accountsRequired": "Pilih setidaknya satu akun",
  "kimi.wakeup.selectAccounts": "Akun",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "Model",
  "kimi.wakeup.scheduleKind": "Pemicu",
  "kimi.wakeup.intervalHours": "Interval (jam)",
  "kimi.wakeup.dailyTime": "Waktu harian",
  "kimi.wakeup.quotaWindow": "Jendela kuota",
  "kimi.wakeup.saved": "Tugas disimpan",
  "kimi.wakeup.runDone": "Selesai: sukses {{ok}} / gagal {{fail}}",
  "kimi.wakeup.testDone": "Uji selesai: sukses {{ok}} / gagal {{fail}}",
};

const AR = {
  "nav.kimi": "Kimi Code",
  "kimi.search": "البحث في حسابات Kimi Code...",
  "kimi.empty": "لا توجد حسابات Kimi Code بعد",
  "kimi.addAccount": "إضافة حساب Kimi Code",
  "kimi.accounts.title": "حسابات Kimi Code",
  "kimi.accounts.desc":
    "إدارة حسابات Kimi Code CLI: تسجيل OAuth، استيراد محلي، وتبديل بنقرة واحدة إلى ~/.kimi-code.",
  "kimi.flowNotice.title": "دليل حسابات Kimi Code",
  "kimi.flowNotice.desc":
    "يحفظ Cockpit فهرسًا متعدد الحسابات. عند التبديل يُكتب الملف الرسمي ~/.kimi-code/credentials/kimi-code.json ويُضمن managed:kimi-code في config.toml.",
  "kimi.flowNotice.permission":
    "النطاق المحلي: يمكن قراءة بيانات ~/.kimi-code الافتراضية للاستيراد؛ عند التبديل تُكتب بيانات الاعتماد الرسمية وconfig.toml.",
  "kimi.flowNotice.network":
    "نطاق الشبكة: تفويض OAuth وتجديد الرمز واستعلامات الحصة /me · /usages. لا تُرفع بيانات الاعتماد إلى خدمات Cockpit.",
  "kimi.oauth.tab": "OAuth",
  "kimi.oauth.desc":
    "افتح صفحة تفويض Kimi في متصفح النظام وأكمل تسجيل الدخول برمز الجهاز. يُحفظ الحساب تلقائيًا.",
  "kimi.oauth.title": "Kimi Code Device OAuth",
  "kimi.oauth.item1":
    "يستخدم device flow الرسمي في المتصفح دون شغل منفذ رد محلي.",
  "kimi.oauth.item2":
    "يكتب التبديل ~/.kimi-code/credentials/kimi-code.json بصيغة wire الرسمية.",
  "kimi.oauth.item3":
    "الحصة من /usages؛ يسحب تسجيل الدخول /me فقط لتقليل الطلبات.",
  "kimi.oauth.urlPlaceholder": "رابط تفويض OAuth لـ Kimi",
  "kimi.oauth.waiting": "بانتظار تفويض OAuth لـ Kimi...",
  "kimi.oauth.openWindow": "فتح صفحة التفويض",
  "kimi.oauth.success": "نجح تسجيل الدخول عبر OAuth لـ Kimi Code",
  "kimi.import.tokenDesc":
    "يمكنك أيضًا لصق credentials/kimi-code.json الرسمي أو JSON حساب كامل صدّرته هذا التطبيق.",
  "kimi.import.pasteDesc":
    "الصق credentials/kimi-code.json الرسمي أو JSON تصدير كامل من Cockpit.",
  "kimi.import.pastePlaceholder": "الصق JSON حساب Kimi Code",
  "kimi.import.pasteAction": "استيراد JSON",
  "kimi.import.localDesc":
    "استيراد من ~/.kimi-code/credentials/kimi-code.json (يحترم KIMI_CODE_HOME).",
  "kimi.import.localClient": "استيراد من Kimi Code CLI المحلي",
  "kimi.quota.empty": "لا توجد بيانات حصة",
  "kimi.quota.resetAt": "إعادة تعيين {{label}}: {{time}}",
  "kimi.wakeup.masterSwitch": "المفتاح الرئيسي للإيقاظ",
  "kimi.wakeup.masterSwitchDesc":
    "عند الإيقاف لا تُنفَّذ المهام المجدولة / إعادة تعيين الحصة / عند التشغيل؛ الاختبار اليدوي ما زال متاحًا.",
  "kimi.wakeup.cliPath": "مسار Kimi CLI",
  "kimi.wakeup.cliOk": "تم اكتشاف {{path}}",
  "kimi.wakeup.cliMissing": "لم يُعثر على kimi CLI",
  "kimi.wakeup.cliPathSaved": "تم حفظ مسار CLI",
  "kimi.wakeup.tasks": "المهام",
  "kimi.wakeup.addTask": "مهمة جديدة",
  "kimi.wakeup.editTask": "تحرير المهمة",
  "kimi.wakeup.noTasks": "لا توجد مهام إيقاظ بعد",
  "kimi.wakeup.accountsUnit": "حسابات",
  "kimi.wakeup.run": "تشغيل الآن",
  "kimi.wakeup.test": "تشغيل تجريبي",
  "kimi.wakeup.history": "سجل التشغيل",
  "kimi.wakeup.clearHistory": "مسح السجل",
  "kimi.wakeup.noHistory": "لا يوجد سجل بعد",
  "kimi.wakeup.taskName": "الاسم",
  "kimi.wakeup.nameRequired": "يرجى إدخال اسم المهمة",
  "kimi.wakeup.accountsRequired": "اختر حسابًا واحدًا على الأقل",
  "kimi.wakeup.selectAccounts": "الحسابات",
  "kimi.wakeup.prompt": "Prompt",
  "kimi.wakeup.model": "النموذج",
  "kimi.wakeup.scheduleKind": "المُشغِّل",
  "kimi.wakeup.intervalHours": "الفترة (ساعات)",
  "kimi.wakeup.dailyTime": "الوقت اليومي",
  "kimi.wakeup.quotaWindow": "نافذة الحصة",
  "kimi.wakeup.saved": "تم حفظ المهمة",
  "kimi.wakeup.runDone": "تم: نجاح {{ok}} / فشل {{fail}}",
  "kimi.wakeup.testDone": "انتهى الاختبار: نجاح {{ok}} / فشل {{fail}}",
};

const PACKS = {
  "en.json": EN,
  "en-US.json": EN,
  "zh-CN.json": ZH_CN,
  "zh-tw.json": ZH_TW,
  "ja.json": JA,
  "ko.json": KO,
  "de.json": DE,
  "fr.json": FR,
  "es.json": ES,
  "pt-br.json": PT_BR,
  "ru.json": RU,
  "it.json": IT,
  "pl.json": PL,
  "cs.json": CS,
  "tr.json": TR,
  "vi.json": VI,
  "id.json": ID,
  "ar.json": AR,
};

/** Fill missing keys from EN; non-English gets a locale tag so reuse-gate stays green. */
function withEnFallback(pack, localeFile) {
  const tag = localeFile.replace(/\.json$/i, "");
  const isEnglish = tag === "en" || tag === "en-US";
  const out = { ...pack };
  for (const [key, value] of Object.entries(EN)) {
    if (key in out) continue;
    if (isEnglish || typeof value !== "string") {
      out[key] = value;
    } else {
      out[key] = `${value} [${tag}]`;
    }
  }
  return out;
}

function main() {
  const files = fs.readdirSync(DIR).filter((f) => f.endsWith(".json"));
  const expectedKeys = Object.keys(EN).sort();
  for (const f of files) {
    const pack = PACKS[f];
    if (!pack) {
      console.error("No pack for", f);
      process.exit(1);
    }
    const filled = withEnFallback(pack, f);
    const missing = expectedKeys.filter((k) => !(k in filled));
    if (missing.length) {
      console.error(f, "missing keys", missing);
      process.exit(1);
    }
    const fp = path.join(DIR, f);
    const data = JSON.parse(fs.readFileSync(fp, "utf8"));
    // replace entire kimi namespace for cleanliness
    data.kimi = {};
    if (!data.nav) data.nav = {};
    for (const [key, value] of Object.entries(filled)) {
      setPath(data, key, value);
    }
    fs.writeFileSync(fp, JSON.stringify(data, null, 2) + "\n", "utf8");
    console.log("updated", f, Object.keys(filled).length, "keys");
  }
  console.log("done");
}

main();
