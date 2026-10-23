import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}
function harness(autoPopup: boolean) {
  const popup = { id: 'fixture', title: 'Fixture announcement', summary: 'Fixture', content: 'Text', type: 'info',
    createdAt: new Date().toISOString(), images: [{ url: 'https://example.invalid/image.png' }] };
  let traps: any[] = [];
  let escapes: any[] = [];
  const store = { state: { announcements: [popup], unreadIds: [], popupAnnouncement: autoPopup ? popup : null },
    loading: false, fetchState: async () => {}, markAsRead: async () => {}, markAllAsRead: async () => {} };
  const h = loadHookModule(new URL('./AnnouncementCenter.tsx', import.meta.url), {
    'react-dom': { createPortal: (tree: any) => tree },
    '@tauri-apps/plugin-opener': { openUrl: async () => {} },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../stores/useAnnouncementStore': { useAnnouncementStore: (selector: any) => selector(store) },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean, _restore = true, active = true) => traps.push({ ref, open, active }) },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => escapes.push({ open, close, ref }) },
  }, { document: { body: {}, querySelector: () => null, addEventListener() {}, removeEventListener() {} },
    window: { addEventListener() {}, removeEventListener() {}, setInterval: () => 1, clearInterval() {} } });
  h.render(() => {
    traps = []; escapes = [];
    const surface = autoPopup ? h.exports.AnnouncementHost({ onNavigate() {} }) : h.exports.AnnouncementCenter({ onNavigate() {} });
    return surface.type(surface.props);
  });
  return { h, nodes() { return nodes(h.flush()); },
    active() { const active = traps.filter((trap) => trap.open && trap.active); assert.equal(active.length, 1); return active[0]; },
    esc() { const active = escapes.filter((entry) => entry.open); assert.equal(active.length, 1); assert.equal(active[0].ref, traps.find((trap) => trap.open && trap.active)?.ref); active[0].close(); },
    async settle() { await settlePromises(); h.flush(); },
  };
}

test('the automatic startup announcement registers the visible root and keeps its image preview above the parent', async () => {
  const h = harness(true);
  let dialog = h.nodes().find((node) => node.props?.role === 'dialog');
  assert.equal(dialog.props.ref, h.active().ref);
  assert.equal(dialog.props.tabIndex, -1);
  assert.equal(dialog.props['aria-modal'], 'true');
  const overlay = h.nodes().find((node) => node.props?.className === 'modal-overlay announcement-modal-overlay');
  assert.equal(overlay.props.onClick, undefined);
  h.nodes().find((node) => node.type === 'img').props.onClick();
  dialog = h.nodes().find((node) => node.props?.className === 'announcement-image-preview-wrapper');
  assert.equal(dialog.props.ref, h.active().ref);
  h.esc(); await h.settle();
  assert.equal(h.nodes().filter((node) => node.props?.role === 'dialog').length, 1);
  assert.equal(h.nodes().find((node) => node.props?.role === 'dialog').props.ref, h.active().ref);
  h.esc(); await h.settle();
  assert.equal(h.nodes().filter((node) => node.props?.role === 'dialog').length, 0);
  h.h.unmount();
});

test('opening the announcement list uses the same focus wiring as startup details', () => {
  const h = harness(false);
  h.nodes().find((node) => node.type === 'button').props.onClick();
  const dialog = h.nodes().find((node) => node.props?.role === 'dialog');
  assert.equal(dialog.props.ref, h.active().ref);
  assert.equal(dialog.props.tabIndex, -1);
  h.esc();
  assert.equal(h.nodes().filter((node) => node.props?.role === 'dialog').length, 0);
  h.h.unmount();
});
