import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { ImagePlus, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { CodexImageAccountPickerModal } from "./codex/CodexImageAccountPickerModal";
import type { CodexAccount } from "../types/codex";
import "./CodexLocalAccessModal.css";

interface CodexImageForwardConfigParams {
  accounts: CodexAccount[];
  /** 已保存的生图转发账号 ID。 */
  accountIds?: string[];
  disabled: boolean;
  onSave: (accountIds: string[]) => Promise<unknown> | unknown;
  /** 生图模型等附加设置，随账号选择一起放进「选择生图账号」弹框。 */
  imageModelControl?: ReactNode;
}

/** 「启用 GPT 生图」的共享状态与左右插槽；卡片形态与启动预览行形态共用。 */
export interface CodexImageForwardConfigController {
  enabled: boolean;
  busy: boolean;
  pending: boolean;
  statusText: string;
  selectedAccounts: CodexAccount[];
  /** 错误/成功提示；行版放进左侧说明区，卡片版放在开关下方。 */
  feedback: ReactNode;
  /** 账号选择弹框（portal）。 */
  overlay: ReactNode;
  /** 启用勾选框；className 由调用方按所在布局传入。 */
  renderEnableCheckbox: (className: string) => ReactNode;
  /**
   * 「选择 GPT 账号」按钮；className 由调用方按所在布局传入。
   * 启动预览行与 DeepSeek 行保持一致（纯文字），配置卡片保留图标。
   */
  renderPickButton: (
    className: string,
    options?: { showIcon?: boolean },
  ) => ReactNode;
}

/**
 * API 服务的「启用 GPT 生图」配置；与实例启动预览里的同名能力一致：
 * 勾选并选择 GPT 账号后，生图与图片编辑请求交给所选账号执行。
 */
export function useCodexImageForwardConfig({
  accounts,
  accountIds,
  disabled,
  onSave,
  imageModelControl,
}: CodexImageForwardConfigParams): CodexImageForwardConfigController {
  const { t } = useTranslation();
  const [pickerOpen, setPickerOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const feedbackRef = useRef<HTMLDivElement>(null);
  const savingRef = useRef(false);
  const busy = disabled || pending;

  const selectedIds = useMemo(() => {
    const seen = new Set<string>();
    return (accountIds ?? [])
      .map((value) => value.trim())
      .filter((value) => {
        if (!value || seen.has(value)) return false;
        seen.add(value);
        return true;
      });
  }, [accountIds]);
  const enabled = selectedIds.length > 0;
  const selectedAccounts = useMemo(
    () =>
      selectedIds
        .map((accountId) => accounts.find((account) => account.id === accountId))
        .filter((account): account is CodexAccount => Boolean(account)),
    [accounts, selectedIds],
  );

  useEffect(() => {
    if (!error) return;
    feedbackRef.current?.scrollIntoView({ block: "nearest" });
  }, [error]);

  const persist = useCallback(
    async (nextAccountIds: string[]) => {
      if (savingRef.current) return;
      savingRef.current = true;
      setPending(true);
      setError("");
      setNotice("");
      try {
        await onSave(nextAccountIds);
        setNotice(t("codex.localAccess.saveSuccess", "API 服务集合已更新"));
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      } finally {
        savingRef.current = false;
        setPending(false);
      }
    },
    [onSave, t],
  );

  const statusText = enabled
    ? t("codex.deepSeek.start.imageGenSelected", {
        count: selectedIds.length,
        defaultValue: "已选 {{count}} 个账号",
      })
    : t("codex.deepSeek.start.imageGenEmpty", "尚未选择账号");

  const overlay = pickerOpen
    ? // 账号选择弹框挂在 body 上，避免嵌在弹框遮罩层里被裁剪或随内容滚动。
      createPortal(
        <CodexImageAccountPickerModal
          accounts={accounts}
          selectedIds={selectedIds}
          contextLabel={t("codex.localAccess.title", "API 服务")}
          imageModelControl={imageModelControl}
          saving={pending}
          onCancel={() => setPickerOpen(false)}
          onConfirm={(ids) => {
            setPickerOpen(false);
            void persist(ids);
          }}
        />,
        document.body,
      )
    : null;

  const feedback = (
    <div ref={feedbackRef}>
      {error && (
        <small role="alert" className="codex-local-access-image-model-error">
          {error}
        </small>
      )}
      {notice && (
        <small role="status" className="codex-local-access-config-hint">
          {notice}
        </small>
      )}
    </div>
  );

  const enableCheckbox = (className: string) => (
    <label className={className}>
      <input
        type="checkbox"
        checked={enabled}
        disabled={busy}
        onChange={(event) => {
          if (event.target.checked) {
            setPickerOpen(true);
            return;
          }
          void persist([]);
        }}
      />
      <span>{t("common.enable", "启用")}</span>
    </label>
  );

  const pickButton = (
    className: string,
    options?: { showIcon?: boolean },
  ) => {
    const showIcon = options?.showIcon ?? true;
    return (
      <button
        type="button"
        className={className}
        onClick={() => setPickerOpen(true)}
        disabled={busy || !enabled}
      >
        {showIcon &&
          (pending ? (
            <RefreshCw size={14} className="loading-spinner" />
          ) : (
            <ImagePlus size={14} />
          ))}
        {t("codex.deepSeek.start.imageGenPick", "选择 GPT 账号")}
      </button>
    );
  };

  return {
    enabled,
    busy,
    pending,
    statusText,
    selectedAccounts,
    feedback,
    overlay,
    renderEnableCheckbox: enableCheckbox,
    renderPickButton: pickButton,
  };
}

interface Props {
  accounts: CodexAccount[];
  /** 已保存的生图转发账号 ID。 */
  accountIds?: string[];
  disabled: boolean;
  maskAccountText: (value?: string | null) => string;
  onSave: (accountIds: string[]) => Promise<unknown> | unknown;
  /** 生图模型等附加设置，随账号选择一起放进「选择生图账号」弹框。 */
  imageModelControl?: ReactNode;
}

/** API 服务面板里的「启用 GPT 生图」配置卡片。 */
export function CodexImageForwardConfig({
  accounts,
  accountIds,
  disabled,
  maskAccountText,
  onSave,
  imageModelControl,
}: Props) {
  const { t } = useTranslation();
  const controller = useCodexImageForwardConfig({
    accounts,
    accountIds,
    disabled,
    onSave,
    imageModelControl,
  });

  return (
    <div className="codex-local-access-config-card codex-local-access-config-card-image-forward">
      <div className="codex-local-access-config-head">
        <span className="codex-local-access-config-label">
          {t("codex.localAccess.imageForwardLabel", "启用 GPT 生图")}
        </span>
        <div className="codex-local-access-config-actions">
          {controller.renderPickButton("btn btn-secondary btn-sm")}
        </div>
      </div>
      {controller.renderEnableCheckbox(
        "codex-local-access-free-toggle codex-local-access-image-forward-toggle",
      )}
      <div className="codex-local-access-image-forward-status">
        {controller.statusText}
      </div>
      {controller.selectedAccounts.length > 0 && (
        <div className="codex-local-access-image-forward-tags">
          {controller.selectedAccounts.map((account) => (
            <span
              key={account.id}
              className="codex-local-access-image-forward-tag"
            >
              {maskAccountText(
                account.email || account.account_name || account.id,
              )}
            </span>
          ))}
        </div>
      )}
      {controller.feedback}
      <small className="codex-local-access-config-hint">
        {t(
          "codex.localAccess.imageForwardHint",
          "勾选后选择 GPT 账号：生图与图片编辑请求交给所选账号执行并消耗其额度，对话请求仍按账号池调度。",
        )}
      </small>
      {controller.overlay}
    </div>
  );
}
