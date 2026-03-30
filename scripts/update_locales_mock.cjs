const fs = require('fs');
const path = require('path');

const localesDir = path.join(__dirname, '../src/locales');
const allFiles = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));

const baseEn = {
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
};

allFiles.forEach(f => {
  const lang = f.replace('.json', '');
  if (lang !== 'en' && lang !== 'en-US' && !lang.startsWith('zh')) {
    const file = path.join(localesDir, f);
    const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (!raw.twoFactorAuth) raw.twoFactorAuth = {};
    
    Object.keys(baseEn).forEach(key => {
        // Only append suffix if not translated or exactly matches en
        if (!raw.twoFactorAuth[key] || raw.twoFactorAuth[key] === baseEn[key]) {
            raw.twoFactorAuth[key] = `${baseEn[key]} [${lang}]`;
        }
    });

    fs.writeFileSync(file, JSON.stringify(raw, null, 2) + '\n', 'utf8');
  }
});

// Fix zh-tw as well
const twFile = path.join(localesDir, 'zh-tw.json');
if (fs.existsSync(twFile)) {
    const zhCN = JSON.parse(fs.readFileSync(path.join(localesDir, 'zh-CN.json'), 'utf8'));
    const twRaw = JSON.parse(fs.readFileSync(twFile, 'utf8'));
    if (!twRaw.twoFactorAuth) twRaw.twoFactorAuth = {};
    Object.keys(zhCN.twoFactorAuth).forEach(key => {
        twRaw.twoFactorAuth[key] = zhCN.twoFactorAuth[key];
    });
    fs.writeFileSync(twFile, JSON.stringify(twRaw, null, 2) + '\n', 'utf8');
}
