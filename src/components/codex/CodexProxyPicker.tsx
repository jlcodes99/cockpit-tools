import { useEffect, useMemo, useState } from 'react';
import { Gauge, Pin, Server } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { catalogErrorKey, catalogGroupKindKey, catalogGroupNodes, catalogLatencyCandidates, catalogSelectors, catalogUnsupportedKey, isCatalogBlockingMember,
  type ProxyCatalog, type ProxyCatalogGroup, type ProxyCatalogSelections, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { proxySelectionGroup } from '../../utils/codexProxySelection';
import { proxyPickerTarget } from '../../utils/codexProxyPickerModel';
import { proxyRuntimeLabelKey, proxyRuntimeRows } from '../../utils/codexProxyPreview';
import type { useProxyLatency } from './useProxyLatency';
import { CodexProxySelect } from './CodexProxySelect';
import { CodexProxyLatencyBadge } from './CodexProxyLatencyBadge';
import { CodexProxySelectionIssues } from './CodexProxySelectionIssues';
import { ModalErrorMessage } from '../ModalErrorMessage';

export function CodexProxyPicker({ source, itemId, selectedGroupId, selections, busy, latency, choose, chooseMember, onCatalogChange, onPendingChange, runtimeStatus }: {
  source: ProxyCatalogSource; itemId: string; selectedGroupId?: string; selections: ProxyCatalogSelections; busy: boolean; latency: ReturnType<typeof useProxyLatency>;
  choose: (id: string, groupId: string) => void; chooseMember: (groupId: string, memberName: string) => void;
  onCatalogChange?: (catalog: ProxyCatalog) => void;
  onPendingChange?: (pending: boolean) => void;
  /** Only pass a runtime whose saved binding still matches the editor's draft. */
  runtimeStatus?: CodexProxyRuntimeStatus | null;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const groupId = proxySelectionGroup(source, itemId, selectedGroupId);
  const group = source.groups.find((entry) => entry.id === groupId);
  const selectedGroup = source.groups.find((entry) => entry.id === itemId);
  const node = source.nodes.find((entry) => entry.id === itemId);
  const selectedItem = selectedGroup ?? node;
  const target = proxyPickerTarget(source, itemId, selections);
  const targetNode = target && 'protocol' in target ? target : undefined;
  const policyGroup = target && 'kind' in target ? target : undefined;
  const strategy = source.kind === 'strategy';
  const blocked = busy || pending;
  const locked = blocked || latency.running;
  const policyBadge = (entry: ProxyCatalogGroup) => <span className="codex-picker-badge" title={strategy ? t('codex.proxy.catalog.strategyTag') : undefined}>{t(catalogGroupKindKey(entry.kind))}</span>;
  const hintKey = policyGroup?.kind === 'select' ? 'codex.proxy.catalog.selectGroupHint'
    : policyGroup?.kind === 'url-test' ? 'codex.proxy.catalog.autoGroupHint'
      : policyGroup ? 'codex.proxy.catalog.policyGroupHint' : 'codex.proxy.catalog.selectionHint';
  const ids = useMemo(() => group ? catalogGroupNodes(source, group.id, true) : source.nodes.map((entry) => entry.id), [source, group]);
  const testIds = useMemo(() => group ? catalogLatencyCandidates(source, group.id) : [], [source, group]);
  const manualGroups = useMemo(() => catalogSelectors(source, itemId, selections), [source, itemId, selections]);
  // Restore a draft without probing. Only explicit selection and test handlers start checks.
  useEffect(() => latency.cancel, [source.id, source.revision, busy, latency.cancel]);
  const measureSelection = (id: string, parentId?: string) => {
    latency.cancel();
    const candidates = catalogLatencyCandidates(source, id);
    const context = source.groups.some((entry) => entry.id === id) ? id : parentId;
    // Empty selections also switch the displayed cache, without starting a request.
    latency.measure(candidates, true, context || undefined);
  };
  const selectedBlockingMember = manualGroups.some((selector) => selector.members.includes(selections[selector.id]) && isCatalogBlockingMember(selections[selector.id]));
  let selectionErrorKey = itemId && !selectedItem?.supported ? catalogUnsupportedKey(selectedItem) : '';
  for (const selector of manualGroups) {
    const name = selections[selector.id];
    if (selectionErrorKey || !name || isCatalogBlockingMember(name)) continue;
    const member = source.nodes.find((entry) => entry.name === name) ?? source.groups.find((entry) => entry.name === name);
    if (!selector.members.includes(name) || !member?.supported || name === selector.name) selectionErrorKey = catalogUnsupportedKey(member, name);
  }
  // Specific member and certificate issues are rendered once, beside the selection.
  if (targetNode?.error || (policyGroup?.issues?.length ?? 0) > 0) selectionErrorKey = '';
  const badge = (id: string) => <CodexProxyLatencyBadge result={latency.results[id]} />;
  const delay = (id: string) => { const result = latency.results[id]; return result?.status === 'success' ? result.value.latencyMs : undefined; };
  const nodeDetail = (entry: ProxyCatalogSource['nodes'][number]) => entry.supported ? entry.protocol.toUpperCase() : t(catalogUnsupportedKey(entry));
  const groupMeasure = (id: string) => {
    const members = catalogLatencyCandidates(source, id);
    return { label: t('codex.proxy.catalog.measureGroup', { count: members.length }), disabled: locked || !members.length, run: () => latency.measure(members, false, id) };
  };
  const nodeMeasure = (id: string, context?: string) => ({ label: t('codex.proxy.catalog.check'), disabled: locked || !source.nodes.find((entry) => entry.id === id)?.supported, run: () => latency.measure([id], false, context || undefined) });
  const cancelCheck = latency.running && <button type="button" className="btn btn-secondary compact" onClick={latency.cancel}>{t('codex.proxy.cancelCheck')}</button>;
  const matchingRuntime = runtimeStatus?.effectiveProxy?.sourceId === source.id && runtimeStatus.effectiveProxy.itemId === itemId ? runtimeStatus : null;
  const liveRows = proxyRuntimeRows(matchingRuntime).filter((row) => (row.kernelState ?? row.state) === 'running' && row.node);
  const liveNames = [...new Set(liveRows.map((row) => row.node!))];
  const targetResult = targetNode ? latency.results[targetNode.id] : undefined;
  const selectionName = selectedBlockingMember ? manualGroups.map((selector) => selections[selector.id]).find((name) => isCatalogBlockingMember(name ?? '')) : target?.name;
  return <div className="codex-picker codex-picker-native">
    <div className="codex-picker-current">
      <Server size={20} className="codex-picker-current-icon" />
      <div className="codex-picker-current-content">
        <span className="codex-picker-current-caption">{t(liveNames.length ? 'codex.proxy.currentNode' : 'codex.proxy.catalog.selectedExit')}</span>
        {liveNames.length ? liveNames.map((name) => {
          const rows = liveRows.filter((row) => row.node === name);
          const selection = rows.flatMap((row) => row.selection ? [row.selection] : []).sort((a, b) => (b.checkedAt ?? 0) - (a.checkedAt ?? 0))[0];
          const runningNode = source.nodes.find((entry) => entry.name === name);
          return <div className="codex-picker-live-node" key={name}>
            <div><strong title={name}>{name}</strong><small>{runningNode?.protocol.toUpperCase()}{runningNode?.udp === true && <span className="codex-picker-protocol-tag">UDP</span>}
              {liveNames.length > 1 && <> · {rows.map((row) => t(proxyRuntimeLabelKey(row.kind))).join(' · ')}</>}</small></div>
            <CodexProxyLatencyBadge result={selection?.delayMs != null && selection.checkedAt != null
              ? { status: 'success', value: { latencyMs: selection.delayMs, checkedAt: selection.checkedAt } } : undefined} />
          </div>;
        }) : <div className="codex-picker-live-node"><div><strong title={selectionName}>{selectionName || t('codex.proxy.catalog.chooseMember')}</strong>
          <small>{policyGroup ? policyBadge(policyGroup) : targetNode ? <><span>{targetNode.protocol.toUpperCase()}</span>{targetNode.udp === true && <span className="codex-picker-protocol-tag">UDP</span>}<span className="codex-picker-fixed"><Pin size={11} />{t('codex.proxy.catalog.fixedNode')}</span></> : null}</small>
          {policyGroup && policyGroup.kind !== 'select' && <small>{t('codex.proxy.catalog.runtimeNodePending')}</small>}</div>{targetNode && <CodexProxyLatencyBadge result={targetResult} />}</div>}
      </div>
    </div>
    <div className="codex-picker-row"><CodexProxySelect label={t('codex.proxy.catalog.groups')} placeholder={t('codex.proxy.catalog.groups')} value={groupId} disabled={blocked}
      searchPlaceholder={strategy ? t('codex.proxy.catalog.strategyFilter') : undefined}
      options={[{ value: '', label: t('codex.proxy.catalog.nodes'), pinned: true }, ...source.groups.map((entry) => ({ value: entry.id, label: entry.name, badge: policyBadge(entry), detail: entry.supported ? undefined : t(catalogUnsupportedKey(entry)) }))]}
      onChange={(id) => { measureSelection(id); choose(id, id); }} />
      <button type="button" className="btn btn-secondary compact codex-picker-icon-button"
        aria-label={t('codex.proxy.catalog.measureGroup', { count: testIds.length })} title={t('codex.proxy.catalog.measureGroup', { count: testIds.length })}
        disabled={locked || !testIds.length} onClick={() => latency.measure(testIds, false, groupId)}><Gauge size={17} /></button>
    </div>
    {manualGroups.map((selector) => <div className="codex-picker-row" key={selector.id}>
      <CodexProxySelect label={selector.id === itemId ? t('codex.proxy.catalog.nodes') : selector.name} placeholder={t('codex.proxy.catalog.chooseMember')} value={selections[selector.id] ?? ''} disabled={blocked}
        sortable measuring={latency.running} footer={cancelCheck}
        options={selector.members.map((name) => {
          const member = source.nodes.find((entry) => entry.name === name) ?? source.groups.find((entry) => entry.name === name);
          const nestedGroup = member && 'kind' in member ? member : undefined;
          return { value: name, label: name, badge: nestedGroup ? policyBadge(nestedGroup) : member ? badge(member.id) : undefined,
            delay: member && 'protocol' in member ? delay(member.id) : undefined,
            measure: member ? nestedGroup ? groupMeasure(member.id) : nodeMeasure(member.id, selector.id) : undefined,
            detail: member && 'protocol' in member ? nodeDetail(member) : member?.supported ? undefined : t(catalogUnsupportedKey(member, name)),
            disabled: name === selector.name || (!nestedGroup && !member?.supported && !('insecure' in (member ?? {}) && (member as { insecure?: boolean }).insecure) && !isCatalogBlockingMember(name)) };
        })}
        onChange={(name) => {
          const member = source.nodes.find((entry) => entry.name === name) ?? source.groups.find((entry) => entry.name === name);
          measureSelection(member?.id ?? '', selector.id); chooseMember(selector.id, name);
        }} />
    </div>)}
    {!(group?.kind === 'select' && itemId === group.id) && <div className="codex-picker-row"><CodexProxySelect label={t('codex.proxy.catalog.nodes')} placeholder={t(group ? 'codex.proxy.catalog.groupPolicyChoice' : 'codex.proxy.catalog.nodes')} value={selectedGroup && selectedGroup.id !== groupId ? selectedGroup.id : node?.id ?? ''} disabled={blocked}
      sortable measuring={latency.running} footer={cancelCheck}
      options={[
        ...(group ? [{ value: '', label: t('codex.proxy.catalog.groupPolicyChoice'), badge: policyBadge(group), pinned: true }] : []),
        ...(group ? source.groups.filter((child) => group.members.includes(child.name) && child.id !== group.id).map((child) => ({ value: child.id, label: child.name, badge: policyBadge(child), measure: groupMeasure(child.id), detail: child.supported ? undefined : t(catalogUnsupportedKey(child)) })) : []),
        ...source.nodes.filter((entry) => ids.includes(entry.id)).map((entry) => ({ value: entry.id, label: entry.name, detail: nodeDetail(entry), measure: nodeMeasure(entry.id, groupId), delay: delay(entry.id), badge: badge(entry.id), disabled: !entry.supported && !entry.insecure }))
      ]} onChange={(id) => { measureSelection(id || groupId, groupId); choose(id || groupId, groupId); }} />
    </div>}
    <p className="codex-proxy-page-note">{t(hintKey)}</p>
    {selectedBlockingMember && <div className="codex-picker-network-hint" role="status"><span>{t('codex.proxy.catalog.blockingMemberHint')}</span></div>}
    <ModalErrorMessage message={selectionErrorKey ? t(selectionErrorKey) : null} className="codex-picker-selection-error" scrollKey={`${itemId}:${JSON.stringify(selections)}`} />
    <ModalErrorMessage message={latency.errorKey ? t(latency.errorKey) : null} />
    <CodexProxySelectionIssues source={source} target={target} busy={busy} onCatalogChange={onCatalogChange}
      onPendingChange={(value) => { if (value) latency.cancel(); setPending(value); onPendingChange?.(value); }} />
    {targetResult?.status === 'error' && <details className="codex-picker-issue-details"><summary>{t('codex.proxy.catalog.checkDetails')}</summary>
      <p>{t(catalogErrorKey(targetResult.error))}</p></details>}
    <div className="codex-picker-footer"><details className="codex-picker-help"><summary>{t('codex.proxy.catalog.latencyAbout')}</summary><p>{t('codex.proxy.catalog.latencyNotice')}</p></details>{cancelCheck}</div>
  </div>;
}
