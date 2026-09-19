import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const controllerSource = readFileSync(
  `${process.cwd()}/src/pages/useCodexAccountsAccessController.tsx`,
  'utf8',
);
const overviewSource = readFileSync(
  `${process.cwd()}/src/pages/useCodexAccountsOverviewController.tsx`,
  'utf8',
);

function sliceBetween(source: string, startMarker: string, endMarker: string): string {
  const start = source.indexOf(startMarker);
  assert.notEqual(start, -1, `missing marker: ${startMarker}`);
  const end = source.indexOf(endMarker, start);
  assert.notEqual(end, -1, `missing marker: ${endMarker}`);
  return source.slice(start, end);
}

describe('codex CLI quick launch', () => {
  it('exposes the quick launch action inside the launch preview', () => {
    const actions = sliceBetween(
      overviewSource,
      'const buildAccountLaunchPreviewActions',
      'const buildLocalAccessLaunchPreviewSummary',
    );
    assert.ok(actions.includes('codex.cli.quickLaunch'));
    assert.ok(actions.includes('handleLaunchCodexCli(account)'));
  });

  it('enters the CLI modal directly when the preview is already open for that account', () => {
    const handler = sliceBetween(
      controllerSource,
      'const handleLaunchCodexCli',
      'const handleLaunchLocalAccessCli',
    );
    const shortcut = handler.indexOf('launchPreviewAccount?.id === account.id');
    const reopen = handler.indexOf('setLaunchPreviewAccount(account)');

    assert.notEqual(shortcut, -1);
    assert.notEqual(reopen, -1);
    // 捷径必须在重新打开预览之前返回，否则预览内的点击不会有任何反馈。
    assert.ok(shortcut < reopen);
    assert.ok(handler.includes('enterCodexCliLaunch(account)'));
  });

  it('shares one helper for both entry points into the CLI modal', () => {
    const helper = sliceBetween(
      controllerSource,
      'const enterCodexCliLaunch',
      'const launchPreviewInstanceOptions',
    );
    assert.ok(helper.includes('setPendingCliLaunchTarget('));
    assert.ok(helper.includes('setLaunchPreviewCliIntent(false)'));
    assert.ok(helper.includes('setLaunchPreviewAccount(null)'));

    const confirmBranch = sliceBetween(
      controllerSource,
      'if (launchPreviewCliIntent) {',
      'if (launchPreviewInstanceId !== DEFAULT_CODEX_INSTANCE_ID)',
    );
    assert.ok(confirmBranch.includes('enterCodexCliLaunch(launchAccount)'));
  });
});
