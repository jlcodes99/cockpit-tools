const fs = require('fs');
const path = require('path');

const srcPages = path.join(__dirname, 'src', 'pages');
const files = fs.readdirSync(srcPages)
  .filter(f => f.endsWith('AccountsPage.tsx'))
  .map(f => path.join(srcPages, f));

files.push(path.join(__dirname, 'src', 'components', 'codebuddy-suite', 'CodebuddySuiteAccountsSharedView.tsx'));

for (const file of files) {
  if (file.endsWith('AccountsPage.tsx') && file.includes('src/pages/AccountsPage.tsx')) continue;

  let content = fs.readFileSync(file, 'utf8');

  // Look for:
  // ) : viewMode === 'grid' ? (
  //   groupByTag ? (
  //     <div ...
  //   ) : (<div ...>)
  // ) : groupByTag ? (

  const startRegex = /\)\s*:\s*viewMode\s*===\s*'grid'\s*\?\s*\(/;
  const matchStart = content.match(startRegex);
  if (!matchStart) {
    console.log('Skip ' + file + ': no viewMode === grid branch');
    continue;
  }

  const idx = matchStart.index;
  // find the next closing branch `) : ` or `) : groupByTag ?` that corresponds to this ternary
  const endRegex1 = /\)\s*:\s*groupByTag\s*\?\s*\(/;
  let endMatch = content.substring(idx).match(endRegex1);

  if (!endMatch) {
    // try Codebuddy fallback
    const endRegex2 = /\)\s*:\s*groupByOrigin\s*\?\s*\(/;
    endMatch = content.substring(idx).match(endRegex2);
  }

  if (!endMatch) {
    console.log('Skip ' + file + ': no groupByTag branch after grid');
    continue;
  }

  const endIdx = idx + endMatch.index;

  const originalGridContent = content.substring(idx + matchStart[0].length, endIdx);

  // We need to inject the select all div and wrap the rest in a div
  const wrapperStart = `
        <div className="grid-view-container">
          {filteredAccounts.length > 0 && (
            <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
                <input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} />
                {t('common.selectAll', '全选')}
              </label>
            </div>
          )}
          `;

  let innerStr = originalGridContent.trim();

  // If already modified, skip
  if (innerStr.includes('grid-view-container')) {
     console.log('Already modified: ' + file);
     continue;
  }

  const newGridContent = wrapperStart + innerStr + `\n        </div>\n      `;

  const newContent = content.substring(0, idx) + ') : viewMode === \'grid\' ? (' + newGridContent + content.substring(endIdx);
  
  fs.writeFileSync(file, newContent, 'utf8');
  console.log('Patched ' + file);
}
