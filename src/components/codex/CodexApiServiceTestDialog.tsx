import { CircleAlert, RefreshCw, Send, ShieldCheck, X } from "lucide-react";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import type { CodexApiServiceViewProps } from "../../pages/CodexApiServiceView";

export interface CodexApiServiceTestDialogProps {
  open: boolean;
  onClose: () => void;
  t: CodexApiServiceViewProps["t"];
  selectedModelId: string;
  testModelIds: string[];
  onSelectModel: (modelId: string) => void;
  running: boolean;
  onClearChat: () => void;
  messages: CodexApiServiceViewProps["testChatMessages"];
  scrollRef: CodexApiServiceViewProps["testChatScrollRef"];
  formatLatencyMs: CodexApiServiceViewProps["formatLatencyMs"];
  error: string;
  chatInput: string;
  onChatInputChange: (value: string) => void;
  onSend: () => Promise<void> | void;
}

export function CodexApiServiceTestDialog({
  open,
  onClose,
  t,
  selectedModelId,
  testModelIds,
  onSelectModel,
  running,
  onClearChat,
  messages,
  scrollRef,
  formatLatencyMs,
  error,
  chatInput,
  onChatInputChange,
  onSend,
}: CodexApiServiceTestDialogProps) {
  if (!open) return null;

  return (
    <div className="modal-overlay codex-local-access-test-dialog-overlay">
      <div
        className="modal codex-local-access-test-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-api-service-test-dialog-title"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header codex-local-access-test-dialog-header">
          <div>
            <h3 id="codex-api-service-test-dialog-title">
              <ShieldCheck size={18} />
              <span>{t("codex.localAccess.testDialogTitle", "测试 API 服务")}</span>
            </h3>
            <p>{t("codex.localAccess.testDialogDesc")}</p>
          </div>
          <button
            className="modal-close codex-local-access-test-dialog-close"
            onClick={onClose}
            aria-label={t("common.close")}
          >
            <X size={18} />
          </button>
        </div>

        <div className="modal-body codex-local-access-test-dialog-body">
          <div className="codex-local-access-test-chat-toolbar">
            <div className="codex-local-access-test-chat-model">
              <span>{t("codex.localAccess.testChatModel", "模型")}</span>
              <SingleSelectDropdown
                value={selectedModelId}
                options={testModelIds.map((modelId) => ({
                  value: modelId,
                  label: modelId,
                }))}
                onChange={onSelectModel}
                disabled={testModelIds.length === 0 || running}
                ariaLabel={t("codex.localAccess.testChatModel", "模型")}
                placeholder={t("codex.localAccess.modelIdPlaceholder", "选择模型 ID")}
                menuPlacement="down"
                menuMaxHeight={240}
              />
            </div>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={onClearChat}
              disabled={running || messages.length === 0}
            >
              {t("codex.localAccess.testChatClear", "清空对话")}
            </button>
          </div>

          <div className="codex-local-access-test-chat" ref={scrollRef}>
            {messages.length === 0 ? (
              <div className="codex-local-access-test-chat-empty">
                {t("codex.localAccess.testChatEmpty")}
              </div>
            ) : (
              messages.map((message) => (
                <div
                  key={message.id}
                  className={`codex-local-access-test-chat-message is-${message.role}${
                    message.failureTitle ? " is-error" : ""
                  }`}
                >
                  <div className="codex-local-access-test-chat-bubble">
                    {message.failureTitle && (
                      <strong className="codex-local-access-test-chat-error-title">
                        {message.failureTitle}
                      </strong>
                    )}
                    {message.imageDataUrl ? (
                      <img src={message.imageDataUrl} alt={t("codex.localAccess.testOutputTitle")} />
                    ) : (
                      <p>{message.content}</p>
                    )}
                    {message.failureDetail && (
                      <span className="codex-local-access-test-chat-meta">
                        {message.failureDetail}
                      </span>
                    )}
                    {typeof message.latencyMs === "number" && (
                      <span className="codex-local-access-test-chat-meta">
                        {t("codex.localAccess.testChatLatency", {
                          latency: formatLatencyMs(message.latencyMs),
                          defaultValue: "耗时 {{latency}}",
                        })}
                      </span>
                    )}
                  </div>
                </div>
              ))
            )}
            {running && (
              <div className="codex-local-access-test-chat-message is-assistant">
                <div className="codex-local-access-test-chat-bubble">
                  <span className="codex-local-access-test-chat-loading">
                    <RefreshCw size={14} className="loading-spinner" />
                    {t("codex.localAccess.testChatSending", "正在请求 API 服务")}
                  </span>
                </div>
              </div>
            )}
          </div>

          {error && (
            <div className="codex-local-access-inline-error" aria-live="assertive">
              <CircleAlert size={14} />
              <span>{error}</span>
            </div>
          )}
        </div>

        <div className="modal-footer codex-local-access-test-dialog-footer">
          <textarea
            className="codex-local-access-test-chat-input"
            value={chatInput}
            onChange={(event) => onChatInputChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                void onSend();
              }
            }}
            disabled={running}
            rows={2}
            placeholder={t(
              "codex.localAccess.testChatInputPlaceholder",
              "输入测试消息，Enter 发送",
            )}
          />
          <button
            className="btn btn-primary codex-local-access-test-chat-send"
            onClick={() => void onSend()}
            disabled={running || !chatInput.trim() || !selectedModelId}
          >
            <Send size={15} />
            {t("codex.localAccess.testChatSend", "发送")}
          </button>
          <button className="btn btn-secondary" onClick={onClose}>
            {t("common.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
