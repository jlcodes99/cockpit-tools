import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  Check,
  KeyRound,
  Pencil,
  PlugZap,
  RefreshCw,
  Server,
  Trash2,
  X,
} from 'lucide-react';
import { useSshServerStore } from '../../stores/useSshServerStore';
import type {
  SshAuthConfig,
  SshCodexSyncResult,
  SshCodexSyncStatus,
  SshServer,
  SshServerDraft,
} from '../../types/sshServer';

interface FormState {
  id?: string;
  name: string;
  host: string;
  port: string;
  username: string;
  codexHome: string;
  authKind: 'agent' | 'private_key_file';
  privateKeyPath: string;
  syncOnSwitch: boolean;
}

const emptyForm: FormState = {
  name: '',
  host: '',
  port: '22',
  username: '',
  codexHome: '~/.codex',
  authKind: 'agent',
  privateKeyPath: '',
  syncOnSwitch: true,
};

function formFromServer(server: SshServer): FormState {
  return {
    id: server.id,
    name: server.name,
    host: server.host,
    port: String(server.port || 22),
    username: server.username,
    codexHome: server.codex_home || '~/.codex',
    authKind: server.auth.kind,
    privateKeyPath: server.auth.kind === 'private_key_file' ? server.auth.path : '',
    syncOnSwitch: server.sync_on_codex_switch,
  };
}

function draftFromForm(form: FormState): SshServerDraft {
  const auth: SshAuthConfig =
    form.authKind === 'private_key_file'
      ? { kind: 'private_key_file', path: form.privateKeyPath.trim() }
      : { kind: 'agent' };
  return {
    id: form.id,
    name: form.name.trim(),
    host: form.host.trim(),
    port: Number.parseInt(form.port, 10) || 22,
    username: form.username.trim(),
    codex_home: form.codexHome.trim() || '~/.codex',
    auth,
    sync_on_codex_switch: form.syncOnSwitch,
  };
}

function formatSyncTime(timestamp?: number) {
  if (!timestamp) return '';
  return new Date(timestamp * 1000).toLocaleString();
}

function describeSyncStatus(t: TFunction, sync: SshCodexSyncStatus) {
  if (!sync.verified) {
    const completedStages = [
      sync.bundle_verified
        ? t('codex.ssh.stageBundle', '鉴权包哈希')
        : null,
      sync.model_provider_verified
        ? t('codex.ssh.stageProvider', 'Provider 配置')
        : null,
      sync.state_repair ? t('codex.ssh.stageState', '状态库') : null,
    ].filter((stage): stage is string => Boolean(stage));
    const error = sync.error ?? t('codex.ssh.syncFailed', 'SSH 同步失败');
    return completedStages.length > 0
      ? t(
          'codex.ssh.syncPartialFailure',
          '{{error}} · 已完成：{{stages}}',
          { error, stages: completedStages.join(' / ') },
        )
      : error;
  }
  const provider = sync.model_provider ?? 'openai';
  const state = sync.state_repair?.database_found
    ? `${t(
        'codex.ssh.stateRepaired',
        '状态库已对齐 {{rows}} 行（provider {{providerRows}}，可见性 {{visibilityRows}}），rollout 已修复 {{rolloutFiles}} 个，备份已校验',
        {
          rows: sync.state_repair.rows_repaired,
          providerRows: sync.state_repair.provider_rows_to_repair,
          visibilityRows: sync.state_repair.visibility_rows_to_repair,
          rolloutFiles: sync.state_repair.rollout_files_repaired,
          rolloutPaths: sync.state_repair.rollout_paths_repaired,
          userEvents: sync.state_repair.user_events_recovered,
          cwdRows: sync.state_repair.cwd_rows_repaired,
          orphanRollouts: sync.state_repair.orphan_rollouts_found,
          orphanThreads: sync.state_repair.orphan_threads_recovered,
        },
      )} · ${t(
        'codex.ssh.reconciliationSummary',
        '路径 {{rolloutPaths}} · 用户事件 {{userEvents}} · 工作目录 {{cwdRows}} · 孤立 rollout {{orphanRollouts}} / 已恢复 {{orphanThreads}}',
        {
          rolloutPaths: sync.state_repair.rollout_paths_repaired,
          userEvents: sync.state_repair.user_events_recovered,
          cwdRows: sync.state_repair.cwd_rows_repaired,
          orphanRollouts: sync.state_repair.orphan_rollouts_found,
          orphanThreads: sync.state_repair.orphan_threads_recovered,
        },
      )}`
    : t('codex.ssh.stateDatabaseMissing', '远端尚无状态库，无历史需要对齐');
  const quiesce = t(
    `codex.ssh.reloadStatus.${sync.app_server_quiesce_status ?? 'unknown'}`,
    sync.app_server_quiesce_status ?? 'unknown',
  );
  const restore = t(
    `codex.ssh.reloadStatus.${sync.app_server_restore_status ?? 'unknown'}`,
    sync.app_server_restore_status ?? 'unknown',
  );
  const reload = t(
    `codex.ssh.reloadStatus.${sync.app_server_reload_status ?? 'unknown'}`,
    sync.app_server_reload_status ?? 'unknown',
  );
  return t(
    'codex.ssh.syncVerifiedDetail',
    'Provider {{provider}} 已校验 · {{state}} · {{quiesce}} / {{restore}} / {{reload}}',
    { provider, state, quiesce, restore, reload },
  );
}

interface CodexSshServersPanelProps {
  /** 嵌入设置弹框时隐藏顶部大标题区 */
  embedded?: boolean;
}

export function CodexSshServersPanel({ embedded = false }: CodexSshServersPanelProps) {
  const { t } = useTranslation();
  const {
    servers,
    selectedServerId,
    loading,
    error,
    lastSyncResult,
    fetchServers,
    upsertServer,
    deleteServer,
    selectServer,
    testConnection,
    syncNow,
    applySyncResult,
  } = useSshServerStore();
  const [form, setForm] = useState<FormState>(emptyForm);
  const [saving, setSaving] = useState(false);
  const [busyServerId, setBusyServerId] = useState<string | null>(null);
  const [localMessage, setLocalMessage] = useState<{
    kind: 'success' | 'warning' | 'error';
    text: string;
  } | null>(null);

  const selectedServer = useMemo(
    () => servers.find((server) => server.id === selectedServerId) ?? null,
    [servers, selectedServerId],
  );

  useEffect(() => {
    void fetchServers();
  }, [fetchServers]);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    listen<SshCodexSyncResult>('codex:ssh-sync-result', (event) => {
      applySyncResult(event.payload);
      setLocalMessage({
        kind: event.payload.verified ? 'success' : 'warning',
        text: describeSyncStatus(t, event.payload),
      });
    }).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [applySyncResult, t]);

  const handleSubmit = async () => {
    setSaving(true);
    setLocalMessage(null);
    try {
      const draft = draftFromForm(form);
      await upsertServer(draft);
      // 开启「切号同步」时自动选中该服务器，设置里的开关会点亮
      if (draft.sync_on_codex_switch) {
        const list = useSshServerStore.getState().servers;
        const matched =
          list.find(
            (server) =>
              server.host === draft.host &&
              server.port === draft.port &&
              server.username === draft.username,
          ) ?? list.find((server) => server.id === draft.id);
        if (matched) {
          await selectServer(matched.id);
        }
      }
      setForm(emptyForm);
      setLocalMessage({ kind: 'success', text: t('common.saved', '已保存') });
    } catch (err) {
      setLocalMessage({ kind: 'error', text: String(err) });
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async (serverId: string) => {
    setBusyServerId(serverId);
    setLocalMessage(null);
    try {
      await testConnection(serverId);
      setLocalMessage({
        kind: 'success',
        text: t('codex.ssh.connectionOk', '连接成功'),
      });
    } catch (err) {
      setLocalMessage({ kind: 'error', text: String(err) });
    } finally {
      setBusyServerId(null);
    }
  };

  const handleSync = async (serverId?: string) => {
    setBusyServerId(serverId ?? selectedServerId);
    setLocalMessage(null);
    try {
      const result = await syncNow(serverId);
      setLocalMessage({
        kind: result.verified ? 'success' : 'warning',
        text: describeSyncStatus(t, result),
      });
    } catch (err) {
      setLocalMessage({ kind: 'error', text: String(err) });
    } finally {
      setBusyServerId(null);
    }
  };

  return (
    <div className={`codex-ssh-panel${embedded ? ' codex-ssh-panel--embedded' : ''}`}>
      {!embedded && (
        <div className="codex-ssh-panel__header">
          <div className="codex-ssh-panel__header-copy">
            <h2 className="codex-ssh-panel__title">
              {t('codex.ssh.title', 'SSH 服务器')}
            </h2>
            <p className="codex-ssh-panel__subtitle">
              {t(
                'codex.ssh.subtitle',
                '选中的服务器会在 Codex 切号后接收当前账号鉴权包。',
              )}
            </p>
          </div>
          <button
            className="btn btn-secondary"
            type="button"
            onClick={() => void fetchServers()}
            disabled={loading}
          >
            <RefreshCw size={14} className={loading ? 'spin' : undefined} />
            <span>{t('common.refresh', '刷新')}</span>
          </button>
        </div>
      )}

      {(error || localMessage) && (
        <div
          className={`codex-ssh-panel__message codex-ssh-panel__message--${localMessage?.kind ?? 'error'}`}
          role="status"
        >
          {localMessage?.text ?? error}
        </div>
      )}

      <div className="codex-ssh-panel__layout">
        <form
          className="codex-ssh-panel__form"
          onSubmit={(event) => {
            event.preventDefault();
            void handleSubmit();
          }}
        >
          <div className="codex-ssh-panel__form-header">
            <div>
              <div className="codex-ssh-panel__section-kicker">
                {form.id
                  ? t('codex.ssh.editServer', '编辑服务器')
                  : t('codex.ssh.addServer', '添加服务器')}
              </div>
              <h3 className="codex-ssh-panel__form-title">
                {t('codex.ssh.formLead', '填写远端主机信息')}
              </h3>
            </div>
            <div className="codex-ssh-panel__form-header-actions">
              {embedded && (
                <button
                  className="btn btn-secondary icon-only"
                  type="button"
                  title={t('common.refresh', '刷新')}
                  aria-label={t('common.refresh', '刷新')}
                  onClick={() => void fetchServers()}
                  disabled={loading}
                >
                  <RefreshCw size={14} className={loading ? 'spin' : undefined} />
                </button>
              )}
              {form.id && (
                <button
                  className="btn btn-secondary icon-only"
                  type="button"
                  title={t('common.cancel', '取消')}
                  aria-label={t('common.cancel', '取消')}
                  onClick={() => setForm(emptyForm)}
                >
                  <X size={14} />
                </button>
              )}
            </div>
          </div>

          <div className="codex-ssh-panel__fields">
            <label className="codex-ssh-panel__field">
              <span>{t('codex.ssh.name', '名称')}</span>
              <input
                className="form-input"
                value={form.name}
                onChange={(event) => setForm({ ...form, name: event.target.value })}
                placeholder={t('codex.ssh.namePlaceholder', '例如：家里 NAS')}
                required
              />
            </label>

            <div className="codex-ssh-panel__field-grid codex-ssh-panel__field-grid--host">
              <label className="codex-ssh-panel__field">
                <span>{t('codex.ssh.host', '主机')}</span>
                <input
                  className="form-input"
                  value={form.host}
                  onChange={(event) => setForm({ ...form, host: event.target.value })}
                  placeholder="192.168.1.10"
                  required
                />
              </label>
              <label className="codex-ssh-panel__field">
                <span>{t('codex.ssh.port', '端口')}</span>
                <input
                  className="form-input"
                  inputMode="numeric"
                  value={form.port}
                  onChange={(event) => setForm({ ...form, port: event.target.value })}
                  required
                />
              </label>
            </div>

            <div className="codex-ssh-panel__field-grid">
              <label className="codex-ssh-panel__field">
                <span>{t('codex.ssh.username', '用户名')}</span>
                <input
                  className="form-input"
                  value={form.username}
                  onChange={(event) =>
                    setForm({ ...form, username: event.target.value })
                  }
                  placeholder="ubuntu"
                  required
                />
              </label>
              <label className="codex-ssh-panel__field">
                <span>{t('codex.ssh.codexHome', '远端 Codex 目录')}</span>
                <input
                  className="form-input"
                  value={form.codexHome}
                  onChange={(event) =>
                    setForm({ ...form, codexHome: event.target.value })
                  }
                  placeholder="~/.codex"
                />
              </label>
            </div>

            <div className="codex-ssh-panel__auth">
              <span className="codex-ssh-panel__field-label">
                {t('codex.ssh.authMethod', '认证方式')}
              </span>
              <div
                className="codex-ssh-panel__segmented"
                role="radiogroup"
                aria-label={t('codex.ssh.authMethod', '认证方式')}
              >
                <button
                  type="button"
                  className={form.authKind === 'agent' ? 'active' : undefined}
                  onClick={() => setForm({ ...form, authKind: 'agent' })}
                >
                  {t('codex.ssh.agentAuth', 'SSH Agent')}
                </button>
                <button
                  type="button"
                  className={
                    form.authKind === 'private_key_file' ? 'active' : undefined
                  }
                  onClick={() =>
                    setForm({ ...form, authKind: 'private_key_file' })
                  }
                >
                  {t('codex.ssh.privateKeyAuth', '私钥文件')}
                </button>
              </div>
            </div>

            {form.authKind === 'private_key_file' && (
              <label className="codex-ssh-panel__field">
                <span>{t('codex.ssh.privateKeyPath', '私钥路径')}</span>
                <input
                  className="form-input"
                  value={form.privateKeyPath}
                  onChange={(event) =>
                    setForm({ ...form, privateKeyPath: event.target.value })
                  }
                  placeholder="~/.ssh/id_ed25519"
                  required
                />
              </label>
            )}

            <label className="codex-ssh-panel__switch-card">
              <span className="codex-ssh-panel__switch-copy">
                <strong>{t('codex.ssh.syncOnSwitch', '切号后自动同步')}</strong>
                <em>
                  {t(
                    'codex.ssh.syncOnSwitchHint',
                    '保存并选中后，Codex 切号会推送鉴权到这台主机',
                  )}
                </em>
              </span>
              <span className="codex-ssh-panel__switch">
                <input
                  type="checkbox"
                  checked={form.syncOnSwitch}
                  onChange={(event) =>
                    setForm({ ...form, syncOnSwitch: event.target.checked })
                  }
                />
                <span className="codex-ssh-panel__switch-slider" />
              </span>
            </label>
          </div>

          <div className="codex-ssh-panel__form-actions">
            {form.id && (
              <button
                className="btn btn-secondary"
                type="button"
                onClick={() => setForm(emptyForm)}
              >
                {t('common.cancel', '取消')}
              </button>
            )}
            <button className="btn btn-primary" type="submit" disabled={saving}>
              <Check size={14} />
              <span>
                {form.id
                  ? t('common.save', '保存')
                  : t('codex.ssh.addAndSave', '添加并保存')}
              </span>
            </button>
          </div>
        </form>

        <section className="codex-ssh-panel__list-wrap">
          <div className="codex-ssh-panel__list-header">
            <div>
              <div className="codex-ssh-panel__section-kicker">
                {t('codex.ssh.serverList', '服务器列表')}
              </div>
              <h3 className="codex-ssh-panel__list-title">
                {t('codex.ssh.serverListLead', '选择同步目标')}
              </h3>
            </div>
            <span className="codex-ssh-panel__count">
              {t('codex.ssh.serverCount', '{{count}} 台', {
                count: servers.length,
              })}
            </span>
          </div>

          <div className="codex-ssh-panel__list">
            {servers.length === 0 && (
              <div className="codex-ssh-panel__empty">
                <div className="codex-ssh-panel__empty-icon" aria-hidden="true">
                  <Server size={22} />
                </div>
                <h3>{t('codex.ssh.emptyTitle', '暂无 SSH 服务器')}</h3>
                <p>
                  {t(
                    'codex.ssh.emptyBody',
                    '在左侧填写主机信息并保存，即可作为切号同步目标。',
                  )}
                </p>
              </div>
            )}

            {servers.map((server) => {
              const isSelected = server.id === selectedServerId;
              const sync = server.last_sync;
              const busy = busyServerId === server.id;
              return (
                <article
                  className={`codex-ssh-panel__item${isSelected ? ' is-selected' : ''}`}
                  key={server.id}
                >
                  <button
                    type="button"
                    className="codex-ssh-panel__item-main"
                    onClick={() => void selectServer(isSelected ? null : server.id)}
                  >
                    <div className="codex-ssh-panel__item-top">
                      <div className="codex-ssh-panel__item-avatar" aria-hidden="true">
                        <Server size={16} />
                      </div>
                      <div className="codex-ssh-panel__item-copy">
                        <div className="codex-ssh-panel__item-title">
                          <strong>{server.name}</strong>
                          {isSelected && (
                            <span className="codex-ssh-panel__badge">
                              {t('codex.ssh.selected', '已选中')}
                            </span>
                          )}
                          {server.sync_on_codex_switch && (
                            <span className="codex-ssh-panel__badge codex-ssh-panel__badge--soft">
                              {t('codex.ssh.autoSyncBadge', '自动同步')}
                            </span>
                          )}
                        </div>
                        <div className="codex-ssh-panel__item-meta">
                          <code>
                            {server.username}@{server.host}:{server.port}
                          </code>
                          <span className="codex-ssh-panel__dot">·</span>
                          <span>{server.codex_home}</span>
                        </div>
                      </div>
                    </div>

                    <div
                      className={`codex-ssh-panel__sync-status${
                        sync?.verified ? ' is-ok' : sync ? ' is-fail' : ''
                      }`}
                    >
                      {sync
                        ? sync.verified
                          ? `${t('codex.ssh.syncedAs', '已同步 {{email}} · {{time}}', {
                              email: sync.account_email,
                              time: formatSyncTime(sync.synced_at),
                            })} · ${describeSyncStatus(t, sync)}`
                          : describeSyncStatus(t, sync)
                        : t('codex.ssh.neverSynced', '尚未同步')}
                    </div>
                  </button>

                  <div className="codex-ssh-panel__item-actions">
                    <button
                      className={`btn btn-secondary${isSelected ? ' is-active' : ''}`}
                      type="button"
                      disabled={busy}
                      onClick={() =>
                        void selectServer(isSelected ? null : server.id)
                      }
                    >
                      <Check size={14} />
                      <span>
                        {isSelected
                          ? t('codex.ssh.selected', '已选中')
                          : t('codex.ssh.select', '选中')}
                      </span>
                    </button>
                    <button
                      className="btn btn-secondary"
                      type="button"
                      disabled={busy}
                      onClick={() => void handleTest(server.id)}
                    >
                      <PlugZap size={14} />
                      <span>{t('codex.ssh.testConnection', '测连')}</span>
                    </button>
                    <button
                      className="btn btn-secondary"
                      type="button"
                      disabled={busy}
                      onClick={() => void handleSync(server.id)}
                    >
                      <KeyRound size={14} />
                      <span>{t('codex.ssh.syncNow', '同步')}</span>
                    </button>
                    <button
                      className="btn btn-secondary icon-only"
                      type="button"
                      title={t('common.edit', '编辑')}
                      aria-label={t('common.edit', '编辑')}
                      onClick={() => setForm(formFromServer(server))}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="btn btn-danger icon-only"
                      type="button"
                      title={t('common.delete', '删除')}
                      aria-label={t('common.delete', '删除')}
                      onClick={() => void deleteServer(server.id)}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </article>
              );
            })}
          </div>
        </section>
      </div>

      {selectedServer && lastSyncResult?.server_id === selectedServer.id && (
        <div
          className={`codex-ssh-panel__footer${
            lastSyncResult.verified ? ' is-ok' : ' is-fail'
          }`}
        >
          {describeSyncStatus(t, lastSyncResult)}
        </div>
      )}
    </div>
  );
}
