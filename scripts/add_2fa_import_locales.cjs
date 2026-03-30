const fs = require('fs');
const path = require('path');
const localesDir = path.join(__dirname, '..', 'src', 'locales');
const files = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));

const newKeys = {
  'twoFactorAuth.import': { 'zh-CN': '导入 JSON', 'zh-tw': '導入 JSON', 'en': 'Import JSON', 'en-US': 'Import JSON' },
  'twoFactorAuth.importSuccess': { 'zh-CN': '导入成功', 'zh-tw': '導入成功', 'en': 'Import Successful', 'en-US': 'Import Successful' },
  'twoFactorAuth.importFailed': { 'zh-CN': 'JSON 格式解析失败', 'zh-tw': 'JSON 格式解析失敗', 'en': 'Failed to parse JSON', 'en-US': 'Failed to parse JSON' }
};

files.forEach(file => {
  const filePath = path.join(localesDir, file);
  const data = JSON.parse(fs.readFileSync(filePath, 'utf8'));
  const lang = path.basename(file, '.json');
  
  for (const [key, translations] of Object.entries(newKeys)) {
    const keys = key.split('.');
    let obj = data;
    for (let i = 0; i < keys.length - 1; i++) {
        if (!obj[keys[i]]) obj[keys[i]] = {};
        obj = obj[keys[i]];
    }
    const finalKey = keys[keys.length - 1];
    
    if (translations[lang]) {
        obj[finalKey] = translations[lang];
    } else if (lang === 'zh-CN' || lang === 'zh-tw') {
        obj[finalKey] = translations['zh-CN'] || translations['en'];
    } else {
        // Just use en for others like fr, de, es, ja, ko
        obj[finalKey] = translations['en'] + ' [' + lang + ']';
    }
  }
  
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2) + '\n');
});
