import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { useEscClose } from "../../hooks/useEscClose";
import type { CodexAccount } from "../../types/codex";
import {
  DEEPSEEK_ACCESS_MODE_CDP,
  DEEPSEEK_ACCESS_MODE_DIRECT,
  DEEPSEEK_ACCESS_MODE_GATEWAY,
  DEEPSEEK_DIRECT_MODELS,
  isDeepSeekAccount,
  isDeepSeekResponsesAccount,
  resolveDeepSeekAccessMode,
  resolveDeepSeekStartupModel,
  type DeepSeekAccessMode,
  type DeepSeekStartChoice,
  type DeepSeekStartTarget,
} from "../../utils/codexDeepSeekAccess";

interface DeepSeekDirectModelModalProps {
  account: DeepSeekStartTarget;
  instanceName?: string;
  submitting?: boolean;
  errorMessage?: string | null;
  onCancel: () => void;
  onConfirm: (choice: DeepSeekStartChoice) => void;
}

export function useDeepSeekDirectModelPrompt() {
  const [prompt, setPrompt] = useState<{
    account: DeepSeekStartTarget;
    instanceName?: string;
  } | null>(null);
  const [submitting, setSubmitting] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const resolverRef = useRef<((choice: DeepSeekStartChoice | null) => void) | null>(
    null,
  );

  const requestStart = useCallback(
    (account: DeepSeekStartTarget, instanceName?: string) => {
      resolverRef.current?.(null);
      setErrorMessage(null);
      setSubmitting(false);
      setPrompt({ account, instanceName });
      return new Promise<DeepSeekStartChoice | null>((resolve) => {
        resolverRef.current = resolve;
      });
    },
    [],
  );

  const confirmStart = useCallback(
    async (
      account: CodexAccount,
      persist: (
        accountId: string,
        accessMode?: string | null,
        startupModel?: string | null,
      ) => Promise<CodexAccount>,
      instanceName?: string,
    ) => {
      if (!isDeepSeekAccount(account)) {
        return account;
      }
      const choice = await requestStart(account, instanceName);
      if (!choice) {
        return null;
      }
      return persist(account.id, choice.accessMode, choice.modelId);
    },
    [requestStart],
  );

  const finish = useCallback((choice: DeepSeekStartChoice | null) => {
    const resolve = resolverRef.current;
    resolverRef.current = null;
    setPrompt(null);
    setSubmitting(false);
    setErrorMessage(null);
    resolve?.(choice);
  }, []);

  const modal = prompt ? (
    <DeepSeekDirectModelModal
      account={prompt.account}
      instanceName={prompt.instanceName}
      submitting={submitting}
      errorMessage={errorMessage}
      onCancel={() => finish(null)}
      onConfirm={(choice) => finish(choice)}
    />
  ) : null;

  return {
    requestStart,
    confirmStart,
    requestModel: requestStart,
    modal,
    setSubmitting,
    setErrorMessage,
    clear: () => finish(null),
  };
}

export function DeepSeekDirectModelModal({
  account,
  instanceName,
  submitting = false,
  errorMessage = null,
  onCancel,
  onConfirm,
}: DeepSeekDirectModelModalProps) {
  const { t } = useTranslation();
  const canChooseAccessMode = isDeepSeekResponsesAccount(account);
  const [accessMode, setAccessMode] = useState<DeepSeekAccessMode>(() =>
    resolveDeepSeekAccessMode(account),
  );
  const [selectedModel, setSelectedModel] = useState(() =>
    resolveDeepSeekStartupModel(account),
  );

  useEffect(() => {
    setAccessMode(resolveDeepSeekAccessMode(account));
    setSelectedModel(resolveDeepSeekStartupModel(account));
  }, [account]);

  useEscClose(!submitting, onCancel);

  const resolvedAccessMode = canChooseAccessMode
    ? accessMode
    : DEEPSEEK_ACCESS_MODE_GATEWAY;

  return (
    <div className="modal-overlay" role="presentation">
      <div
        className="modal deepseek-start-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="deepseek-start-title"
      >
        <div className="modal-header">
          <h2 id="deepseek-start-title">
            {t("codex.deepSeek.start.title", "启动 DeepSeek")}
          </h2>
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
          <p className="form-hint">
            {t("codex.deepSeek.start.hint", {
              defaultValue:
                "启动前选择接入方式和模型。网关和 CDP 都可在 Codex 官方模型列表里切换 Flash / Pro；直连不走网关，每次启动在此选择。",
              instance: instanceName ? ` ${instanceName}` : "",
            })}
          </p>
          {canChooseAccessMode ? (
            <div className="form-group">
              <label>{t("codex.deepSeek.start.accessMode", "接入方式")}</label>
              <div className="deepseek-start-option-list">
                <button
                  type="button"
                  className={`deepseek-start-option ${
                    resolvedAccessMode === DEEPSEEK_ACCESS_MODE_GATEWAY
                      ? "active"
                      : ""
                  }`}
                  onClick={() => setAccessMode(DEEPSEEK_ACCESS_MODE_GATEWAY)}
                  disabled={submitting}
                >
                  <span className="deepseek-start-option-title">
                    {t("codex.deepSeek.start.gatewayMode", "网关列出")}
                  </span>
                  <span className="deepseek-start-option-desc">
                    {t(
                      "codex.deepSeek.start.gatewayDesc",
                      "走实例本地网关，只改写模型名。Codex 里可以看到并切换 Flash / Pro。",
                    )}
                  </span>
                </button>
                <button
                  type="button"
                  className={`deepseek-start-option ${
                    resolvedAccessMode === DEEPSEEK_ACCESS_MODE_DIRECT
                      ? "active"
                      : ""
                  }`}
                  onClick={() => setAccessMode(DEEPSEEK_ACCESS_MODE_DIRECT)}
                  disabled={submitting}
                >
                  <span className="deepseek-start-option-title">
                    {t("codex.deepSeek.start.directMode", "直连官方")}
                  </span>
                  <span className="deepseek-start-option-desc">
                    {t(
                      "codex.deepSeek.start.directDesc",
                      "不走网关，直连官方 API。Codex 内不能切模型，每次启动在此选择。",
                    )}
                  </span>
                </button>
                <button
                  type="button"
                  className={`deepseek-start-option ${
                    resolvedAccessMode === DEEPSEEK_ACCESS_MODE_CDP
                      ? "active"
                      : ""
                  }`}
                  onClick={() => setAccessMode(DEEPSEEK_ACCESS_MODE_CDP)}
                  disabled={submitting}
                >
                  <span className="deepseek-start-option-title">
                    {t("codex.deepSeek.start.cdpMode", "CDP 注入")}
                  </span>
                  <span className="deepseek-start-option-desc">
                    {t(
                      "codex.deepSeek.start.cdpDesc",
                      "直连官方 API，不走网关。通过 CDP 接管 Codex 官方模型列表，交互与网关相同，可在列表里切换 Flash / Pro。",
                    )}
                  </span>
                </button>
              </div>
            </div>
          ) : (
            <p className="form-hint">
              {t(
                "codex.deepSeek.start.chatOnlyHint",
                "Chat Completions 只能走本地网关，并在此选择启动模型。",
              )}
            </p>
          )}
          <div className="form-group">
            <label>{t("codex.deepSeek.start.model", "启动模型")}</label>
            <div className="api-provider-chip-list">
              {DEEPSEEK_DIRECT_MODELS.map((model) => (
                <button
                  key={model.id}
                  type="button"
                  className={`api-provider-chip ${selectedModel === model.id ? "active" : ""}`}
                  onClick={() => setSelectedModel(model.id)}
                  disabled={submitting}
                >
                  <span>{model.label}</span>
                </button>
              ))}
            </div>
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
            onClick={() =>
              onConfirm({
                accessMode: resolvedAccessMode,
                modelId: selectedModel,
              })
            }
            disabled={submitting}
          >
            {submitting
              ? t("common.saving", "保存中...")
              : t("codex.deepSeek.start.confirm", "确认启动")}
          </button>
        </div>
      </div>
    </div>
  );
}
