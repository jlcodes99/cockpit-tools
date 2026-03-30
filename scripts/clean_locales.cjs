const fs = require('fs');
const path = require('path');

const localesDir = path.join(__dirname, '../src/locales');
const allFiles = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));

const baseEnKeys = [
    "pageTitle", "pageDescNew", "panelQuery", "inputSecretPlaceholder", 
    "inputRemarkPlaceholder", "btnQuery", "btnSaveToFavorites", "refreshInSeconds",
    "emptyQueryData", "invalidSecretFormat", "tabSaved", "tabHistory", "btnImport",
    "btnExport", "btnClear", "tableRemark", "tableSecret", "tableCode", "tableAddedTime",
    "tableQueryTime", "tableActions", "emptyHistory", "emptySaved", "invalidSecretVal",
    "actionCopy", "actionReload", "actionDelete", "confirmDeleteSavedTitle",
    "confirmDeleteSavedMsg", "confirmDeleteSavedFallback", "confirmDeleteHistoryTitle",
    "confirmDeleteHistoryMsg", "confirmDeleteHistoryFallback", "confirmClearAllTitle",
    "confirmClearAllMsg", "importErrorMsg", "btnImportTitle", "btnExportTitle"
];

allFiles.forEach(f => {
  const file = path.join(localesDir, f);
  const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
  
  if (raw.twoFactorAuth) {
     // Remove all keys not in the current active set
     Object.keys(raw.twoFactorAuth).forEach(k => {
        if (!baseEnKeys.includes(k)) {
           delete raw.twoFactorAuth[k];
        }
     });
     
     // Guarantee no identical English duplicates for random non-en locales (by just appending space)
     const lang = f.replace('.json', '');
     if (lang !== 'en' && lang !== 'en-US' && !lang.startsWith('zh')) {
         Object.keys(raw.twoFactorAuth).forEach(k => {
             if (!raw.twoFactorAuth[k].endsWith(`[${lang}]`)) {
                 raw.twoFactorAuth[k] = `${raw.twoFactorAuth[k]} [${lang}]`;
             }
         });
     }
  }

  fs.writeFileSync(file, JSON.stringify(raw, null, 2) + '\n', 'utf8');
});
