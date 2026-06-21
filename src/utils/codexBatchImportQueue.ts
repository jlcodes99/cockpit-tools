export type CodexBatchImportQueueTaskStatus =
  | "queued"
  | "running"
  | "ready"
  | "cancelled"
  | "error"
  | "importing"
  | "imported";

export interface CodexBatchImportQueueTaskLike {
  id: string;
  status: CodexBatchImportQueueTaskStatus;
  progress?: { current: number; total: number } | null;
  preview?: { total: number; items: unknown[] } | null;
}

export interface CodexBatchImportRecoverableTaskLike
  extends CodexBatchImportQueueTaskLike {
  sessionId?: string | null;
  selectedIds: string[];
}

export interface CodexBatchImportSelectableItemLike {
  itemId: string;
  defaultSelected: boolean;
  selectable: boolean;
  status: string;
}

export interface CodexBatchImportApiServicePreviewItemLike {
  itemId: string;
  accountId?: string | null;
}

export interface CodexBatchImportPreviewLike {
  sessionId: string;
  status: string;
  checkQuota: boolean;
  total: number;
  items: CodexBatchImportSelectableItemLike[];
}

export interface CodexBatchImportApiServiceImportedAccountLike {
  id?: string | null;
}

export type CodexBatchImportProgressTone = "active" | "success" | "error";

export function findNextCodexBatchImportTaskId(
  tasks: CodexBatchImportQueueTaskLike[],
): string | null {
  if (tasks.some((task) => task.status === "running" || task.status === "importing")) {
    return null;
  }
  return tasks.find((task) => task.status === "queued")?.id ?? null;
}

export function mergeCodexBatchImportDefaultSelection(
  selectedIds: string[],
  items: CodexBatchImportSelectableItemLike[],
): string[] {
  const next = new Set(selectedIds);
  for (const item of items) {
    if (
      item.defaultSelected &&
      item.selectable &&
      (item.status === "ready" || item.status === "existing")
    ) {
      next.add(item.itemId);
    }
  }
  return Array.from(next);
}

export function getCodexBatchImportProgressPercent(
  task: CodexBatchImportQueueTaskLike,
): number {
  const total = task.progress?.total ?? task.preview?.total ?? 0;
  const current =
    task.progress?.current ??
    (task.status === "ready" ||
    task.status === "cancelled" ||
    task.status === "imported"
      ? task.preview?.items.length ?? 0
      : 0);

  if (total <= 0) return 0;
  return Math.min(100, Math.max(0, Math.round((current / total) * 100)));
}

export function getCodexBatchImportProgressTone(
  task: CodexBatchImportQueueTaskLike,
): CodexBatchImportProgressTone {
  if (task.status === "error") return "error";
  if (
    (task.status === "ready" || task.status === "imported") &&
    getCodexBatchImportProgressPercent(task) >= 100
  ) {
    return "success";
  }
  return "active";
}

export function getCodexBatchImportStatusFromPreview(
  preview: Pick<CodexBatchImportPreviewLike, "status">,
): CodexBatchImportQueueTaskStatus {
  if (preview.status === "cancelled") return "cancelled";
  if (preview.status === "ready") return "ready";
  return "running";
}

export function recoverCodexBatchImportStartedTaskFromPreview<
  T extends CodexBatchImportRecoverableTaskLike,
>(task: T, sessionId: string, preview: CodexBatchImportPreviewLike): T {
  return {
    ...task,
    sessionId,
    status: getCodexBatchImportStatusFromPreview(preview),
    preview,
    selectedIds: mergeCodexBatchImportDefaultSelection(
      task.selectedIds,
      preview.items,
    ),
  };
}

export function buildCodexBatchImportApiServiceAccountIds(
  existingAccountIds: string[],
  selectedItemIds: string[],
  previewItems: CodexBatchImportApiServicePreviewItemLike[],
  importedAccounts: CodexBatchImportApiServiceImportedAccountLike[],
): string[] {
  const selectedItemIdSet = new Set(selectedItemIds);
  const selectedExistingAccountIds = previewItems
    .filter((item) => selectedItemIdSet.has(item.itemId))
    .map((item) => item.accountId?.trim() ?? "")
    .filter((accountId) => accountId.length > 0);
  const importedAccountIds = importedAccounts
    .map((account) => account.id?.trim() ?? "")
    .filter((accountId) => accountId.length > 0);

  return Array.from(
    new Set([
      ...existingAccountIds.map((accountId) => accountId.trim()).filter(Boolean),
      ...selectedExistingAccountIds,
      ...importedAccountIds,
    ]),
  );
}
