import { useCallback, useEffect, useId, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { createPortal } from "react-dom";
import { Database, HardDrive, List, RefreshCw, Trash2, X } from "lucide-react";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import { useModalScrollLock } from "../../hooks/useModalScrollLock";
import { CodexAccountPlanBadge } from "./CodexAccountPlanBadge";
import { useTranslation } from "react-i18next";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import { isCodexApiKeyAccount } from "../../types/codex";
import { resolveCodexAccountDisplayName } from "../../utils/codexAccountDisplayName";
import { isPrivacyModeEnabledByDefault, maskSensitiveValue } from "../../utils/privacy";
import { subscribeTauriEvent } from "../../utils/tauriEventListener";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import "./CodexQuotaHistorySettings.css";

type Settings = {
  enabled: boolean;
  intervalMinutes: number;
  retentionDays: number;
  autoCleanup: boolean;
  cleanupEveryDays: number;
  accountIds: string[] | null;
  onlyChanges: boolean;
  defaultAccountId: string | null;
};
type NumericSetting = "intervalMinutes" | "retentionDays" | "cleanupEveryDays";
type Draft = Omit<Settings, NumericSetting> & Record<NumericSetting, string>;
type Stats = { count: number; bytes: number };
type CleanupPreview = {
  range: { accountId: string | null; identity: string | null; from: number; to: number };
  count: number;
};
type CleanupKind = "expired" | "dates" | "all";
const NUMERIC_LIMITS: Record<NumericSetting, number> = {
  intervalMinutes: 1440,
  retentionDays: 3650,
  cleanupEveryDays: 3650,
};

const key = (name: string) => `codex.quotaHistory.storage.${name}`;
const validNumber = (value: string, max: number) => /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= max;
const toDraft = (settings: Settings): Draft => ({
  ...settings,
  intervalMinutes: String(settings.intervalMinutes),
  retentionDays: String(settings.retentionDays),
  cleanupEveryDays: String(settings.cleanupEveryDays),
});

export type CodexQuotaHistorySettingsVariant = "button" | "quick-settings" | "settings-row";

export function CodexQuotaHistorySettings({
  variant = "button",
}: {
  variant?: CodexQuotaHistorySettingsVariant;
} = {}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const dialog = open && <QuotaHistoryStorageControls onClose={() => setOpen(false)} />;

  if (variant === "quick-settings") {
    return <>
      <div className="qs-row">
        <div className="qs-row-label">
          <Database size={15} aria-hidden="true" />
          <span>{t(key("title"))}</span>
        </div>
        <div className="qs-row-control">
          <button
            type="button"
            className="qs-btn"
            aria-haspopup="dialog"
            aria-expanded={open}
            onClick={() => setOpen(true)}
          >
            {t("nav.settings")}
          </button>
        </div>
      </div>
      {dialog}
    </>;
  }

  if (variant === "settings-row") {
    return <>
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">{t(key("title"))}</div>
        </div>
        <div className="row-control">
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            aria-haspopup="dialog"
            aria-expanded={open}
            onClick={() => setOpen(true)}
          >
            {t("nav.settings")}
          </button>
        </div>
      </div>
      {dialog}
    </>;
  }

  return <section className="quota-history-storage">
    <button type="button" className="btn btn-secondary quota-history-storage__entry"
      aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(true)}>
      <Database size={15} aria-hidden="true" />{t(key("title"))}
    </button>
    {dialog}
  </section>;
}

function QuotaHistoryStorageControls({ onClose }: { onClose: () => void }) {
  const { t, i18n } = useTranslation();
  const titleId = useId();
  const lang = i18n.resolvedLanguage || i18n.language;
  const accounts = useCodexAccountStore((state) => state.accounts);
  const [saved, setSaved] = useState<Draft>();
  const [draft, setDraft] = useState<Draft>();
  const [stats, setStats] = useState<Stats>();
  const [statsFailed, setStatsFailed] = useState(false);
  const [settingsFailed, setSettingsFailed] = useState(false);
  const [busy, setBusy] = useState<"save" | "preview" | "delete">();
  const [settingsError, setSettingsError] = useState(false);
  const [cleanupError, setCleanupError] = useState(false);
  const [notice, setNotice] = useState<"saved" | number>();
  const [cleanupAccount, setCleanupAccount] = useState("");
  const [cleanupKind, setCleanupKind] = useState<CleanupKind>("expired");
  const [fromDate, setFromDate] = useState("");
  const [toDate, setToDate] = useState("");
  const [preview, setPreview] = useState<CleanupPreview>();
  const operation = useRef(false);
  const dialog = useRef<HTMLDivElement>(null);
  useEscCloseTopmost(true, onClose, dialog);
  useModalFocusTrap(dialog, true);
  useModalScrollLock(true);
  const latestSettings = useRef(0);
  const latestStats = useRef(0);
  const settingsErrorRef = useRef<HTMLParagraphElement>(null);
  const cleanupErrorRef = useRef<HTMLParagraphElement>(null);
  const eligible = accounts.filter((account) => !isCodexApiKeyAccount(account));
  const privacy = isPrivacyModeEnabledByDefault();
  const accountOptions = eligible.map((account) => ({
    value: account.id,
    badge: <CodexAccountPlanBadge account={account} />,
    label: maskSensitiveValue(resolveCodexAccountDisplayName(account, t("common.apiKeyAccount")), privacy),
  }));
  const loadSettings = useCallback(async () => {
    const sequence = ++latestSettings.current;
    setSettingsFailed(false);
    try {
      const value = toDraft(await invoke<Settings>("codex_quota_history_settings"));
      if (sequence !== latestSettings.current) return;
      setDraft(value);
      setSaved(value);
      setPreview(undefined);
    } catch {
      if (sequence === latestSettings.current) setSettingsFailed(true);
    }
  }, []);
  const loadStats = useCallback(async () => {
    const sequence = ++latestStats.current;
    setStatsFailed(false);
    try {
      const value = await invoke<Stats>("codex_quota_history_stats");
      if (sequence === latestStats.current) setStats(value);
    } catch {
      if (sequence === latestStats.current) setStatsFailed(true);
    }
  }, []);
  useEffect(() => {
    const store = useCodexAccountStore.getState();
    if (!store.accountsLoaded && !store.loading) void store.fetchAccounts();
    const stop = subscribeTauriEvent("codex:quota-history-settings-changed", () => {
      if (!operation.current) void loadSettings();
    });
    void loadSettings();
    void loadStats();
    return () => {
      latestSettings.current++;
      latestStats.current++;
      stop();
    };
  }, [loadSettings, loadStats]);
  useEffect(() => {
    if (settingsError) settingsErrorRef.current?.scrollIntoView({ block: "nearest" });
    if (cleanupError) cleanupErrorRef.current?.scrollIntoView({ block: "nearest" });
  }, [settingsError, cleanupError]);

  const updateDraft = (change: Partial<Draft>) => {
    setDraft((value) => value && { ...value, ...change });
    setSettingsError(false);
    setNotice(undefined);
  };
  const clearPreview = () => {
    setPreview(undefined);
    setCleanupError(false);
    setNotice(undefined);
  };
  const run = async (kind: NonNullable<typeof busy>, action: () => Promise<void>) => {
    if (operation.current) return;
    operation.current = true;
    setBusy(kind);
    setSettingsError(false);
    setCleanupError(false);
    setNotice(undefined);
    try {
      await action();
    } catch {
      if (kind === "save") setSettingsError(true);
      else {
        setCleanupError(true);
        setPreview(undefined);
        if (kind === "delete") void loadStats();
      }
    } finally {
      operation.current = false;
      setBusy(undefined);
    }
  };
  const valid = draft && (Object.keys(NUMERIC_LIMITS) as NumericSetting[])
    .every((field) => validNumber(draft[field], NUMERIC_LIMITS[field]));
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved);
  const from = new Date(`${fromDate}T00:00:00`).getTime();
  const end = new Date(`${toDate}T00:00:00`);
  const to = end.setDate(end.getDate() + 1);
  const validDates = from >= 0 && from < to && from < Date.now();
  const number = new Intl.NumberFormat(lang);
  const size = stats && new Intl.NumberFormat(lang, {
    style: "unit", unit: stats.bytes >= 1_000_000 ? "megabyte" : "kilobyte", maximumFractionDigits: 1,
  }).format(stats.bytes / (stats.bytes >= 1_000_000 ? 1_000_000 : 1000));
  const checkbox = (name: "enabled" | "onlyChanges" | "autoCleanup", extraDisabled = false) =>
    <label className="quota-history-storage__check">
      <input type="checkbox" role="switch" checked={draft?.[name] ?? false} disabled={!!busy || extraDisabled}
        onChange={(event) => updateDraft({ [name]: event.target.checked })} />
      <span className="quota-history-storage__switch-track" aria-hidden="true" />
      <span>{t(key(name))}</span>
    </label>;
  const numericField = (field: NumericSetting, extraDisabled = false) => {
    if (!draft) return null;
    const max = NUMERIC_LIMITS[field];
    const invalid = !validNumber(draft[field], max);
    return <label className="quota-history-storage__field">
      <span>{t(key(field))}</span>
      <input type="text" inputMode="numeric" value={draft[field]} aria-invalid={invalid}
        disabled={!!busy || extraDisabled} onChange={(event) => updateDraft({ [field]: event.target.value })} />
      {invalid && <span role="alert">{t(key("validRange"), { max })}</span>}
    </label>;
  };
  const save = () => {
    if (!draft) return;
    void run("save", async () => {
      latestSettings.current++;
      const settings = await invoke<Settings>("set_codex_quota_history_settings", { settings: {
        ...draft,
        intervalMinutes: Number(draft.intervalMinutes),
        retentionDays: Number(draft.retentionDays),
        cleanupEveryDays: Number(draft.cleanupEveryDays),
      } });
      const next = toDraft(settings);
      setSaved(next);
      setDraft(next);
      setPreview(undefined);
      setNotice("saved");
    });
  };

  return createPortal(<div className="modal-overlay quota-history-storage-overlay">
    <div className="modal quota-history-storage-dialog" role="dialog" aria-modal="true"
      aria-labelledby={titleId} ref={dialog} tabIndex={-1}>
      <header className="modal-header quota-history-storage__header">
        <div className="quota-history-storage__heading">
          <span className="quota-history-storage__icon"><Database size={20} aria-hidden="true" /></span>
          <h2 id={titleId}>{t(key("title"))}</h2>
        </div>
        <button type="button" className="modal-close" aria-label={t("common.close")} onClick={onClose}>
          <X size={20} aria-hidden="true" />
        </button>
      </header>
      <div className="modal-body quota-history-storage__body">
        <div className="quota-history-storage__summary">
          {stats ? <>
            <div className="quota-history-storage__metric">
              <List size={17} aria-hidden="true" />
              <div><span>{t(key("recordCount"))}</span><strong>{number.format(stats.count)}</strong></div>
            </div>
            <div className="quota-history-storage__metric">
              <HardDrive size={17} aria-hidden="true" />
              <div><span>{t(key("diskSpace"))}</span><strong>{size}</strong></div>
            </div>
          </> : !statsFailed && <span role="status">{t("common.loading")}</span>}
          {statsFailed && <span role="alert">{t(key("loadFailed"))}</span>}
          <button type="button" className="btn btn-secondary btn-sm quota-history-storage__refresh" disabled={!!busy}
            aria-label={t("common.refresh")} title={t("common.refresh")} onClick={() => void loadStats()}>
            <RefreshCw size={16} aria-hidden="true" />
          </button>
        </div>
        {settingsFailed && <div className="quota-history-storage__row" role="alert">
          {t(key("loadFailed"))}
          <button type="button" className="btn btn-secondary btn-sm" onClick={() => void loadSettings()}>
            {t("common.retry")}
          </button>
        </div>}
        {!draft && !settingsFailed && <p role="status">{t("common.loading")}</p>}
        {draft && <>
          <section className="quota-history-storage__section">
            <h3>{t(key("defaultAccount"))}</h3>
            <SingleSelectDropdown className="quota-history-storage__select"
              value={eligible.find((account) => account.id === draft.defaultAccountId)?.id ?? ""}
              disabled={!!busy} ariaLabel={t(key("defaultAccount"))}
              options={[{ value: "", label: t("instances.form.followCurrent") }, ...accountOptions]}
              onChange={(value) => updateDraft({ defaultAccountId: value || null })} />
          </section>
          <section className="quota-history-storage__section">
            <h3>{t(key("recording"))}</h3>
            <div className="quota-history-storage__switches">
              {checkbox("enabled")}
              {checkbox("onlyChanges", !draft.enabled)}
            </div>
            <div className="quota-history-storage__grid">
              {numericField("intervalMinutes", !draft.enabled)}
              <div className="quota-history-storage__field">
                <span>{t(key("recordScope"))}</span>
                <SingleSelectDropdown className="quota-history-storage__select"
                  value={draft.accountIds === null ? "all" : "selected"} disabled={!!busy || !draft.enabled}
                  ariaLabel={t(key("recordScope"))} options={[
                    { value: "all", label: t(key("allAccounts")) },
                    { value: "selected", label: t(key("selectedAccounts")) },
                  ]} onChange={(value) => updateDraft({ accountIds: value === "all" ? null : [] })} />
              </div>
            </div>
            {draft.accountIds !== null && <div className="quota-history-storage__accounts"
              role="group" aria-label={t(key("selectedAccounts"))}>
              {accountOptions.length === 0 && <span>{t("codex.quotaHistory.noAccount")}</span>}
              {accountOptions.map((account) => <label className="quota-history-storage__check" key={account.value}>
                <input type="checkbox" checked={draft.accountIds?.includes(account.value)}
                  disabled={!!busy || !draft.enabled} onChange={(event) => {
                    const selected = draft.accountIds ?? [];
                    updateDraft({ accountIds: event.target.checked
                      ? [...selected, account.value] : selected.filter((id) => id !== account.value) });
                  }} />
                <span className="quota-history-storage__account-name" title={account.label}>{account.label}</span>{account.badge}
              </label>)}
            </div>}
          </section>
          <section className="quota-history-storage__section">
            {checkbox("autoCleanup")}
            <div className="quota-history-storage__grid">
              {numericField("retentionDays", !draft.autoCleanup)}
              {numericField("cleanupEveryDays", !draft.autoCleanup)}
            </div>
          </section>
        </>}
        <section className="quota-history-storage__section quota-history-storage__cleanup">
          <h3><Trash2 size={15} aria-hidden="true" />{t(key("cleanup"))}</h3>
          <div className="quota-history-storage__grid">
            <div className="quota-history-storage__field">
              <span>{t(key("cleanupAccounts"))}</span>
              <SingleSelectDropdown className="quota-history-storage__select" value={cleanupAccount}
                ariaLabel={t(key("cleanupAccounts"))} disabled={!!busy}
                options={[{ value: "", label: t(key("allAccounts")) }, ...accountOptions]}
                onChange={(value) => { clearPreview(); setCleanupAccount(value); }} />
            </div>
            <div className="quota-history-storage__field">
              <span>{t(key("cleanupRange"))}</span>
              <SingleSelectDropdown className="quota-history-storage__select" value={cleanupKind}
                ariaLabel={t(key("cleanupRange"))} disabled={!!busy}
                options={(["expired", "dates", "all"] as const).map((kind) => ({ value: kind, label: t(key(kind)) }))}
                onChange={(value) => { clearPreview(); setCleanupKind(value as CleanupKind); }} />
            </div>
          </div>
          {cleanupKind === "dates" && <div className="quota-history-storage__grid">
            <label className="quota-history-storage__field">{t(key("fromDate"))}
              <input type="date" value={fromDate} disabled={!!busy}
                onChange={(event) => { clearPreview(); setFromDate(event.target.value); }} />
            </label>
            <label className="quota-history-storage__field">{t(key("toDate"))}
              <input type="date" value={toDate} disabled={!!busy}
                onChange={(event) => { clearPreview(); setToDate(event.target.value); }} />
            </label>
            {fromDate && toDate && !validDates && <span role="alert" className="quota-history-storage__error quota-history-storage__date-error">
              {t(key("invalidDates"))}
            </span>}
          </div>}
          {cleanupError && <p className="quota-history-storage__error" role="alert" ref={cleanupErrorRef}>
            {t(key("cleanupFailed"))}
          </p>}
          {preview ? <div className="quota-history-storage__row quota-history-storage__preview">
            <span role="status">{t(key("previewCount"), { records: number.format(preview.count) })}</span>
            <div className="quota-history-storage__preview-actions">
              <button type="button" className="btn btn-secondary btn-sm" disabled={!!busy} onClick={clearPreview}>{t("common.cancel")}</button>
              <button type="button" className="btn btn-danger btn-sm" disabled={!!busy || preview.count === 0}
                onClick={() => void run("delete", async () => {
                  const count = await invoke<number>("codex_quota_history_cleanup", { range: preview.range });
                  setPreview(undefined);
                  setNotice(count);
                  void loadStats();
                })}>{t(busy === "delete" ? "common.loading" : key("confirmCleanup"))}</button>
            </div>
          </div> : <div className="quota-history-storage__row">
            <button type="button" className="btn btn-secondary btn-sm"
              disabled={!!busy || (cleanupKind === "dates" && !validDates)}
              onClick={() => void run("preview", async () => {
                setPreview(await invoke<CleanupPreview>("codex_quota_history_cleanup_preview", {
                  accountId: cleanupAccount || null,
                  selection: cleanupKind === "dates" ? { kind: "dates", from, to } : { kind: cleanupKind },
                }));
              })}>{t(busy === "preview" ? "common.loading" : key("preview"))}</button>
            {typeof notice === "number" && <span role="status">{t(key("deleted"), { records: number.format(notice) })}</span>}
          </div>}
        </section>
        {settingsError && <p className="quota-history-storage__error" role="alert" ref={settingsErrorRef}>{t(key("saveFailed"))}</p>}
      </div>
      <footer className="modal-footer quota-history-storage__footer">
        {notice === "saved" && <span role="status">{t(key("saved"))}</span>}
        <button type="button" className="btn btn-secondary" onClick={onClose}>{t(dirty && !busy ? "common.cancel" : "common.close")}</button>
        <button type="button" className="btn btn-primary" disabled={!!busy || !valid || !dirty} onClick={save}>
          {t(busy === "save" ? "common.loading" : "common.save")}
        </button>
      </footer>
    </div>
  </div>, document.body);
}
