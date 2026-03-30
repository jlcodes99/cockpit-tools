const fs = require('fs');
const path = require('path');

const localesDir = path.join(__dirname, '../src/locales');
const files = fs.readdirSync(localesDir).filter(f => f.endsWith('.json'));

const newKeys = {
  'nav.2faManager': {
    'zh-CN': '2FA管理',
    'zh-tw': '2FA管理',
    'en': '2FA Management',
    'en-US': '2FA Management'
  },
  'twoFactorAuth.emptyDesc': {
    'zh-CN': '暂无 2FA 账户，请添加一个。',
    'zh-tw': '暫無 2FA 帳戶，請添加一個。',
    'en': 'No 2FA accounts yet, please add one.',
    'en-US': 'No 2FA accounts yet, please add one.'
  },
  'twoFactorAuth.addAccount': {
    'zh-CN': '添加账户',
    'zh-tw': '添加帳戶',
    'en': 'Add Account',
    'en-US': 'Add Account'
  },
  'twoFactorAuth.accountList': {
    'zh-CN': '账户列表',
    'zh-tw': '帳戶列表',
    'en': 'Account List',
    'en-US': 'Account List'
  },
  'twoFactorAuth.add': {
    'zh-CN': '添加',
    'zh-tw': '添加',
    'en': 'Add',
    'en-US': 'Add'
  },
  'twoFactorAuth.issuer': {
    'zh-CN': '服务商 (Issuer)',
    'zh-tw': '服務商 (Issuer)',
    'en': 'Issuer',
    'en-US': 'Issuer'
  },
  'twoFactorAuth.accountName': {
    'zh-CN': '账户名',
    'zh-tw': '帳戶名',
    'en': 'Account Name',
    'en-US': 'Account Name'
  },
  'twoFactorAuth.secret': {
    'zh-CN': '密钥 (Secret)',
    'zh-tw': '密鑰 (Secret)',
    'en': 'Secret',
    'en-US': 'Secret'
  }
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
    
    // Fallback logic
    if (translations[lang]) {
        obj[finalKey] = translations[lang];
    } else if (lang === 'zh-CN' || lang === 'zh-tw') {
        obj[finalKey] = translations['zh-CN'] || translations['en'];
    } else {
        // Just use en for others like fr, de, es, ja, ko
        obj[finalKey] = translations['en'];
    }
  }
  
  fs.writeFileSync(filePath, JSON.stringify(data, null, 2) + "\n");
  console.log(`Updated ${file}`);
});
