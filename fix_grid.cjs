const fs = require('fs');
const path = require('path');

const srcPages = path.join(__dirname, 'src', 'pages');
const files = fs.readdirSync(srcPages)
  .filter(f => f.endsWith('AccountsPage.tsx'))
  .map(f => path.join(srcPages, f));

files.push(path.join(__dirname, 'src', 'components', 'codebuddy-suite', 'CodebuddySuiteAccountsSharedView.tsx'));

for (const file of files) {
  let content = fs.readFileSync(file, 'utf8');

  // We are looking for something like:
  //           )}
  //           groupByTag ? ...
  // or
  //           groupByOrigin ? ...
  
  // Actually, we can just look for the injection we made:
  //           )}
  //           groupByTag ? (
  
  if (content.includes('          )}\n          groupByTag ? (')) {
    content = content.replace(
      '          )}\n          groupByTag ? (',
      '          )}\n          {groupByTag ? ('
    );
    // we also need to close the curly brace at the end of the ternary branch.
    // The previous structure was:
    // groupByTag ? ( <div...>...</div> ) : (<div ...>...</div>)
    // 
    // And it is immediately followed by:
    //         </div>
    //       ) : ...
    
    // So we can replace the closing part:
    const searchEndStr = ')\n        </div>\n      ) : groupByTag ? (';
    if (content.includes(searchEndStr)) {
      content = content.replace(searchEndStr, ')}\n        </div>\n      ) : groupByTag ? (');
    } else {
      // maybe `) : groupByOrigin`
      content = content.replace(')\n        </div>\n      ) : groupByOrigin ? (', ')}\n        </div>\n      ) : groupByOrigin ? (');
    }
    
    // some files might just be `) : (table...`
    // Let's use robust regex
    const fixRegex = /\)\n\s*<\/div>\n\s*\)\s*:\s*(?:groupByTag|groupByOrigin)\s*\?/g;
    content = content.replace(fixRegex, (match) => {
      return match.replace(/^\)\n/, ')}\n');
    });

    fs.writeFileSync(file, content, 'utf8');
    console.log('Fixed ' + file);
  } else if (content.includes('          )}\n          groupByOrigin ? (')) {
    content = content.replace(
      '          )}\n          groupByOrigin ? (',
      '          )}\n          {groupByOrigin ? ('
    );
     const fixRegex = /\)\n\s*<\/div>\n\s*\)\s*:\s*(?:groupByTag|groupByOrigin)\s*\?/g;
    content = content.replace(fixRegex, (match) => {
      return match.replace(/^\)\n/, ')}\n');
    });
    fs.writeFileSync(file, content, 'utf8');
    console.log('Fixed ' + file);
  }
}
