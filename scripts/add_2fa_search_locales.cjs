const fs = require('fs');
const path = require('path');
const localesDir = path.join(__dirname, '..', 'src', 'locales');
const files = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));

const newKeys = {
  'twoFactorAuth.search': { 'zh-CN': '搜索服务商或账户名', 'zh-tw': '搜索服務商或帳戶名', 'en': 'Search issuer or account', 'en-US': 'Search issuer or account' },
  'twoFactorAuth.noMatch': { 'zh-CN': '没有匹配的账户', 'zh-tw': '沒有匹配的帳戶', 'en': 'No matching accounts', 'en-US': 'No matching accounts' }
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
        obj[finalKey] = translations['en'] + ' [' + lang + ']';
    }
  }
  
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2) + '\n');
});
