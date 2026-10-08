import { useMemo, useState } from "react";
import { ArrowRight, ImagePlus, KeyRound, Play, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useEscClose } from "../../hooks/useEscClose";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import {
  isCodexApiKeyAccount,
  isCodexChatCompletionsApiKeyAccount,
  type CodexAccount,
} from "../../types/codex";
import {
  DEEPSEEK_ACCESS_MODE_CDP,
  DEEPSEEK_ACCESS_MODE_DIRECT,
  DEEPSEEK_ACCESS_MODE_GATEWAY,
  isDeepSeekAccount,
  isDeepSeekResponsesAccount,
  resolveDeepSeekAccessMode,
  resolveDeepSeekModelOptions,
  resolveDeepSeekStartupModel,
  shouldUseDeepSeekProviderGateway,
  type DeepSeekAccessMode,
  type DeepSeekStartTarget,
} from "../../utils/codexDeepSeekAccess";
import { CodexImageAccountPickerModal } from "./CodexImageAccountPickerModal";
import "./CodexLaunchPreviewModal.css";
import "./CodexApiKeyLaunchSection.css";

export interface CodexApiKeyLaunchOptions {
  accessMode: DeepSeekAccessMode;
  startupModel: string;
  imageGenEnabled: boolean;
  imageGenAccountIds: string[];
}

/** 启动预览与模型供应商弹框共用：后者传入的是表单草稿，字段可能不全。 */
export type CodexApiKeyLaunchAccount = DeepSeekStartTarget & {
  id?: string;
  email?: string;
  account_name?: string | null;
  plan_type?: string | null;
  api_provider_name?: string | null;
  api_model_context_windows?: Record<string, number> | null;
  api_image_generation_account_ids?: string[] | null;
};

export function createCodexApiKeyLaunchOptions(
  account: CodexApiKeyLaunchAccount | null | undefined,
): CodexApiKeyLaunchOptions {
  const imageAccountIds = Array.isArray(account?.api_image_generation_account_ids)
    ? (account?.api_image_generation_account_ids ?? []).filter(
        (value): value is string => typeof value === "string" && value.trim().length > 0,
      )
    : [];
  return {
    accessMode: resolveDeepSeekAccessMode(account ?? undefined),
    startupModel: resolveDeepSeekStartupModel(account ?? undefined),
    imageGenEnabled: imageAccountIds.length > 0,
    imageGenAccountIds: imageAccountIds,
  };
}

/** 该账号是否会经过实例本地网关（决定生图转发是否可用）。 */
export function accountUsesInstanceLocalGateway(
  account: CodexApiKeyLaunchAccount | null | undefined,
): boolean {
  if (!account || !isCodexApiKeyAccount(account as CodexAccount)) return false;
  const full = account as CodexAccount;
  if (isCodexChatCompletionsApiKeyAccount(full)) return true;
  if (isDeepSeekAccount(full)) return shouldUseDeepSeekProviderGateway(full);
  return (
    (full.api_provider_mode ?? "openai_builtin") === "custom" &&
    full.api_sync_model_catalog_to_codex === true
  );
}

function formatContextWindow(tokens: number): string {
  if (!Number.isFinite(tokens) || tokens <= 0) return "";
  if (tokens >= 1_000_000) {
    const millions = tokens / 1_000_000;
    return `${Number.isInteger(millions) ? millions : millions.toFixed(1)}M`;
  }
  if (tokens >= 1_000) {
    const thousands = tokens / 1_000;
    return `${Number.isInteger(thousands) ? thousands : thousands.toFixed(0)}K`;
  }
  return String(tokens);
}

export interface CodexApiKeyLaunchSectionProps {
  account: CodexApiKeyLaunchAccount;
  options: CodexApiKeyLaunchOptions;
  onChange: (options: CodexApiKeyLaunchOptions) => void;
  disabled?: boolean;
}

export function CodexApiKeyLaunchSection({
  account,
  options,
  onChange,
  disabled = false,
}: CodexApiKeyLaunchSectionProps) {
  const { t } = useTranslation();
  const [pickerOpen, setPickerOpen] = useState(false);
  const accounts = useCodexAccountStore((state) => state.accounts);

  const canChooseAccessMode = isDeepSeekResponsesAccount(account);
  const isDeepSeek = isDeepSeekAccount(account);
  const models = useMemo(() => {
    if (isDeepSeek) return resolveDeepSeekModelOptions(account);
    const catalog = (account.api_model_catalog ?? [])
      .map((model) => model.trim())
      .filter((model, index, list) => model.length > 0 && list.indexOf(model) === index);
    return catalog.map((id) => ({ id, label: id }));
  }, [account, isDeepSeek]);
  const contextFacts = useMemo(
    () =>
      models.map((model) => {
        const configured = account.api_model_context_windows?.[model.id];
        return {
          id: model.id,
          label: model.label,
          context: configured ? formatContextWindow(configured) : "",
        };
      }),
    [account.api_model_context_windows, models],
  );
  const hasContextData = contextFacts.some((fact) => fact.context.length > 0);
  const usesLocalGateway = accountUsesInstanceLocalGateway(account);
  const wireApiLabel =
    (account.api_wire_api ?? "responses").trim().toLowerCase() === "chat_completions"
      ? "Chat Completions"
      : "Responses";

  const modes: {
    id: DeepSeekAccessMode;
    title: string;
    description: string;
  }[] = [
    {
      id: DEEPSEEK_ACCESS_MODE_GATEWAY,
      title: t("codex.deepSeek.start.gatewayMode", "网关列出"),
      description: t(
        "codex.deepSeek.start.gatewayDesc",
        "需要本地网关。支持绑定 OAuth 的 GPT 账号并使用 OAuth 的全部能力（浏览器操作等），支持在 Codex 内切换模型，可把生图交给 GPT 账号。",
      ),
    },
    {
      id: DEEPSEEK_ACCESS_MODE_DIRECT,
      title: t("codex.deepSeek.start.directMode", "直连官方"),
      description: t(
        "codex.deepSeek.start.directDesc",
        "不走网关，速度更快。不能在 Codex 内切换模型，也没有 OAuth 能力与生图转发。",
      ),
    },
    {
      id: DEEPSEEK_ACCESS_MODE_CDP,
      title: t("codex.deepSeek.start.cdpMode", "CDP 注入"),
      description: t(
        "codex.deepSeek.start.cdpDesc",
        "通过注入接管 Codex 官方模型列表，速度与直连一致。需要注入官方客户端，不支持绑定 OAuth。",
      ),
    },
  ];

  const selectedImageAccounts = options.imageGenAccountIds
    .map((id) => accounts.find((item) => item.id === id))
    .filter((item): item is CodexAccount => Boolean(item));

  return (
    <div className="codex-api-key-launch">
      {hasContextData && (
        <div className="form-group">
          <label>{t("codex.deepSeek.start.context", "支持上下文")}</label>
          <div className="codex-api-key-launch-context-list">
            {contextFacts
              .filter((fact) => fact.context.length > 0)
              .map((fact) => (
                <span key={fact.id} className="codex-api-key-launch-context-item">
                  <span className="codex-api-key-launch-context-model">{fact.label}</span>
                  <span className="codex-api-key-launch-context-value">{fact.context}</span>
                </span>
              ))}
          </div>
        </div>
      )}

      {canChooseAccessMode ? (
        <div className="form-group">
          <label>{t("codex.deepSeek.start.accessMode", "接入方式")}</label>
          <div className="codex-api-key-launch-mode-list">
            {modes.map((mode) => (
              <button
                key={mode.id}
                type="button"
                className={`codex-api-key-launch-mode ${
                  options.accessMode === mode.id ? "active" : ""
                }`}
                onClick={() => onChange({ ...options, accessMode: mode.id })}
                disabled={disabled}
              >
                <span className="codex-api-key-launch-mode-title">{mode.title}</span>
                <span className="codex-api-key-launch-mode-desc">{mode.description}</span>
              </button>
            ))}
          </div>
        </div>
      ) : (
        <div className="form-group">
          <label>{t("codex.deepSeek.start.accessMode", "接入方式")}</label>
          <div className="codex-api-key-launch-context-list">
            <span className="codex-api-key-launch-context-item">
              <span className="codex-api-key-launch-context-model">
                {t("codex.deepSeek.start.protocol", "协议")}
              </span>
              <span className="codex-api-key-launch-context-value">{wireApiLabel}</span>
            </span>
            <span className="codex-api-key-launch-context-item">
              <span className="codex-api-key-launch-context-model">
                {t("codex.deepSeek.start.accessRoute", "接入")}
              </span>
              <span className="codex-api-key-launch-context-value">
                {usesLocalGateway
                  ? t("codex.deepSeek.start.gatewayRequired", "走实例本地网关")
                  : t("codex.deepSeek.start.directUpstream", "直连上游，不经网关")}
              </span>
            </span>
          </div>
          <p className="form-hint">
            {usesLocalGateway
              ? t(
                  "codex.deepSeek.start.gatewayHint",
                  "该协议需要本地网关转换，可在 Codex 内切换模型；网关还能承接生图转发。",
                )
              : t(
                  "codex.deepSeek.start.directUpstreamHint",
                  "该账号直接访问上游，不经过本地网关，因此不支持生图转发。",
                )}
          </p>
        </div>
      )}

      {isDeepSeek && (
        <div className="form-group">
          <label>{t("codex.deepSeek.start.model", "启动模型")}</label>
          <div className="api-provider-chip-list">
            {models.map((model) => (
              <button
                key={model.id}
                type="button"
                className={`api-provider-chip ${
                  options.startupModel === model.id ? "active" : ""
                }`}
                onClick={() => onChange({ ...options, startupModel: model.id })}
                disabled={disabled}
              >
                <span>{model.label}</span>
              </button>
            ))}
          </div>
        </div>
      )}

      {(canChooseAccessMode ? options.accessMode === DEEPSEEK_ACCESS_MODE_GATEWAY : usesLocalGateway) && (
        <div className="form-group codex-api-key-launch-imagegen">
          <label className="codex-api-key-launch-imagegen-head">
            <input
              type="checkbox"
              checked={options.imageGenEnabled}
              disabled={disabled}
              onChange={(event) => {
                const enabled = event.target.checked;
                onChange({
                  ...options,
                  imageGenEnabled: enabled,
                  imageGenAccountIds: enabled ? options.imageGenAccountIds : [],
                });
              }}
            />
            <span>
              <ImagePlus size={15} />
              {t("codex.deepSeek.start.imageGen", "支持 GPT 生图")}
            </span>
          </label>
          <p className="form-hint">
            {t(
              "codex.deepSeek.start.imageGenHint",
              "对话仍由 DeepSeek 处理；生图走 gpt-image 原链路，由所选 GPT 账号执行并消耗其额度。",
            )}
          </p>
          <div className="codex-api-key-launch-imagegen-row">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              disabled={disabled || !options.imageGenEnabled}
              onClick={() => setPickerOpen(true)}
            >
              {t("codex.deepSeek.start.imageGenPick", "选择 GPT 账号")}
            </button>
            <span className="codex-api-key-launch-imagegen-status">
              {!options.imageGenEnabled
                ? t("codex.deepSeek.start.imageGenLocked", "勾选后可选择账号")
                : selectedImageAccounts.length > 0
                  ? t("codex.deepSeek.start.imageGenSelected", {
                      count: selectedImageAccounts.length,
                      defaultValue: "已选 {{count}} 个账号",
                    })
                  : t("codex.deepSeek.start.imageGenEmpty", "尚未选择账号")}
            </span>
          </div>
          {options.imageGenEnabled && selectedImageAccounts.length > 0 && (
            <div className="codex-api-key-launch-imagegen-tags">
              {selectedImageAccounts.map((item) => (
                <span key={item.id} className="codex-api-key-launch-imagegen-tag">
                  {item.email || item.account_name || item.id}
                </span>
              ))}
            </div>
          )}
        </div>
      )}

      {pickerOpen && (
        <CodexImageAccountPickerModal
          accounts={accounts}
          selectedIds={options.imageGenAccountIds}
          onCancel={() => setPickerOpen(false)}
          onConfirm={(ids) => {
            setPickerOpen(false);
            onChange({ ...options, imageGenEnabled: ids.length > 0, imageGenAccountIds: ids });
          }}
        />
      )}
    </div>
  );
}

export interface CodexApiKeyLaunchDialogProps {
  account: CodexApiKeyLaunchAccount;
  options: CodexApiKeyLaunchOptions;
  onChange: (options: CodexApiKeyLaunchOptions) => void;
  title?: string;
  description?: string;
  /** 预览信息区的供应商展示名；模型供应商启用流程传入。 */
  subjectLabel?: string;
  /** 预览信息区的类型徽标；默认取账号套餐值，API Key 供应商回落到 API_KEY。 */
  subjectBadge?: string;
  /** 目标实例名；传入后按启动预览样式展示。 */
  instanceName?: string;
  /** 主操作按钮文案；默认「确认启动」。 */
  confirmLabel?: string;
  submitting?: boolean;
  errorMessage?: string | null;
  onCancel: () => void;
  onConfirm: (options: CodexApiKeyLaunchOptions) => void;
}

/** 弹框形态：模型供应商启用流程复用启动预览样式，并按真实数据渲染供应商信息。 */
export function CodexApiKeyLaunchDialog({
  account,
  options,
  onChange,
  title,
  description,
  subjectLabel,
  subjectBadge,
  instanceName,
  confirmLabel,
  submitting = false,
  errorMessage = null,
  onCancel,
  onConfirm,
}: CodexApiKeyLaunchDialogProps) {
  const { t } = useTranslation();
  useEscClose(!submitting, onCancel);
  const providerLabel =
    subjectLabel?.trim() ||
    account.api_provider_name?.trim() ||
    account.account_name?.trim() ||
    account.api_base_url?.trim() ||
    t("codex.api.provider.custom", "自定义");
  const badgeLabel =
    subjectBadge?.trim() || account.plan_type?.trim() || "API_KEY";
  const modelCount = isDeepSeekAccount(account)
    ? resolveDeepSeekModelOptions(account).length
    : (account.api_model_catalog ?? []).filter((model) => model.trim().length > 0)
        .length;
  const baseUrl = account.api_base_url?.trim() || "-";
  const targetInstance = instanceName?.trim() || "";

  return (
    <div
      className="modal-overlay codex-launch-preview-overlay"
      role="presentation"
    >
      <div
        className="modal codex-launch-preview-modal codex-api-key-launch-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-api-key-launch-dialog-title"
      >
        <div className="modal-header">
          <div className="codex-launch-preview-title-icon">
            <Play size={18} />
          </div>
          <div className="codex-launch-preview-heading">
            <h2 id="codex-api-key-launch-dialog-title">
              {title || t("codex.launchPreview.title", "Codex 启动预览")}
            </h2>
            {description && <p>{description}</p>}
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={onCancel}
            disabled={submitting}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>
        <div className="modal-body">
          <ModalErrorMessage message={errorMessage} />
          <section className="codex-launch-preview-summary-card">
            <div className="codex-launch-preview-summary-head">
              <div className="codex-launch-preview-subject">
                <div className="codex-launch-preview-subject-icon">
                  <KeyRound size={19} />
                </div>
                <div className="codex-launch-preview-subject-copy">
                  <div className="codex-launch-preview-subject-title-row">
                    <strong title={providerLabel}>{providerLabel}</strong>
                    <span className="codex-launch-preview-plan-badge">
                      {badgeLabel}
                    </span>
                  </div>
                  <span title={providerLabel}>{providerLabel}</span>
                </div>
              </div>
              {targetInstance && (
                <div className="codex-launch-preview-summary-controls">
                  <div className="codex-launch-preview-target">
                    <span>
                      {t(
                        "codex.sessionManager.repairModal.targetInstance",
                        "目标实例",
                      )}
                    </span>
                    <strong title={targetInstance}>{targetInstance}</strong>
                    <ArrowRight size={16} />
                  </div>
                </div>
              )}
            </div>
            <div className="codex-launch-preview-facts">
              <div>
                <span>{t("codex.api.provider.label", "供应商")}</span>
                <strong title={providerLabel}>{providerLabel}</strong>
              </div>
              <div>
                <span>{t("codex.api.modelCatalog.label", "模型列表")}</span>
                <strong>
                  {t("codex.api.modelCatalog.count", {
                    count: modelCount,
                    defaultValue: "{{count}} 个模型",
                  })}
                </strong>
              </div>
              <div>
                <span>{t("codex.api.baseUrl", "基础地址")}</span>
                <strong className="is-monospace" title={baseUrl}>
                  {baseUrl}
                </strong>
              </div>
            </div>
          </section>
          <div className="codex-launch-preview-tool-list">
            <section className="codex-launch-preview-tool-section">
              <CodexApiKeyLaunchSection
                account={account}
                options={options}
                onChange={onChange}
                disabled={submitting}
              />
            </section>
          </div>
        </div>
        <div className="modal-footer">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={onCancel}
            disabled={submitting}
          >
            {t("common.cancel", "取消")}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => onConfirm(options)}
            disabled={submitting}
          >
            {!submitting && <Play size={15} />}
            {submitting
              ? t("common.saving", "保存中...")
              : confirmLabel || t("codex.deepSeek.start.confirm", "确认启动")}
          </button>
        </div>
      </div>
    </div>
  );
}
