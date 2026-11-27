import './WorkbuddyCodexNativeModal.css';
import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Loader2, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import * as codexService from '../../services/codexService';
import type { CodexWorkbuddyDelegateCatalog, CodexWorkbuddyDelegateProfile } from '../../types/codex';
import { getWorkbuddyAccountDisplayEmail, type WorkbuddyAccount } from '../../types/workbuddy';
import { buildWorkbuddyAccountPresentation } from '../../presentation/platformAccountPresentation';

interface Props {
  accounts: WorkbuddyAccount[];
  initialAccountId?: string | null;
  onClose: () => void;
}

/** A fixed-source opt-in. The legacy text-only account action is unchanged. */
export function WorkbuddyCodexNativeModal({ accounts, initialAccountId, onClose }: Props) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const request = useRef(0);
  const active = useRef(true);
  const [accountId, setAccountId] = useState(initialAccountId ?? accounts[0]?.id ?? '');
  const [catalog, setCatalog] = useState<CodexWorkbuddyDelegateCatalog | null>(null);
  const [modelId, setModelId] = useState('');
  const [effort, setEffort] = useState('');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [profiles, setProfiles] = useState<CodexWorkbuddyDelegateProfile[]>([]);
  const [revokedProfileIds, setRevokedProfileIds] = useState<Set<string>>(() => new Set());
  const selectedExists = accounts.some((account) => account.id === accountId);
  const error = useModalErrorState();
  const close = () => { if (!busy) onClose(); };
  useEscCloseTopmost(true, close, dialog);

  const describeError = (value: unknown) => {
    const code = String(value);
    if (code.includes('NATIVE_SERVICE_REQUIRED')) return t('workbuddy.nativeCodex.nativeRequired', '请先在 API 服务页面创建并启用 Native Rust 服务；不会自动切换模式。');
    if (code.includes('ENTERPRISE_UNSUPPORTED')) return t('workbuddy.nativeCodex.enterpriseUnsupported', '首版尚不能严格绑定企业租户，仅支持个人 WorkBuddy 来源。');
    if (code.includes('KEY_REVOKED_FILES_PRESERVED')) return t('workbuddy.nativeCodex.revokedFilesPreserved', '专属 Key 已撤销。配置文件已被修改或无法安全清理，所有修改文件与会话数据均已保留；可检查文件后重试恢复。');
    if (code.includes('PROFILE_NOT_OWNED')) return t('workbuddy.nativeCodex.notOwned', '配置所有权凭证与专属 Key 不匹配，未修改任何 Key。请检查配置目录的归属标记。');
    if (code.includes('PROFILE_CHANGED')) return t('workbuddy.nativeCodex.profileChanged', '该独立配置已被修改，程序不会覆盖你的修改。');
    if (code.includes('MODEL_OR_EFFORT_UNAVAILABLE')) return t('workbuddy.nativeCodex.selectionChanged', '模型或推理强度已不可用，请重新读取目录并选择。');
    if (code.includes('SOURCE_UNAVAILABLE')) return t('workbuddy.nativeCodex.sourceUnavailable', '该 WorkBuddy 来源不可用，请检查登录状态。不会尝试其他账号。');
    return t('workbuddy.nativeCodex.operationFailed', '操作失败，请检查 WorkBuddy CLI 是否已安装、账号是否有效，并重新读取目录。');
  };

  const refreshProfiles = async () => {
    try {
      const result = await codexService.listCodexWorkbuddyDelegateProfiles();
      if (active.current) setProfiles(result);
    } catch (cause) { if (active.current) error.report(describeError(cause)); }
  };

  const loadCatalog = async (source: string, selectedModel?: string) => {
    const sequence = ++request.current;
    error.clear();
    setLoading(true);
    setEffort('');
    try {
      const result = await codexService.getCodexWorkbuddyDelegateCatalog(source, selectedModel);
      if (!active.current || request.current !== sequence) return;
      setCatalog(result);
      setModelId(result.currentModelId);
      // This is a visible initial selection from the exact model's live options,
      // not a translation of an unsupported user-selected effort.
      setEffort(result.currentReasoningEffort && result.reasoningEfforts.includes(result.currentReasoningEffort)
        ? result.currentReasoningEffort : result.reasoningEfforts[0] ?? '');
    } catch (cause) {
      if (active.current && request.current === sequence) {
        setCatalog(null);
        error.report(describeError(cause));
      }
    } finally {
      if (active.current && request.current === sequence) setLoading(false);
    }
  };

  useEffect(() => {
    active.current = true;
    void refreshProfiles();
    return () => { active.current = false; request.current += 1; };
  }, []);

  useEffect(() => {
    setCatalog(null);
    setModelId('');
    setEffort('');
    if (accountId && selectedExists) void loadCatalog(accountId);
    else setLoading(false);
    return () => { request.current += 1; };
  }, [accountId, selectedExists]);

  const create = async () => {
    if (busy || loading || !accountId || !modelId || !effort) return;
    setBusy(true);
    error.clear();
    try {
      const profile = await codexService.createCodexWorkbuddyDelegateProfile(accountId, modelId, effort);
      if (active.current) setProfiles((current) => [...current, profile]);
    } catch (cause) {
      if (active.current) {
        error.report(describeError(cause));
        // A service-start error may follow a committed isolated profile. Keep
        // the managed restore entry reachable instead of hiding the transaction.
        void refreshProfiles();
      }
    } finally { if (active.current) setBusy(false); }
  };

  const restore = async (profile: CodexWorkbuddyDelegateProfile) => {
    if (busy) return;
    setBusy(true);
    error.clear();
    try {
      await codexService.restoreCodexWorkbuddyDelegateProfile(profile.id);
      if (active.current) setProfiles((current) => current.filter((item) => item.id !== profile.id));
    } catch (cause) {
      if (active.current) {
        if (String(cause).includes('KEY_REVOKED_FILES_PRESERVED')) {
          setRevokedProfileIds((current) => new Set([...current, profile.id]));
        }
        error.report(describeError(cause));
      }
    }
    finally { if (active.current) setBusy(false); }
  };

  const selected = accounts.find((account) => account.id === accountId);
  const enterprise = Boolean(selected?.enterprise_id);
  const planBadges = useMemo(() => {
    const map = new Map<string, { label: string; className: string }>();
    for (const account of accounts) {
      const { planLabel, planClass } = buildWorkbuddyAccountPresentation(account, t);
      if (planLabel) {
        map.set(account.id, { label: planLabel, className: planClass });
      }
    }
    return map;
  }, [accounts, t]);
  const accountOptions = useMemo(
    () =>
      accounts.map((account) => {
        const badge = planBadges.get(account.id);
        return {
          value: account.id,
          label: getWorkbuddyAccountDisplayEmail(account),
          badge: badge ? (
            <span className={`tier-badge account-picker-plan ${badge.className}`}>{badge.label}</span>
          ) : undefined,
        };
      }),
    [accounts, planBadges],
  );
  const modelOptions = useMemo(
    () =>
      (catalog?.models ?? []).map((model) => ({
        value: model.id,
        label: `${model.name} (${model.id})`,
      })),
    [catalog],
  );
  const effortOptions = useMemo(
    () =>
      (catalog?.reasoningEfforts ?? []).map((value) => ({
        value,
        label: value,
      })),
    [catalog],
  );
  const sourceUnavailable = (source?: WorkbuddyAccount) => !source
    || Boolean(source.enterprise_id)
    || (source.expires_at != null && source.expires_at <= Date.now())
    || (source.status != null && !['normal', 'active', 'ok'].includes(source.status));
  return createPortal(
    <div className="modal-overlay">
      <div ref={dialog} className="modal modal-content workbuddy-codex-native-modal" role="dialog" aria-modal="true" aria-labelledby="workbuddy-native-codex-title">
        <div className="modal-header">
          <h2 id="workbuddy-native-codex-title">{t('workbuddy.nativeCodex.title', 'WorkBuddy → Codex 原生工具桥')}</h2>
          <button type="button" className="modal-close" disabled={busy} onClick={close} aria-label={t('common.close', '关闭')}><X size={18} /></button>
        </div>
        <div className="modal-body">
          <p>{t('workbuddy.nativeCodex.scope', '创建固定来源的专属本地 Key 和独立 CODEX_HOME。Codex 自己执行工具；WorkBuddy Token 不会写入 Codex 配置，也不会修改默认登录或 API 服务主 Key。')}</p>
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
          <div className="form-group">
            <span className="workbuddy-codex-native-field-label">{t('workbuddy.nativeCodex.account', 'WorkBuddy 来源')}</span>
            <SingleSelectDropdown
              className="workbuddy-codex-native-select"
              ariaLabel={t('workbuddy.nativeCodex.account', 'WorkBuddy 来源')}
              value={selectedExists ? accountId : ''}
              placeholder={t('workbuddy.nativeCodex.noSourceSelected', '暂无可用来源；仍可管理已创建的配置')}
              options={accountOptions}
              disabled={busy || loading || accounts.length === 0}
              onChange={(nextAccountId) => setAccountId(nextAccountId)}
            />
          </div>
          {enterprise && <p role="note">{t('workbuddy.nativeCodex.enterpriseUnsupported', '首版尚不能严格绑定企业租户，仅支持个人 WorkBuddy 来源。')}</p>}
          <div className="form-group">
            <span className="workbuddy-codex-native-field-label">{t('workbuddy.nativeCodex.model', 'CLI 实时模型')}</span>
            <SingleSelectDropdown
              className="workbuddy-codex-native-select"
              ariaLabel={t('workbuddy.nativeCodex.model', 'CLI 实时模型')}
              value={modelId}
              placeholder={loading ? t('common.loading', '加载中…') : t('workbuddy.nativeCodex.readCatalog', '请读取实时目录')}
              options={modelOptions}
              disabled={busy || loading || !catalog}
              onChange={(nextModel) => {
                setModelId(nextModel);
                void loadCatalog(accountId, nextModel);
              }}
            />
          </div>
          <div className="form-group">
            <span className="workbuddy-codex-native-field-label">{t('workbuddy.nativeCodex.effort', '推理强度')}</span>
            <SingleSelectDropdown
              className="workbuddy-codex-native-select"
              ariaLabel={t('workbuddy.nativeCodex.effort', '推理强度')}
              value={effort}
              placeholder="—"
              options={effortOptions}
              disabled={busy || loading || !catalog}
              onChange={(nextEffort) => setEffort(nextEffort)}
            />
          </div>
          <p className="help-text">{t('workbuddy.nativeCodex.metadataOnly', '读取目录只初始化 CLI，不发送模型提示词。创建前会再次校验模型和强度；来源失效时直接报错，不回退其他账号。仅支持 HTTP Responses；独立配置已禁用联网搜索（web_search），不支持图片、其他托管工具、compact 或 WebSocket。')}</p>
          <button type="button" className="btn btn-secondary btn-sm" disabled={busy || loading || !accountId || !selectedExists} onClick={() => { void loadCatalog(accountId, modelId || undefined); }}>
            {loading ? <Loader2 size={14} className="spin" /> : <RefreshCw size={14} />}{t('workbuddy.nativeCodex.refreshCatalog', '重新读取目录')}
          </button>
          {profiles.map((profile) => {
            const source = accounts.find((account) => account.id === profile.workbuddyAccountId);
            const sourceBadge = source ? planBadges.get(source.id) : undefined;
            const sourceLabel = source ? getWorkbuddyAccountDisplayEmail(source) : profile.workbuddyAccountId;
            const revoked = revokedProfileIds.has(profile.id);
            return <section key={profile.id} style={{ marginTop: 20 }} aria-label={t('workbuddy.nativeCodex.managedProfile', '已创建的独立配置')}>
              <strong>{profile.publicModel} · {profile.reasoningEffort}</strong>
              <p className="workbuddy-codex-native-profile-source">
                <span>{t('workbuddy.nativeCodex.profileSource', '来源：')}</span>
                <span className="workbuddy-codex-native-source-email" title={sourceLabel}>{sourceLabel}</span>
                {sourceBadge && (
                  <span className={`tier-badge account-picker-plan ${sourceBadge.className}`}>
                    {sourceBadge.label}
                  </span>
                )}
              </p>
              {!source && <p role="note">{t('workbuddy.nativeCodex.profileSourceMissing', '来源已删除；仍可撤销该专属 Key 并恢复独立配置。')}</p>}
              {source && sourceUnavailable(source) && <p role="note">{t('workbuddy.nativeCodex.profileSourceUnavailable', '来源当前不可用；撤销和恢复不依赖来源登录状态。')}</p>}
              {revoked
                ? <p role="status">{t('workbuddy.nativeCodex.revokedFilesPreserved', '专属 Key 已撤销。配置文件已被修改或无法安全清理，所有修改文件与会话数据均已保留；可检查文件后重试恢复。')}</p>
                : <>
                  <p>{t('workbuddy.nativeCodex.launchHelp', '在终端进入你的项目目录，再运行下面的命令。仅该次 Codex 启动使用此配置。')}</p>
                  <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', userSelect: 'text' }}>{profile.launchCommand}</pre>
                </>}
              <p className="help-text">{profile.profileDir}</p>
              <button type="button" className="btn btn-secondary btn-sm" disabled={busy} onClick={() => { void restore(profile); }}>
                {revoked ? t('workbuddy.nativeCodex.retryRestore', '重试恢复（Key 已撤销）') : t('workbuddy.nativeCodex.restore', '撤销专属 Key 并恢复此配置')}
              </button>
            </section>;
          })}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" disabled={busy} onClick={close}>{t('common.close', '关闭')}</button>
          <button type="button" className="btn btn-primary" disabled={busy || loading || enterprise || !accountId || !modelId || !effort} onClick={() => { void create(); }}>
            {busy && <Loader2 size={14} className="spin" />}{t('workbuddy.nativeCodex.create', '创建专属 Key 与独立配置')}
          </button>
        </div>
      </div>
    </div>, document.body,
  );
}
