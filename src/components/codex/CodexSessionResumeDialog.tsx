import { useEffect, useId, useRef, useState, type FormEvent } from "react";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import type { InstanceProfile } from "../../types/instance";
import {
  getCodexSessionResumeSettings,
  saveCodexSessionResumeSettings,
  type CodexSessionResumeSettings,
  type CodexSessionResumeTarget,
} from "../../services/codexInstanceService";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { SingleSelectDropdown } from "../SingleSelectDropdown";

const uuidPattern = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;

function ResumeForm({ instanceId, cli, onSaving }: {
  instanceId: string;
  cli: boolean;
  onSaving: (saving: boolean) => void;
}) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<CodexSessionResumeSettings>();
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const [saving, setSaving] = useState(false);
  const field = useId();

  useEffect(() => {
    let active = true;
    void getCodexSessionResumeSettings(instanceId).then((data) => {
      if (active) setSettings(data);
    }, (reason) => {
      if (active) setError(String(reason));
    });
    return () => { active = false; };
  }, [instanceId]);

  function edit(next: CodexSessionResumeSettings) {
    setSettings(next);
    setSaved(false);
    setError(undefined);
  }

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!settings || saving) return;
    const threadId = settings.threadId?.trim() || null;
    const hasValidThread = threadId != null && uuidPattern.test(threadId);
    if (settings.enabled && settings.target === "specific" && !hasValidThread) {
      setError(t("codex.instances.sessionResume.invalidId"));
      return;
    }
    if (settings.enabled && !settings.prompt.trim()) {
      setError(t("codex.instances.sessionResume.promptRequired"));
      return;
    }
    setError(undefined);
    setSaved(false);
    setSaving(true);
    onSaving(true);
    try {
      await saveCodexSessionResumeSettings(instanceId, {
        ...settings, threadId: hasValidThread ? threadId : null,
      });
      setSaved(true);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
      onSaving(false);
    }
  }

  return (
    <form onSubmit={(event) => void save(event)}>
      {!settings && !error && <p role="status">{t("common.loading")}</p>}
      {settings && (
        <>
          <div className="form-group">
            <label>
              <input type="checkbox" checked={settings.enabled} disabled={saving}
                onChange={(event) => edit({ ...settings, enabled: event.target.checked })} />
              {" "}{t("codex.instances.sessionResume.enabled")}
            </label>
          </div>
          {settings.enabled && (
            <>
              {cli && <p>{t("codex.instances.sessionResume.cliHint")}</p>}
              <div className="form-group">
                <label>{t("codex.instances.sessionResume.target")}</label>
                <SingleSelectDropdown
                  value={settings.target}
                  options={([
                    "recentActivity", "lastCreated", "previousDesktopSession", "specific",
                  ] as CodexSessionResumeTarget[]).map((value) => ({
                    value, label: t(`codex.instances.sessionResume.${value}`),
                  }))}
                  onChange={(target) => edit({ ...settings, target: target as CodexSessionResumeTarget })}
                  disabled={saving}
                  ariaLabel={t("codex.instances.sessionResume.target")}
                />
              </div>
              {settings.target === "specific" && (
                <div className="form-group">
                  <label htmlFor={`${field}-thread`}>{t("codex.instances.sessionResume.threadId")}</label>
                  <input id={`${field}-thread`} className="form-input" disabled={saving}
                    value={settings.threadId ?? ""}
                    onChange={(event) => edit({ ...settings, threadId: event.target.value })} />
                </div>
              )}
              <div className="form-group">
                <label htmlFor={`${field}-prompt`}>{t("codex.instances.sessionResume.prompt")}</label>
                <textarea id={`${field}-prompt`} className="form-input" rows={3} disabled={saving}
                  value={settings.prompt}
                  onChange={(event) => edit({ ...settings, prompt: event.target.value })} />
              </div>
            </>
          )}
          <div className="form-group">
            <button type="submit" className="btn btn-primary" disabled={saving}>
              {t(saving ? "common.loading" : "common.save")}
            </button>
            {saved && <p role="status">{t("common.saved")}</p>}
          </div>
        </>
      )}
      <ModalErrorMessage message={error} position="bottom" />
    </form>
  );
}

export function CodexSessionResumeDialog({ instances, onClose }: {
  instances: InstanceProfile[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const title = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const [saving, setSaving] = useState(false);
  const [instanceId, setInstanceId] = useState(instances[0]?.id ?? "");
  const selected = instances.find((instance) => instance.id === instanceId) ?? instances[0];

  useEscCloseTopmost(true, () => { if (!saving) onClose(); }, dialog);
  useModalFocusTrap(dialog, true);

  return (
    <div className="modal-overlay">
      <div className="modal" role="dialog" aria-modal="true" aria-labelledby={title} tabIndex={-1} ref={dialog}>
        <div className="modal-header">
          <h2 id={title}>{t("codex.instances.sessionResume.title")}</h2>
          <button type="button" className="modal-close" onClick={onClose} disabled={saving}
            aria-label={t("common.close")}><X size={18} /></button>
        </div>
        <div className="modal-body">
          {selected && (
            <>
              <div className="form-group">
                <label>{t("instances.columns.instance")}</label>
                <SingleSelectDropdown
                  value={selected.id}
                  options={instances.map((instance) => ({ value: instance.id, label: instance.name }))}
                  onChange={setInstanceId}
                  disabled={saving}
                  ariaLabel={t("instances.columns.instance")}
                />
              </div>
              <ResumeForm key={selected.id} instanceId={selected.id} cli={selected.launchMode === "cli"} onSaving={setSaving} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}
