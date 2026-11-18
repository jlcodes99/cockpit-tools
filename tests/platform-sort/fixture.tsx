import React, { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { PlatformLayoutModal } from '../../src/components/PlatformLayoutModal';
import { SideNav } from '../../src/components/layout/SideNav';
import { usePlatformLayoutStore } from '../../src/stores/usePlatformLayoutStore';
import { useSideNavLayoutStore } from '../../src/stores/useSideNavLayoutStore';
import type { Page } from '../../src/types/navigation';
import '../../src/App.css';
const snapshot = { values: {}, revisions: {} };
let writes = 0, closed = 0;
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main' } },
  transformCallback: () => 1, unregisterCallback: () => {},
  invoke: async (command, args) => {
    if (command === 'load_ui_preferences') return structuredClone(snapshot);
    if (command === 'update_ui_preference') {
      snapshot.values[args.key] = args.value;
      snapshot.revisions[args.key] = (snapshot.revisions[args.key] || 0) + 1;
      if (args.key === 'agtools.platform_layout.v1') writes++;
      return structuredClone(snapshot);
    }
    if (command === 'plugin:event|listen') return 1;
    return {};
  },
};
usePlatformLayoutStore.getState().upsertPlatformGroup({
  id: 'fixture-sort-group', name: 'Fixture group',
  platformIds: ['qoder', 'codebuddy', 'codebuddy_cn'], defaultPlatformId: 'qoder',
  iconKind: 'platform', iconPlatformId: 'qoder',
});
const fixtureCodexGroup = usePlatformLayoutStore.getState().platformGroups.find((group) => group.platformIds.includes('codex_api_service'))!;
window.fixture = {
  get writes() { return writes; },
  get closed() { return closed; },
  get order() { return usePlatformLayoutStore.getState().orderedEntryIds; },
  get groupOrder() { return usePlatformLayoutStore.getState().platformGroups.find(g => g.id === 'fixture-sort-group').platformIds; },
  setMode: useSideNavLayoutStore.getState().setMode,
  setSidebarEntry: usePlatformLayoutStore.getState().setSidebarEntry,
  prepareSidebar() {
    const store = usePlatformLayoutStore.getState();
    store.upsertPlatformGroup({
      ...fixtureCodexGroup,
      childConfigs: [{
        platformId: 'codex_api_service',
        name: 'My API Service',
        iconKind: 'custom',
        iconCustomDataUrl: 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7',
      }],
    });
    for (const id of usePlatformLayoutStore.getState().sidebarEntryIds) {
      if (id !== `group:${fixtureCodexGroup.id}`) store.setSidebarEntry(id, false);
    }
  },
};
function Fixture() {
  const [open, setOpen] = useState(true);
  const [page, setPage] = useState<Page>('codex');
  return <>
    {!open && <div data-page={page}>
      <SideNav page={page} setPage={setPage} sponsorEntryVisible={false}
        onOpenPlatformLayout={() => setOpen(true)} onOpenLogViewer={() => {}}
        easterEggClickCount={0} onEasterEggTriggerClick={() => {}} hasBreakoutSession={false}
        updateActionState="hidden" updateProgress={0} onUpdateActionClick={() => {}}
        updateRemindersEnabled={false} />
    </div>}
    <PlatformLayoutModal open={open} onClose={() => { closed++; setOpen(false); }} />
  </>;
}
createRoot(document.getElementById('root')!).render(<React.StrictMode><Fixture /></React.StrictMode>);
