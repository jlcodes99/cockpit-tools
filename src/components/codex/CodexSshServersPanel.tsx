import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Check, KeyRound, Pencil, PlugZap, RefreshCw, Server, Trash2, X } from 'lucide-react';
import { useSshServerStore } from '../../stores/useSshServerStore';
import type { SshAuthConfig, SshCodexSyncResult, SshServer, SshServerDraft } from '../../types/sshServer';

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

export function CodexSshServersPanel() {
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
  const [localMessage, setLocalMessage] = useState<{ kind: 'success' | 'warning' | 'error'; text: string } | null>(null);

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
        text: event.payload.verified
          ? t('codex.ssh.syncVerified', 'SSH server verified with the switched Codex account.')
          : event.payload.error ?? t('codex.ssh.syncFailed', 'Local switch succeeded, but SSH sync failed.'),
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
      await upsertServer(draftFromForm(form));
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
      setLocalMessage({ kind: 'success', text: t('codex.ssh.connectionOk', 'SSH connection verified.') });
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
        text: result.verified
          ? t('codex.ssh.syncVerified', 'SSH server verified with the current Codex account.')
          : result.error ?? t('codex.ssh.syncFailed', 'SSH sync failed.'),
      });
    } catch (err) {
      setLocalMessage({ kind: 'error', text: String(err) });
    } finally {
      setBusyServerId(null);
    }
  };

  return (
    <div className="codex-ssh-panel">
      <div className="codex-ssh-toolbar">
        <div>
          <h2>{t('codex.ssh.title', 'SSH servers')}</h2>
          <p>{t('codex.ssh.subtitle', 'Selected servers receive the active Codex auth bundle after account switches.')}</p>
        </div>
        <button className="btn secondary" type="button" onClick={() => void fetchServers()} disabled={loading}>
          <RefreshCw size={16} />
          {t('common.refresh', '刷新')}
        </button>
      </div>

      {(error || localMessage) && (
        <div className={`codex-ssh-message ${localMessage?.kind ?? 'error'}`}>
          {localMessage?.text ?? error}
        </div>
      )}

      <div className="codex-ssh-layout">
        <form className="codex-ssh-form" onSubmit={(event) => { event.preventDefault(); void handleSubmit(); }}>
          <div className="codex-ssh-form-header">
            <h3>{form.id ? t('codex.ssh.editServer', 'Edit server') : t('codex.ssh.addServer', 'Add server')}</h3>
            {form.id && (
              <button className="btn icon-only" type="button" title={t('common.cancel', '取消')} onClick={() => setForm(emptyForm)}>
                <X size={16} />
              </button>
            )}
          </div>
          <label>
            {t('codex.ssh.name', 'Name')}
            <input value={form.name} onChange={(event) => setForm({ ...form, name: event.target.value })} required />
          </label>
          <div className="codex-ssh-form-grid">
            <label>
              {t('codex.ssh.host', 'Host')}
              <input value={form.host} onChange={(event) => setForm({ ...form, host: event.target.value })} required />
            </label>
            <label>
              {t('codex.ssh.port', 'Port')}
              <input inputMode="numeric" value={form.port} onChange={(event) => setForm({ ...form, port: event.target.value })} required />
            </label>
          </div>
          <label>
            {t('codex.ssh.username', 'Username')}
            <input value={form.username} onChange={(event) => setForm({ ...form, username: event.target.value })} required />
          </label>
          <label>
            {t('codex.ssh.codexHome', 'Codex home')}
            <input value={form.codexHome} onChange={(event) => setForm({ ...form, codexHome: event.target.value })} />
          </label>
          <div className="codex-ssh-auth-row">
            <label>
              <input type="radio" checked={form.authKind === 'agent'} onChange={() => setForm({ ...form, authKind: 'agent' })} />
              {t('codex.ssh.agentAuth', 'SSH agent')}
            </label>
            <label>
              <input type="radio" checked={form.authKind === 'private_key_file'} onChange={() => setForm({ ...form, authKind: 'private_key_file' })} />
              {t('codex.ssh.privateKeyAuth', 'Private key')}
            </label>
          </div>
          {form.authKind === 'private_key_file' && (
            <label>
              {t('codex.ssh.privateKeyPath', 'Private key path')}
              <input value={form.privateKeyPath} onChange={(event) => setForm({ ...form, privateKeyPath: event.target.value })} required />
            </label>
          )}
          <label className="codex-ssh-checkbox">
            <input type="checkbox" checked={form.syncOnSwitch} onChange={(event) => setForm({ ...form, syncOnSwitch: event.target.checked })} />
            {t('codex.ssh.syncOnSwitch', 'Sync after Codex account switches')}
          </label>
          <button className="btn primary" type="submit" disabled={saving}>
            <Check size={16} />
            {t('common.save', '保存')}
          </button>
        </form>

        <div className="codex-ssh-server-list">
          {servers.length === 0 && (
            <div className="empty-state">
              <Server className="empty-icon" />
              <h3>{t('codex.ssh.emptyTitle', 'No SSH servers')}</h3>
              <p>{t('codex.ssh.emptyBody', 'Add a server to sync Codex auth to a remote machine.')}</p>
            </div>
          )}
          {servers.map((server) => {
            const isSelected = server.id === selectedServerId;
            const sync = server.last_sync;
            return (
              <div className={`codex-ssh-server-card ${isSelected ? 'selected' : ''}`} key={server.id}>
                <div className="codex-ssh-server-main">
                  <div className="codex-ssh-server-title">
                    <Server size={18} />
                    <strong>{server.name}</strong>
                    {isSelected && <span>{t('codex.ssh.selected', 'Selected')}</span>}
                  </div>
                  <div className="codex-ssh-server-meta">{server.username}@{server.host}:{server.port} · {server.codex_home}</div>
                  <div className={`codex-ssh-sync-status ${sync?.verified ? 'verified' : sync ? 'failed' : ''}`}>
                    {sync
                      ? sync.verified
                        ? t('codex.ssh.syncedAs', 'Synced as {{email}} at {{time}}', {
                            email: sync.account_email,
                            time: formatSyncTime(sync.synced_at),
                          })
                        : sync.error ?? t('codex.ssh.syncFailed', 'SSH sync failed.')
                      : t('codex.ssh.neverSynced', 'Not synced yet')}
                  </div>
                </div>
                <div className="codex-ssh-server-actions">
                  <button className="btn icon-only" type="button" title={t('codex.ssh.select', 'Select')} onClick={() => void selectServer(isSelected ? null : server.id)}>
                    <Check size={16} />
                  </button>
                  <button className="btn icon-only" type="button" title={t('codex.ssh.testConnection', 'Test connection')} disabled={busyServerId === server.id} onClick={() => void handleTest(server.id)}>
                    <PlugZap size={16} />
                  </button>
                  <button className="btn icon-only" type="button" title={t('codex.ssh.syncNow', 'Sync now')} disabled={busyServerId === server.id} onClick={() => void handleSync(server.id)}>
                    <KeyRound size={16} />
                  </button>
                  <button className="btn icon-only" type="button" title={t('common.edit', '编辑')} onClick={() => setForm(formFromServer(server))}>
                    <Pencil size={16} />
                  </button>
                  <button className="btn icon-only danger" type="button" title={t('common.delete', '删除')} onClick={() => void deleteServer(server.id)}>
                    <Trash2 size={16} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {selectedServer && lastSyncResult?.server_id === selectedServer.id && (
        <div className={`codex-ssh-footer-status ${lastSyncResult.verified ? 'verified' : 'failed'}`}>
          {lastSyncResult.verified
            ? t('codex.ssh.latestVerified', 'Latest SSH sync verified.')
            : lastSyncResult.error ?? t('codex.ssh.syncFailed', 'SSH sync failed.')}
        </div>
      )}
    </div>
  );
}
