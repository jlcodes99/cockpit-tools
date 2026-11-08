import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { sshSyncSucceeded } from '../../utils/sshSyncStatus';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  if (!tree || typeof tree !== 'object' || !tree.props) return [];
  return [tree, ...nodes(tree.props.children)];
}

function fixture(file: string, exportName: string) {
  const state: any = {
    servers:['a','b'].map(id=>({id,name:id,host:'fixture.invalid',username:'fixture',port:22,
      codex_home:'~/.codex',auth:{kind:'agent'},sync_on_codex_switch:true,last_sync:null})),
    selectedServerIds:['a'],loading:false,error:null,lastSyncResult:null,
    async fetchServers(){}, async setServerSelected(id:string,selected:boolean){
      state.selectedServerIds=state.selectedServerIds.filter((item:string)=>item!==id);
      if(selected)state.selectedServerIds.push(id);
    },
    async selectServers(ids:string[]){state.selectedServerIds=ids;},
  };
  const t = (_key:string,fallback:string,params:Record<string,unknown>={}) =>
    fallback.replace(/{{(\w+)}}/g,(_match,key)=>String(params[key]??''));
  const module = loadHookModule(new URL(file,import.meta.url), {
    'react-i18next':{useTranslation:()=>({t})},
    '@tauri-apps/api/event':{listen:async()=>()=>{}},
    '../../stores/useSshServerStore':{useSshServerStore:(selector?:any)=>selector?selector(state):state},
    './CodexSshSyncModal':{CodexSshSyncModal:()=>null},
    '../../utils/sshSyncStatus':{sshSyncSucceeded},
  });
  const component=module.exports[exportName];
  return {state, module, render:()=>module.render(()=>component({embedded:true}))};
}

test('server cards independently toggle and render two selected targets at once', async () => {
  const s=fixture('./CodexSshServersPanel.tsx','CodexSshServersPanel');
  let tree=s.render();
  let cards=nodes(tree).filter(node=>node.type==='article');
  assert.equal(cards.filter(card=>card.props.className.includes('is-selected')).length,1);
  nodes(cards[1]).find(node=>node.props.className==='codex-ssh-panel__item-main').props.onClick();
  await settlePromises();
  tree=s.module.flush();
  cards=nodes(tree).filter(node=>node.type==='article');
  assert.equal(cards.filter(card=>card.props.className.includes('is-selected')).length,2);
  assert.equal(nodes(tree).find(node=>node.props.className==='codex-ssh-panel__count').props.children,'已选 2 / 2 台');
  nodes(cards[0]).find(node=>node.props.className==='codex-ssh-panel__item-main').props.onClick();
  await settlePromises();
  cards=nodes(s.module.flush()).filter(node=>node.type==='article');
  assert.equal(cards[0].props.className.includes('is-selected'),false);
  assert.equal(cards[1].props.className.includes('is-selected'),true);
  s.module.unmount();
});

test('the settings switch counts all automatic targets and clears all on disable', async () => {
  const s=fixture('./CodexSshSyncSettingsControl.tsx','CodexSshSyncSettingsControl');
  s.state.selectedServerIds=['a','b'];
  const tree=s.render();
  assert.equal(nodes(tree).find(node=>node.props.className==='row-desc').props.children,'已启用 · 2 台服务器');
  const toggle=nodes(tree).find(node=>node.type==='input'&&node.props.type==='checkbox');
  assert.equal(toggle.props.checked,true);
  toggle.props.onChange({target:{checked:false}});
  await settlePromises();
  assert.equal(nodes(s.module.flush()).find(node=>node.type==='input'&&node.props.type==='checkbox').props.checked,false);
  assert.equal(s.state.selectedServerIds.length,0);
  s.module.unmount();
});

test('a successful file write with a failed client refresh stays visible on its server card',()=>{
  const s=fixture('./CodexSshServersPanel.tsx','CodexSshServersPanel');
  s.state.servers[0].last_sync={verified:true,error:'ssh_connection_refresh_failed: account_mismatch'};
  const tree=s.render();
  const status=nodes(tree).find(node=>String(node.props.className).includes('codex-ssh-panel__sync-status'));
  assert.ok(status.props.className.includes('is-fail'));
  assert.ok(status.props.children.includes('鉴权已同步'));
  assert.ok(status.props.children.includes('重试'));
  s.module.unmount();
});
