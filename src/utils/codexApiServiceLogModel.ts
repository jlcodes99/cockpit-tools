export interface CodexApiServiceLogModelInput {
  modelId?: string | null;
  requestedModel?: string | null;
  upstreamModel?: string | null;
}

export interface CodexApiServiceLogModelPair {
  /** 首行展示的客户端请求模型。 */
  requestedModel: string;
  /** 第二行展示的实际上游模型；与请求模型一致时同样展示，未记录时为空字符串。 */
  upstreamModel: string;
}

const trimmed = (value: string | null | undefined): string => (value ?? "").trim();

/**
 * 始终返回实际发送到上游的模型：与请求模型一致时也保留第二行，
 * 便于在日志里直接确认转发链路是否生效。
 * 只有当日志未记录上游模型（历史数据或空行）时才返回空字符串，
 * 由调用方回退为单行样式。
 */
export function resolveCodexApiServiceLogModelPair(
  event: CodexApiServiceLogModelInput,
): CodexApiServiceLogModelPair {
  const requestedModel = trimmed(event.requestedModel) || trimmed(event.modelId) || "--";
  const upstreamModel = trimmed(event.upstreamModel);
  return { requestedModel, upstreamModel };
}
