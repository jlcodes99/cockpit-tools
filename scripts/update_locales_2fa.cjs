const fs = require('fs');
const path = require('path');

const locales = ['zh-CN', 'en', 'en-US'];

const translations = {
  'zh-CN': {
    "pageTitle": "2FA 查询器",
    "pageDescNew": "输入或粘贴 2FA Base32 秘钥即可即时生成验证码，查询系统会自动矫正并去除非标准字符。",
    "panelQuery": "功能区 (查询面板)",
    "inputSecretPlaceholder": "在此粘贴 2FA 秘钥 (如: JBSWY3DPEHPK3PXP)",
    "inputRemarkPlaceholder": "[选填] 给这个秘钥设置一个备注名称",
    "btnQuery": "查 询",
    "btnSaveToFavorites": "保存到列表",
    "refreshInSeconds": "{{time}} 秒后刷新",
    "emptyQueryData": "暂未查询数据",
    "invalidSecretFormat": "检测到无效秘钥格式",
    "tabSaved": "★ 已保存",
    "tabHistory": "近期查询",
    "btnImport": "导入",
    "btnExport": "导出",
    "btnClear": "清空",
    "tableRemark": "备注",
    "tableSecret": "秘钥",
    "tableCode": "动态码",
    "tableAddedTime": "添加时间",
    "tableQueryTime": "查询时间",
    "tableActions": "操作",
    "emptyHistory": "暂无查询历史",
    "emptySaved": "您尚未保存任何 2FA 认证",
    "invalidSecretVal": "无效秘钥",
    "actionCopy": "复制验证码",
    "actionReload": "重新加载到查询器",
    "actionDelete": "删除",
    "confirmDeleteSavedTitle": "删除确认",
    "confirmDeleteSavedMsg": "确定要永久删除 \n{{secret}}\n 这条认证记录吗？",
    "confirmDeleteSavedFallback": "确定要永久删除这条认证记录吗？",
    "confirmDeleteHistoryTitle": "删除历史",
    "confirmDeleteHistoryMsg": "确定要删除 \n{{secret}}\n 的查询历史吗？",
    "confirmDeleteHistoryFallback": "确定要删除这条查询历史吗？",
    "confirmClearAllTitle": "清空确认",
    "confirmClearAllMsg": "确定要清空全部近期查询历史吗？",
    "importErrorMsg": "导入失败，请检查文件格式是否为您之前导出的格式 (JSON Array)。",
    "btnImportTitle": "导入本地的 JSON 文件",
    "btnExportTitle": "导出已保存为 JSON"
  },
  'en': {
    "pageTitle": "2FA Query",
    "pageDescNew": "Enter or paste a 2FA Base32 secret to instantly generate a token. The system automatically corrects and removes non-standard characters.",
    "panelQuery": "Workspace (Query Panel)",
    "inputSecretPlaceholder": "Paste 2FA Secret here (e.g. JBSWY3DPEHPK3PXP)",
    "inputRemarkPlaceholder": "[Optional] Set a remark name for this secret",
    "btnQuery": "Query",
    "btnSaveToFavorites": "Save to List",
    "refreshInSeconds": "Refresh in {{time}}s",
    "emptyQueryData": "No query data yet",
    "invalidSecretFormat": "Invalid secret format detected",
    "tabSaved": "★ Saved",
    "tabHistory": "Recent Queries",
    "btnImport": "Import",
    "btnExport": "Export",
    "btnClear": "Clear",
    "tableRemark": "Remark",
    "tableSecret": "Secret",
    "tableCode": "Code",
    "tableAddedTime": "Added Time",
    "tableQueryTime": "Query Time",
    "tableActions": "Actions",
    "emptyHistory": "No query history",
    "emptySaved": "No saved 2FA records yet",
    "invalidSecretVal": "Invalid Secret",
    "actionCopy": "Copy Code",
    "actionReload": "Reload into Query Data",
    "actionDelete": "Delete",
    "confirmDeleteSavedTitle": "Delete Confirmation",
    "confirmDeleteSavedMsg": "Are you sure you want to permanently delete \n{{secret}}\n?",
    "confirmDeleteSavedFallback": "Are you sure you want to permanently delete this secret?",
    "confirmDeleteHistoryTitle": "Delete History",
    "confirmDeleteHistoryMsg": "Are you sure you want to delete the query history for \n{{secret}}\n?",
    "confirmDeleteHistoryFallback": "Are you sure you want to delete this query history?",
    "confirmClearAllTitle": "Clear History",
    "confirmClearAllMsg": "Are you sure you want to clear all recent query history?",
    "importErrorMsg": "Import failed. Please ensure the file format is a valid exported JSON array.",
    "btnImportTitle": "Import Local JSON Backup",
    "btnExportTitle": "Export Saved as JSON"
  }
};
translations['en-US'] = translations['en'];

const localesDir = path.join(__dirname, '../src/locales');

locales.forEach(lang => {
  const file = path.join(localesDir, `${lang}.json`);
  if (fs.existsSync(file)) {
    const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!raw.twoFactorAuth) raw.twoFactorAuth = {};
    Object.assign(raw.twoFactorAuth, translations[lang]);
    fs.writeFileSync(file, JSON.stringify(raw, null, 2) + '\n', 'utf8');
    console.log(`Updated ${lang}.json`);
  }
});

// Also push english to any other files
const allFiles = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));
allFiles.forEach(f => {
  const lang = f.replace('.json', '');
  if (!locales.includes(lang)) {
    const file = path.join(localesDir, f);
    const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!raw.twoFactorAuth) raw.twoFactorAuth = {};
    Object.assign(raw.twoFactorAuth, translations['en']); // fallback English
    fs.writeFileSync(file, JSON.stringify(raw, null, 2) + '\n', 'utf8');
    console.log(`Updated fallback for ${lang}.json`);
  }
});
