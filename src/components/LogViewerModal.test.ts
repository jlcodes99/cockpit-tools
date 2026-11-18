import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { LogSnapshot } from '../services/logService';

type Element = { type: unknown; props: { children?: unknown; onClick?: () => void } };

function nodes(tree: unknown): Element[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  if (typeof tree !== 'object' || tree === null || !('props' in tree)) return [];
  const node = tree as Element;
  return [node, ...nodes(node.props.children)];
}

test('unchanged log polls preserve visible and cleared output, while refresh fetches the full tail', async () => {
  const snapshot: LogSnapshot = {
    log_dir_path: '/logs',
    log_file_path: '/logs/app.log',
    log_file_name: 'app.log',
    content: 'first line\n',
    content_revision: 'first',
    line_limit: 200,
    file_size: 11,
    modified_at_ms: null,
    available_files: [],
  };
  const revisions: Array<string | undefined> = [];
  let poll: (() => void) | undefined;
  const h = loadHookModule(new URL('./LogViewerModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    './SingleSelectDropdown': { SingleSelectDropdown: 'select-dropdown' },
    '../hooks/useEscClose': { useEscClose() {} },
    '../hooks/useLogContentFilter': {
      useLogContentFilter: (_open: boolean, { content }: { content: string }) => ({ content }),
    },
    '../services/logService': {
      async getLogSnapshot(_name?: string, _limit?: number, known?: string) {
        revisions.push(known);
        return { ...snapshot, content: known === snapshot.content_revision ? null : snapshot.content };
      },
    },
  }, {
    window: {
      setInterval(callback: () => void) { poll = callback; return 1; },
      clearInterval() { poll = undefined; },
    },
  });
  const displayed = () => nodes(h.flush()).find(({ type }) => type === 'pre')?.props.children;
  const click = (label: string) => {
    const button = nodes(h.flush()).find(({ type, props }) =>
      type === 'button' && props.children === label);
    assert.ok(button?.props.onClick);
    button.props.onClick();
  };

  let open = true;
  h.render(() => h.exports.LogViewerModal({ open, onClose() {} }));
  await settlePromises();
  assert.equal(displayed(), 'first line\n');
  poll?.();
  await settlePromises();
  assert.equal(displayed(), 'first line\n');

  click('breakout.historyClear');
  poll?.();
  await settlePromises();
  assert.equal(displayed(), undefined);
  snapshot.content = 'first line\nsecond line\n';
  snapshot.content_revision = 'second';
  poll?.();
  await settlePromises();
  assert.equal(displayed(), 'second line\n');

  click('common.refresh');
  await settlePromises();
  assert.deepEqual(revisions, [undefined, 'first', 'first', 'first', undefined]);
  open = false;
  h.flush();
  assert.equal(poll, undefined);
  h.unmount();
});
