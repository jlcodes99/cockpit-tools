import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { CodexAccount, CodexAppSpeed } from "../../types/codex";
import {
  getCodexApiServiceAppSpeedConfig,
  saveCodexApiServiceAppSpeed,
  updateCodexAccountAppSpeed,
} from "../../services/codexService";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import { useCodexInstanceStore } from "../../stores/useCodexInstanceStore";
import { CodexSpeedSelect } from "./CodexSpeedSelect";
import { isDeepSeekAccount } from "../../utils/codexDeepSeekAccess";

type Props = {
  scope: "account" | "api" | "instance";
  account?: CodexAccount | null;
  instanceId?: string;
  compact?: boolean;
  disabled?: boolean;
  showHint?: boolean;
  onError: (message: string | null) => void;
  onSavingChange?: (saving: boolean) => void;
};

const API_SPEED_CHANGED = "cockpit:api-speed-changed";
let pendingApiRead: Promise<CodexAppSpeed> | null = null;
function readApiSpeed(): Promise<CodexAppSpeed> {
  if (pendingApiRead) return pendingApiRead;
  let timer: ReturnType<typeof setTimeout>;
  pendingApiRead = Promise.race([
    getCodexApiServiceAppSpeedConfig().then((config) => config.speed),
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error("timeout")), 5000);
    }),
  ]).finally(() => { clearTimeout(timer); pendingApiRead = null; });
  return pendingApiRead;
}

/** One persisted speed control for overview cards and the launch preview. */
export function CodexSpeedControl({ scope, account, instanceId, compact, disabled, showHint, onError, onSavingChange }: Props) {
  const { t } = useTranslation();
  const savedAccount = useCodexAccountStore((state) => state.accounts.find((item) => item.id === account?.id));
  const instance = useCodexInstanceStore((state) => state.instances.find((item) => item.id === instanceId));
  const [apiSpeed, setApiSpeed] = useState<CodexAppSpeed>("standard");
  const [loaded, setLoaded] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  const pending = useRef(false);
  const session = useRef(0);
  const reportError = useRef(onError);
  const reportSaving = useRef(onSavingChange);
  reportError.current = onError;
  reportSaving.current = onSavingChange;

  useEffect(() => {
    const generation = ++session.current;
    let changed = false;
    setLoaded(false);
    setLoadFailed(false);
    if (scope === "api") {
      void readApiSpeed().then((speed) => {
        if (generation !== session.current || changed) return;
        setApiSpeed(speed);
        setLoaded(true);
      }).catch((error) => {
        if (generation !== session.current || changed) return;
        setLoadFailed(true);
        reportError.current(t("codex.speed.loadFailed", { error: String(error) }));
      });
    }
    const sync = (event: Event) => {
      changed = true;
      setApiSpeed((event as CustomEvent<CodexAppSpeed>).detail);
      setLoaded(true);
      setLoadFailed(false);
    };
    if (scope === "api") window.addEventListener(API_SPEED_CHANGED, sync);
    return () => {
      session.current++;
      if (scope === "api") window.removeEventListener(API_SPEED_CHANGED, sync);
      reportSaving.current?.(false);
    };
  }, [scope, account?.id, instanceId, retry, t]);

  const speed = scope === "api" ? apiSpeed : scope === "account"
    ? (savedAccount ?? account)?.app_speed ?? "standard"
    : instance?.appSpeed ?? "standard";
  const unavailable = scope === "api" ? !loaded : scope === "account" ? !account : !instance;
  const change = async (next: CodexAppSpeed) => {
    if (pending.current || disabled || unavailable) return;
    const generation = session.current;
    pending.current = true;
    setSaving(true);
    reportSaving.current?.(true);
    reportError.current(null);
    try {
      if (scope === "api") {
        const saved = await saveCodexApiServiceAppSpeed(next);
        window.dispatchEvent(new CustomEvent(API_SPEED_CHANGED, { detail: saved.speed }));
      } else if (scope === "account" && account) {
        const saved = await updateCodexAccountAppSpeed(account.id, next);
        useCodexAccountStore.getState().applyAccountSnapshot(saved);
      } else if (instanceId) {
        await useCodexInstanceStore.getState().updateInstance({ instanceId, appSpeed: next, deferBindAccountApplication: true });
      }
      // Update bound-instance labels without blocking the control on a scan.
      if (scope !== "instance") void useCodexInstanceStore.getState().fetchInstances().catch(() => {});
    } catch (error) {
      if (generation === session.current) {
        const syncFailed = scope === "api" && String(error).includes("codex_api_service_speed_sync_failed");
        reportError.current(syncFailed
          ? t("codex.speed.apiSyncFailed")
          : t("codex.speed.saveFailed", { error: String(error) }));
      }
    } finally {
      pending.current = false;
      if (generation === session.current) {
        setSaving(false);
        reportSaving.current?.(false);
      }
    }
  };
  const hint = t(scope === "api" ? "codex.speed.apiHint" : "codex.speed.launchHint");
  // The DeepSeek integration deliberately removes unsupported service tiers.
  if (scope !== "api" && account && isDeepSeekAccount(account)) return null;
  return (
    <span className="codex-speed-control" title={hint}>
      <CodexSpeedSelect value={speed} onChange={change} busy={saving} compact={compact}
        disabled={disabled || unavailable} />
      {loadFailed && <button type="button" className="btn btn-outline btn-sm" disabled={saving}
        onClick={() => { reportError.current(null); setRetry((value) => value + 1); }}>{t("common.retry")}</button>}
      {showHint && <small className="codex-speed-control-hint">{hint}</small>}
    </span>
  );
}
