import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../../tests/helpers/reactHookHarness';
import { PELICAN_BENCHMARKS } from './pelicanBenchmarks.ts';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
const t = (key: string) => key;

test('text replies open in full, HTML keeps its sandbox and failed replies retain retry controls', async () => {
  for (const kind of ['text', 'html', 'failed']) {
    const rawReply = `21\n<script>not executed</script>\n${'Explanation. '.repeat(250)}`;
    const html = kind === 'html' ? '<html><body>animation</body></html>' : null;
    const batch = { id: 'batch', prompt: PELICAN_BENCHMARKS.simple1, status: 'completed', items: [{ id: 'item', accountId: 'one', accountEmail: 'one@example.com',
      status: kind === 'failed' ? 'failed' : 'completed', hasHtml: !!html, error: kind === 'failed' ? 'upstream failure' : undefined }] };
    const observers: { deliver: (entries: unknown[]) => void }[] = [];
    const h = loadHookModule(new URL('./PelicanResults.tsx', import.meta.url), {
      'react-i18next': { useTranslation: () => ({ t }) },
      'react-dom': { createPortal: (value: unknown) => value },
      '../../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts: [] }) },
      '../../../stores/useCodexPelicanStore': {},
      '../../../services/codexPelicanService': { artifactPelican: async () => ({ rawReply, html }) },
      '../../../types/codexPelican': { isPelicanRunning: () => false },
      './PelicanAccountSummary': { PelicanPlanBadge: 'plan' },
      '../../TagEditModal': { TagEditModal: 'tag-editor' },
      '../../ModalErrorMessage': { ModalErrorMessage: 'error', useModalErrorState: () => ({ clear() {}, set() {} }) },
    }, {
      window: { document: { body: {} } },
      IntersectionObserver: class {
        constructor(public deliver: (entries: unknown[]) => void) { observers.push(this); }
        observe() {}
        disconnect() {}
      },
    });
    h.render(() => {
      const tree = h.exports.PelicanResults({ batch, mask: (value: string) => value });
      const item = nodes(tree).find((node) => typeof node.type === 'function' && node.type.name === 'PelicanItem');
      return item.type(item.props);
    });
    observers[0].deliver([{ isIntersecting: true }]);
    await settlePromises();
    const tree = nodes(h.flush());
    const score = tree.find((node) => typeof node.type === 'function' && node.type.name === 'PelicanScore');
    assert.equal(score.props.prompt, batch.prompt);
    assert.equal(score.props.reply, rawReply);
    assert.equal(score.props.status, batch.items[0].status);
    const text = tree.find((node) => node.props.className === 'pelican-canvas pelican-text-canvas');
    if (kind === 'text') {
      assert.equal(nodes(text).find((node) => node.type === 'span').props.children, rawReply.slice(0, 2000));
      assert.ok(!tree.some((node) => node.type === 'iframe' || node.type === 'script'));
      text.props.onClick();
      const details = nodes(h.flush());
      assert.equal(details.find((node) => node.type === 'pre').props.children, rawReply);
      assert.ok(details.find((node) => node.type === 'button' && Array.isArray(node.props.children)
        && node.props.children.includes('pelican.openBrowser')).props.disabled);
    } else if (kind === 'html') {
      assert.equal(text, undefined);
      assert.equal(tree.find((node) => node.type === 'iframe').props.sandbox, 'allow-scripts');
    } else {
      assert.equal(text, undefined);
      assert.ok(tree.some((node) => node.props.children === 'common.windowsOperation.retry'));
      assert.ok(tree.some((node) => node.props.children === 'pelican.viewError'));
    }
    h.unmount();
  }
});

test('result footers edit the selected live account and reflect saved tags without changing the batch', async () => {
  const state = {
    accounts: [
      { id: 'one', tags: ['good'] },
      { id: 'two', tags: [' fast ', 'GOOD'] },
    ],
    async updateAccountTags(id: string, tags: string[]) {
      saved.push({ id, tags: Array.from(tags) });
      state.accounts = state.accounts.map((account) => account.id === id ? { ...account, tags } : account);
    },
  };
  const saved: { id: string; tags: string[] }[] = [];
  const batch = { id: 'batch', status: 'completed', items: [
    { id: 'result-one', accountId: 'one', accountEmail: 'one@example.com', status: 'completed' },
    { id: 'result-two', accountId: 'two', accountEmail: 'two@example.com', status: 'failed' },
    { id: 'deleted', accountId: 'deleted', status: 'completed' },
  ] };
  const batchBefore = JSON.stringify(batch);
  const h = loadHookModule(new URL('./PelicanResults.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    'react-dom': { createPortal: (value: unknown) => value },
    '../../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select(state) },
    '../../../stores/useCodexPelicanStore': {},
    '../../../services/codexPelicanService': {},
    '../../../types/codexPelican': { isPelicanRunning: () => false },
    './PelicanAccountSummary': { PelicanPlanBadge: 'plan' },
    '../../TagEditModal': { TagEditModal: 'tag-editor' },
    '../../ModalErrorMessage': { ModalErrorMessage: 'error' },
  }, { window: { document: { body: {} } } });
  h.render(() => h.exports.PelicanResults({ batch, mask: (value: string) => `masked:${value}` }));
  const buttons = () => nodes(h.flush()).filter((node) => node.props.className === 'card-action-btn');
  const editor = () => nodes(h.flush()).find((node) => node.type === 'tag-editor');
  assert.equal(buttons().length, 2, 'deleted accounts do not offer an unusable edit action');
  assert.equal(editor(), undefined);
  buttons()[1].props.onClick();
  assert.equal(editor().props.resetKey, 'two');
  assert.deepEqual(Array.from(editor().props.initialTags), [' fast ', 'GOOD']);
  assert.deepEqual(Array.from(editor().props.availableTags), ['fast', 'good']);
  await editor().props.onSave(['fast', 'reviewed']);
  assert.deepEqual(saved, [{ id: 'two', tags: ['fast', 'reviewed'] }]);
  const tiles = nodes(h.flush()).filter((node) => node.type === 'article');
  assert.ok(nodes(tiles[1]).some((node) => node.props.className === 'tag-pill' && node.props.children === 'reviewed'));
  assert.ok(nodes(tiles[0]).some((node) => node.props.className === 'tag-pill' && node.props.children === 'good'));
  editor().props.onClose();
  assert.equal(editor(), undefined);
  assert.equal(JSON.stringify(batch), batchBefore);
  h.unmount();
});
