import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const source = (relative: string) => readFileSync(new URL(`../${relative}`, import.meta.url), 'utf8');

test('WorkBuddy AI OAuth uses the dedicated browser command, not the default opener', () => {
  const page = source('src/pages/WorkbuddyAiAccountsPage.tsx');
  const service = source('src/services/workbuddyAiService.ts');
  assert.match(page, /openAuthUrl: workbuddyAiService\.openWorkbuddyAiOAuthFreshBrowser/);
  assert.match(service, /invoke\('workbuddy_ai_oauth_open_fresh_browser', \{ authUrl \}\)/);
  assert.match(source('src-tauri/src/lib.rs'), /commands::workbuddy_ai::workbuddy_ai_oauth_open_fresh_browser,/);
});

test('isolated browser uses a fresh profile and direct argument-based launch', () => {
  const browser = source('src-tauri/src/modules/workbuddy_ai_auth_browser.rs');
  assert.match(browser, /uuid::Uuid::new_v4\(\)/);
  assert.match(browser, /--user-data-dir=/);
  assert.match(browser, /profile_arg\.push\(profile_dir\.as_os_str\(\)\)/);
  assert.match(browser, /--incognito/);
  assert.match(browser, /--inprivate/);
  assert.match(browser, /Command::new\(executable\)/);
  assert.doesNotMatch(browser, /taskkill|pkill|opener::|open_url\(/);
});

test('browser lifetime is tied to the exact login and browser generation', () => {
  const oauth = source('src-tauri/src/modules/workbuddy_ai_oauth.rs');
  assert.match(oauth, /state\.auth_url != auth_url/);
  assert.match(oauth, /state\.login_id == login_id && !state\.cancelled/);
  assert.match(oauth, /state\.browser_generation\.as_deref\(\) == Some\(generation\.as_str\(\)\)/);
  assert.match(source('src-tauri/src/lib.rs'), /modules::workbuddy_ai_auth_browser::shutdown\(\)/);
});

test('account import checks current login before committing old network responses', () => {
  const command = source('src-tauri/src/commands/workbuddy_ai.rs');
  assert.match(command, /commit_login\(&login_id, \|\| \{\s*workbuddy_ai_account::upsert_account\(payload\)/);
  assert.match(source('src-tauri/src/modules/workbuddy_ai_oauth.rs'), /state\.login_id != login_id \|\| state\.cancelled/);
});
