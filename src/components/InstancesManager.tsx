import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import {
  Plus,
  Play,
  Pencil,
  Trash2,
  Terminal,
  FolderOpen,
  Square,
  ChevronDown,
  X,
  Search,
  ArrowDownWideNarrow,
  ExternalLink,
  Eye,
  EyeOff,
} from "lucide-react";
import { confirm as confirmDialog, open } from "@tauri-apps/plugin-dialog";
import md5 from "blueimp-md5";
import {
  InstanceInitMode,
  InstanceLaunchMode,
  InstanceProfile,
} from "../types/instance";
import type { PlatformId } from "../types/platform";
import {
  FileCorruptedModal,
  parseFileCorruptedError,
  type FileCorruptedError,
} from "./FileCorruptedModal";
import type { InstanceStoreState } from "../stores/createInstanceStore";
import { showInstanceFloatingCardWindow } from "../services/floatingCardService";
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled,
} from "../utils/privacy";

type MessageState = { text: string; tone?: "error" };
type AccountLike = { id: string; email: string };
type InstanceSortField = "createdAt" | "lastLaunchedAt";
type SortDirection = "asc" | "desc";
type StartInstanceOutcome =
  | "started"
  | "already-running"
  | "missing-path"
  | "failed";
type AccountSelectPortalPosition = {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: "top" | "bottom";
};

interface InstancesManagerProps<TAccount extends AccountLike> {
  instanceStore: InstanceStoreState;
  accounts: TAccount[];
  fetchAccounts: () => Promise<void>;
  renderAccountQuotaPreview: (account: TAccount) => ReactNode;
  renderAccountBadge?: (account: TAccount) => ReactNode;
  getAccountSearchText?: (account: TAccount) => string;
  appType?:
    | "antigravity"
    | "codex"
    | "vscode"
    | "windsurf"
    | "kiro"
    | "cursor"
    | "gemini"
    | "codebuddy"
    | "codebuddy_cn"
    | "qoder"
    | "trae"
    | "workbuddy";
  onInstanceStarted?: (instance: InstanceProfile) => void | Promise<void>;
  resolveStartSuccessMessage?: (instance: InstanceProfile) => string;
}

const INSTANCE_AUTO_REFRESH_INTERVAL_MS = 10_000;
const ACCOUNT_SELECT_PORTAL_GAP = 8;
const ACCOUNT_SELECT_PORTAL_SAFE_MARGIN = 12;
const ACCOUNT_SELECT_PORTAL_MAX_HEIGHT = 320;
const ACCOUNT_SELECT_PORTAL_MIN_HEIGHT = 140;
const ACCOUNT_SELECT_PORTAL_Z_INDEX = 10020;

const resolveAccountSelectPortalPosition = (
  trigger: HTMLButtonElement | null,
): AccountSelectPortalPosition | null => {
  const rect = trigger?.getBoundingClientRect();
  if (!rect) return null;

  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const width = Math.min(
    rect.width,
    viewportWidth - ACCOUNT_SELECT_PORTAL_SAFE_MARGIN * 2,
  );
  const maxLeft = viewportWidth - ACCOUNT_SELECT_PORTAL_SAFE_MARGIN - width;
  const left = Math.min(
    Math.max(ACCOUNT_SELECT_PORTAL_SAFE_MARGIN, rect.left),
    maxLeft,
  );
  const spaceBelow =
    viewportHeight -
    rect.bottom -
    ACCOUNT_SELECT_PORTAL_GAP -
    ACCOUNT_SELECT_PORTAL_SAFE_MARGIN;
  const spaceAbove =
    rect.top -
    ACCOUNT_SELECT_PORTAL_GAP -
    ACCOUNT_SELECT_PORTAL_SAFE_MARGIN;
  const placement: "top" | "bottom" =
    spaceBelow >= ACCOUNT_SELECT_PORTAL_MAX_HEIGHT || spaceBelow >= spaceAbove
      ? "bottom"
      : "top";
  const availableHeight = placement === "bottom" ? spaceBelow : spaceAbove;
  const maxHeight = Math.min(
    ACCOUNT_SELECT_PORTAL_MAX_HEIGHT,
    Math.max(
      availableHeight,
      Math.min(
        ACCOUNT_SELECT_PORTAL_MIN_HEIGHT,
        Math.max(spaceAbove, spaceBelow),
      ),
    ),
  );
  const top =
    placement === "bottom"
      ? Math.min(
          rect.bottom + ACCOUNT_SELECT_PORTAL_GAP,
          viewportHeight - ACCOUNT_SELECT_PORTAL_SAFE_MARGIN,
        )
      : Math.max(
          ACCOUNT_SELECT_PORTAL_SAFE_MARGIN,
          rect.top - ACCOUNT_SELECT_PORTAL_GAP,
        );

  return {
    top,
    left,
    width,
    maxHeight,
    placement,
  };
};

const resolveInstanceSortStorageKeys = (
  appType: InstancesManagerProps<AccountLike>["appType"],
) => ({
  sortField: `agtools.${appType}.instances.sort_field`,
  sortDirection: `agtools.${appType}.instances.sort_direction`,
});

const hashDirName = (name: string) => {
  const trimmed = name.trim();
  if (!trimmed) return "";
  return md5(trimmed).substring(0, 16);
};

const joinPath = (root: string, name: string) => {
  if (!root) return name;
  const sep = root.includes("\\") ? "\\" : "/";
  if (root.endsWith(sep)) return `${root}${name}`;
  return `${root}${sep}${name}`;
};

const resolveFloatingCardPlatformId = (
  appType: NonNullable<InstancesManagerProps<AccountLike>["appType"]>,
): PlatformId => {
  switch (appType) {
    case "vscode":
      return "github-copilot";
    default:
      return appType;
  }
};

export function InstancesManager<TAccount extends AccountLike>({
  instanceStore,
  accounts,
  fetchAccounts,
  renderAccountQuotaPreview,
  renderAccountBadge,
  getAccountSearchText,
  appType = "antigravity",
  onInstanceStarted,
  resolveStartSuccessMessage,
}: InstancesManagerProps<TAccount>) {
  const { t } = useTranslation();
  const {
    instances,
    defaults,
    loading,
    error,
    fetchInstances,
    refreshInstances,
    fetchDefaults,
    createInstance,
    updateInstance,
    deleteInstance,
    startInstance,
    stopInstance,
    openInstanceWindow,
    closeAllInstances,
  } = instanceStore;

  const [message, setMessage] = useState<MessageState | null>(null);
  const [fileCorruptedError, setFileCorruptedError] =
    useState<FileCorruptedError | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [openInlineMenuId, setOpenInlineMenuId] = useState<string | null>(null);
  const [runningNoticeInstance, setRunningNoticeInstance] =
    useState<InstanceProfile | null>(null);
  const [initGuideInstance, setInitGuideInstance] =
    useState<InstanceProfile | null>(null);
  const [deleteConfirmInstance, setDeleteConfirmInstance] =
    useState<InstanceProfile | null>(null);
  const [restartingAll, setRestartingAll] = useState(false);
  const [bulkActionLoading, setBulkActionLoading] = useState(false);

  const [showModal, setShowModal] = useState(false);
  const [editing, setEditing] = useState<InstanceProfile | null>(null);
  const [formName, setFormName] = useState("");
  const [formPath, setFormPath] = useState("");
  const [formWorkingDir, setFormWorkingDir] = useState("");
  const [formExtraArgs, setFormExtraArgs] = useState("");
  const [formInitMode, setFormInitMode] = useState<InstanceInitMode>("copy");
  const [formLaunchMode, setFormLaunchMode] =
    useState<InstanceLaunchMode>("app");
  const [formBindAccountId, setFormBindAccountId] = useState<string>("");
  const [formCopySourceInstanceId, setFormCopySourceInstanceId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const formErrorRef = useRef<HTMLDivElement | null>(null);
  const [formErrorTick, setFormErrorTick] = useState(0);
  const [pathAuto, setPathAuto] = useState(true);
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  const [startingInstanceIds, setStartingInstanceIds] = useState<string[]>([]);
  const [stoppingInstanceIds, setStoppingInstanceIds] = useState<string[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [sortField, setSortField] = useState<InstanceSortField>(() => {
    const keys = resolveInstanceSortStorageKeys(appType);
    const saved = localStorage.getItem(keys.sortField);
    return saved === "lastLaunchedAt" ? "lastLaunchedAt" : "createdAt";
  });
  const [sortDirection, setSortDirection] = useState<SortDirection>(() => {
    const keys = resolveInstanceSortStorageKeys(appType);
    const saved = localStorage.getItem(keys.sortDirection);
    return saved === "desc" ? "desc" : "asc";
  });
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );

  const startingInstanceIdSet = useMemo(
    () => new Set(startingInstanceIds),
    [startingInstanceIds],
  );
  const stoppingInstanceIdSet = useMemo(
    () => new Set(stoppingInstanceIds),
    [stoppingInstanceIds],
  );
  const isGeminiApp = appType === "gemini";
  const isCodexApp = appType === "codex";
  const supportsLaunchModeSelect = isCodexApp;
  const resolveInstanceLaunchMode = (
    instance?: InstanceProfile | null,
  ): InstanceLaunchMode => {
    if (isGeminiApp) {
      return "cli";
    }
    if (isCodexApp) {
      return instance?.launchMode ?? "app";
    }
    return "app";
  };
  const usesTerminalLaunch = (instance: InstanceProfile) =>
    isGeminiApp ||
    (isCodexApp && resolveInstanceLaunchMode(instance) === "cli");
  const supportsStopControl =
    !isGeminiApp && instances.some((item) => !usesTerminalLaunch(item));
  const hidePathFieldInEditModal = isGeminiApp && Boolean(editing?.isDefault);
  const showWorkingDirField =
    isGeminiApp || (supportsLaunchModeSelect && formLaunchMode === "cli");
  const floatingCardPlatformId = useMemo(
    () => resolveFloatingCardPlatformId(appType),
    [appType],
  );

  const markInstanceStarting = useCallback((instanceId: string) => {
    setStartingInstanceIds((prev) =>
      prev.includes(instanceId) ? prev : [...prev, instanceId],
    );
  }, []);

  const unmarkInstanceStarting = useCallback((instanceId: string) => {
    setStartingInstanceIds((prev) => prev.filter((id) => id !== instanceId));
  }, []);

  const replaceStartingInstances = useCallback((instanceIds: string[]) => {
    setStartingInstanceIds(Array.from(new Set(instanceIds)));
  }, []);

  const markInstanceStopping = useCallback((instanceId: string) => {
    setStoppingInstanceIds((prev) =>
      prev.includes(instanceId) ? prev : [...prev, instanceId],
    );
  }, []);

  const unmarkInstanceStopping = useCallback((instanceId: string) => {
    setStoppingInstanceIds((prev) => prev.filter((id) => id !== instanceId));
  }, []);

  const togglePrivacyMode = useCallback(() => {
    setPrivacyModeEnabled((prev) => {
      const next = !prev;
      persistPrivacyModeEnabled(next);
      return next;
    });
  }, []);

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );

  useEffect(() => {
    fetchDefaults();
    fetchInstances();
    fetchAccounts();
  }, [fetchDefaults, fetchInstances, fetchAccounts]);

  useEffect(() => {
    let inFlight = false;
    const timer = window.setInterval(() => {
      if (document.visibilityState === "hidden") return;
      if (inFlight) return;
      inFlight = true;
      Promise.all([refreshInstances(), fetchAccounts()])
        .catch(() => {
          // ignore periodic refresh errors; manual refresh still exposes errors
        })
        .finally(() => {
          inFlight = false;
        });
    }, INSTANCE_AUTO_REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [fetchAccounts, refreshInstances]);

  useEffect(() => {
    if (!error) return;
    const corrupted = parseFileCorruptedError(error);
    if (corrupted) {
      setFileCorruptedError(corrupted);
    } else {
      setMessage({ text: String(error), tone: "error" });
    }
  }, [error]);

  useEffect(() => {
    if (stoppingInstanceIds.length === 0) return;
    const runningIds = new Set(
      instances.filter((item) => item.running).map((item) => item.id),
    );
    setStoppingInstanceIds((prev) => {
      const next = prev.filter((id) => runningIds.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [instances, stoppingInstanceIds.length]);

  useEffect(() => {
    if (!formError || !showModal) return;
    formErrorRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [formError, formErrorTick, showModal]);

  useEffect(() => {
    const keys = resolveInstanceSortStorageKeys(appType);
    localStorage.setItem(keys.sortField, sortField);
  }, [appType, sortField]);

  useEffect(() => {
    const keys = resolveInstanceSortStorageKeys(appType);
    localStorage.setItem(keys.sortDirection, sortDirection);
  }, [appType, sortDirection]);

  const sortedInstances = useMemo(
    () =>
      [...instances].sort((a, b) => {
        if (a.isDefault && !b.isDefault) return -1;
        if (!a.isDefault && b.isDefault) return 1;
        const av =
          sortField === "createdAt" ? a.createdAt || 0 : a.lastLaunchedAt || 0;
        const bv =
          sortField === "createdAt" ? b.createdAt || 0 : b.lastLaunchedAt || 0;
        return sortDirection === "asc" ? av - bv : bv - av;
      }),
    [instances, sortDirection, sortField],
  );

  const defaultInstanceId = useMemo(() => {
    const defaultInstance = instances.find((item) => item.isDefault);
    return defaultInstance?.id || "__default__";
  }, [instances]);

  const filteredInstances = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) return sortedInstances;
    return sortedInstances.filter((instance) => {
      const displayName = instance.isDefault
        ? t("instances.defaultName", "默认实例")
        : instance.name || "";
      const account = instance.bindAccountId
        ? accounts.find((item) => item.id === instance.bindAccountId) || null
        : null;
      const accountText = account
        ? getAccountSearchText
          ? getAccountSearchText(account)
          : account.email
        : "";
      const haystack = [displayName, accountText, instance.userDataDir || ""]
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [accounts, getAccountSearchText, searchQuery, sortedInstances, t]);

  const defaultRoot = defaults?.rootDir ?? "";

  const buildDefaultPath = (name: string) => {
    if (!defaultRoot) return "";
    const segment = hashDirName(name);
    if (!segment) return defaultRoot;
    return joinPath(defaultRoot, segment);
  };

  useEffect(() => {
    if (editing || !pathAuto || !defaultRoot || formInitMode === "existingDir")
      return;
    const nextPath = buildDefaultPath(formName);
    if (nextPath && nextPath !== formPath) {
      setFormPath(nextPath);
    }
  }, [defaultRoot, editing, formName, pathAuto, formInitMode]);

  const resetForm = (showRoot = false) => {
    setFormName("");
    setFormPath(showRoot && defaultRoot ? defaultRoot : "");
    setFormWorkingDir("");
    setFormExtraArgs("");
    setFormInitMode("copy");
    setFormLaunchMode(isGeminiApp ? "cli" : "app");
    setFormBindAccountId("");
    setFormCopySourceInstanceId(defaultInstanceId);
    setFormError(null);
    setPathAuto(true);
  };

  const openCreateModal = () => {
    setOpenInlineMenuId(null);
    resetForm(true);
    setEditing(null);
    setShowModal(true);
  };

  useEffect(() => {
    if (!showModal || editing) return;
    if (!formCopySourceInstanceId) {
      setFormCopySourceInstanceId(defaultInstanceId);
    }
  }, [defaultInstanceId, editing, formCopySourceInstanceId, showModal]);

  useEffect(() => {
    if (editing) return;
    if (formInitMode === "empty") {
      setFormBindAccountId("");
      return;
    }
    if (!formCopySourceInstanceId) {
      setFormCopySourceInstanceId(defaultInstanceId);
    }
  }, [defaultInstanceId, editing, formCopySourceInstanceId, formInitMode]);

  const openEditModal = (instance: InstanceProfile) => {
    setOpenInlineMenuId(null);
    setEditing(instance);
    setFormName(
      instance.isDefault
        ? t("instances.defaultName", "默认实例")
        : instance.name || "",
    );
    setFormPath(instance.userDataDir || "");
    setFormWorkingDir(instance.workingDir || "");
    setFormExtraArgs(instance.extraArgs || "");
    setFormInitMode("copy");
    setFormLaunchMode(resolveInstanceLaunchMode(instance));
    setFormBindAccountId(instance.bindAccountId || "");
    setFormError(null);
    setPathAuto(false);
    setShowModal(true);
  };

  const closeModal = () => {
    setOpenInlineMenuId(null);
    setShowModal(false);
    resetForm();
    setEditing(null);
  };

  const handleNameChange = (value: string) => {
    setFormName(value);
    if (!editing && defaultRoot && formInitMode !== "existingDir") {
      const nextPath = buildDefaultPath(value);
      if (nextPath) {
        setFormPath(nextPath);
      }
    }
  };

  const handleSelectPath = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: defaultRoot || undefined,
      });
      if (selected && typeof selected === "string") {
        setFormPath(selected);
      }
    } catch (e) {
      setFormError(String(e));
      setFormErrorTick((prev) => prev + 1);
    }
  };

  const handleSelectWorkingDir = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
      });
      if (selected && typeof selected === "string") {
        setFormWorkingDir(selected);
      }
    } catch (e) {
      setFormError(String(e));
      setFormErrorTick((prev) => prev + 1);
    }
  };

  const handleSubmit = async () => {
    setFormError(null);
    setMessage(null);
    const isEditingDefault = Boolean(editing?.isDefault);
    const isCreateEmpty = !editing && formInitMode === "empty";

    if (!isEditingDefault) {
      if (!formName.trim()) {
        setFormError(t("instances.form.nameRequired", "请输入实例名称"));
        setFormErrorTick((prev) => prev + 1);
        return;
      }
      if (!formPath.trim()) {
        setFormError(t("instances.form.pathRequired", "请选择实例目录"));
        setFormErrorTick((prev) => prev + 1);
        return;
      }
    }

    const isExistingDir = !editing && formInitMode === "existingDir";

    if (
      !editing &&
      !isCreateEmpty &&
      !isExistingDir &&
      !formCopySourceInstanceId
    ) {
      setFormError(
        t("instances.form.copySourceRequired", "请选择复制来源实例"),
      );
      setFormErrorTick((prev) => prev + 1);
      return;
    }

    if (!editing && !isCreateEmpty && !isExistingDir && !formBindAccountId) {
      setFormError(t("instances.form.bindRequired", "请选择要绑定的账号"));
      setFormErrorTick((prev) => prev + 1);
      return;
    }

    try {
      const nextLaunchMode = supportsLaunchModeSelect
        ? formLaunchMode
        : undefined;
      const nextWorkingDir = showWorkingDirField ? formWorkingDir : null;
      if (editing) {
        setActionLoading(editing.id);
        const updatePayload: {
          instanceId: string;
          name?: string;
          workingDir?: string | null;
          extraArgs?: string;
          bindAccountId?: string | null;
          followLocalAccount?: boolean;
          launchMode?: InstanceLaunchMode;
        } = {
          instanceId: editing.id,
          workingDir: nextWorkingDir,
          extraArgs: formExtraArgs,
          launchMode: nextLaunchMode,
        };
        if (!isEditingDefault) {
          updatePayload.name = formName.trim();
        }
        const canEditBind = !(
          editing.initialized === false && !isEditingDefault
        );
        if (canEditBind) {
          const nextBindId = formBindAccountId;
          updatePayload.bindAccountId = nextBindId;
        }
        if (isEditingDefault) {
          updatePayload.followLocalAccount = false;
        }

        await updateInstance(updatePayload);
        setMessage({ text: t("instances.messages.updated", "实例已更新") });
      } else {
        setActionLoading("create");
        await createInstance({
          name: formName.trim(),
          userDataDir: formPath.trim(),
          workingDir: nextWorkingDir,
          extraArgs: formExtraArgs,
          initMode: formInitMode,
          launchMode: nextLaunchMode,
          bindAccountId: isCreateEmpty ? null : formBindAccountId,
          copySourceInstanceId: formCopySourceInstanceId || defaultInstanceId,
        });
        setMessage({
          text: isCreateEmpty
            ? t(
                "instances.messages.emptyCreated",
                "空白实例已创建，请先启动一次后再绑定账号",
              )
            : t("instances.messages.created", "实例已创建"),
        });
      }
      closeModal();
    } catch (e) {
      setFormError(String(e));
    } finally {
      setActionLoading(null);
    }
  };

  const handleDelete = (instance: InstanceProfile) => {
    setDeleteConfirmInstance(instance);
  };

  const handleConfirmDelete = async () => {
    if (!deleteConfirmInstance) return;
    const target = deleteConfirmInstance;
    setActionLoading(target.id);
    try {
      await deleteInstance(target.id);
      setMessage({ text: t("instances.messages.deleted", "实例已删除") });
      setDeleteConfirmInstance(null);
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setActionLoading(null);
    }
  };

  const handleMissingPathError = (error: unknown, instanceId?: string) => {
    const message = String(error ?? "");
    if (!message.startsWith("APP_PATH_NOT_FOUND:")) {
      return false;
    }
    const rawApp = message.slice("APP_PATH_NOT_FOUND:".length);
    const app =
      rawApp === "codex" ||
      rawApp === "antigravity" ||
      rawApp === "vscode" ||
      rawApp === "windsurf" ||
      rawApp === "kiro" ||
      rawApp === "cursor" ||
      rawApp === "gemini" ||
      rawApp === "codebuddy" ||
      rawApp === "codebuddy_cn" ||
      rawApp === "qoder"
        ? rawApp
        : appType;
    const retry = instanceId
      ? { kind: "instance" as const, instanceId }
      : { kind: "default" as const };
    window.dispatchEvent(
      new CustomEvent("app-path-missing", { detail: { app, retry } }),
    );
    return true;
  };

  const triggerDelayedRefreshAfterStart = () => {
    window.setTimeout(() => {
      refreshInstances().catch(() => {
        // ignore delayed refresh errors
      });
    }, 2000);
  };

  const startStoppedInstance = useCallback(
    async (
      instance: InstanceProfile,
      options?: {
        showRunningNotice?: boolean;
        showSuccessMessage?: boolean;
        preMarkedStarting?: boolean;
      },
    ): Promise<StartInstanceOutcome> => {
      const showRunningNotice = options?.showRunningNotice ?? false;
      const showSuccessMessage = options?.showSuccessMessage ?? true;
      const preMarkedStarting = options?.preMarkedStarting ?? false;

      if (instance.running) {
        if (showRunningNotice) {
          setRunningNoticeInstance(instance);
        }
        return "already-running";
      }

      if (!preMarkedStarting) {
        markInstanceStarting(instance.id);
      }

      try {
        const startedInstance = await startInstance(instance.id);
        let startHookError: string | null = null;
        if (onInstanceStarted) {
          try {
            await onInstanceStarted(startedInstance);
          } catch (callbackError) {
            startHookError = String(callbackError);
            setMessage({ text: startHookError, tone: "error" });
          }
        }
        triggerDelayedRefreshAfterStart();
        if (showSuccessMessage && !startHookError) {
          const successMessage = resolveStartSuccessMessage
            ? resolveStartSuccessMessage(startedInstance)
            : t("instances.messages.started", "实例已启动");
          setMessage({ text: successMessage });
        }
        return "started";
      } catch (e) {
        if (handleMissingPathError(e, instance.id)) {
          return "missing-path";
        }
        setMessage({ text: String(e), tone: "error" });
        return "failed";
      } finally {
        if (!preMarkedStarting) {
          unmarkInstanceStarting(instance.id);
        }
      }
    },
    [
      handleMissingPathError,
      markInstanceStarting,
      onInstanceStarted,
      resolveStartSuccessMessage,
      startInstance,
      t,
      triggerDelayedRefreshAfterStart,
      unmarkInstanceStarting,
    ],
  );

  const handleStart = async (instance: InstanceProfile) => {
    await startStoppedInstance(instance, {
      showRunningNotice: supportsStopControl && !usesTerminalLaunch(instance),
      showSuccessMessage: true,
    });
  };

  const handleStop = async (instance: InstanceProfile) => {
    try {
      const confirmed = await confirmDialog(
        t(
          "instances.stop.message",
          "将向实例进程发送终止信号（SIGTERM）强制关闭，可能导致未保存的数据丢失。确认继续？",
        ),
        {
          title: t("instances.stop.title", "强制关闭实例"),
          kind: "warning",
        },
      );
      if (!confirmed) return;
    } catch {
      // ignore dialog errors
    }

    markInstanceStopping(instance.id);
    try {
      await stopInstance(instance.id);
      setMessage({ text: t("instances.messages.stopped", "实例已关闭") });
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      unmarkInstanceStopping(instance.id);
    }
  };

  const handleOpenRunningInstance = async () => {
    if (!runningNoticeInstance) return;
    try {
      await openInstanceWindow(runningNoticeInstance.id);
      setRunningNoticeInstance(null);
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    }
  };

  const handleLocateInstance = async (instance: InstanceProfile) => {
    if (!instance.running) return;
    setActionLoading(instance.id);
    try {
      await openInstanceWindow(instance.id);
    } catch (e) {
      if (handleMissingPathError(e, instance.id)) {
        return;
      }
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setActionLoading(null);
    }
  };

  const handleShowFloatingCard = async (instance: InstanceProfile) => {
    const { account, missing } = resolveAccount(instance);
    if (!instance.bindAccountId || !account || missing) {
      return;
    }
    try {
      await showInstanceFloatingCardWindow({
        platformId: floatingCardPlatformId,
        instanceId: instance.id,
        instanceName: instance.isDefault
          ? t("instances.defaultName", "默认实例")
          : instance.name || t("instances.defaultName", "默认实例"),
        boundAccountId: instance.bindAccountId,
      });
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    }
  };

  const handleForceRestart = async () => {
    if (!runningNoticeInstance) return;
    const target = runningNoticeInstance;
    setRunningNoticeInstance(null);
    setActionLoading(target.id);
    try {
      await stopInstance(target.id);
      const latest = await refreshInstances();
      const refreshedTarget = latest.find((item) => item.id === target.id) || {
        ...target,
        running: false,
      };
      await startStoppedInstance(refreshedTarget, {
        showSuccessMessage: true,
      });
    } catch (e) {
      if (handleMissingPathError(e, target.id)) {
        return;
      }
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setRestartingAll(false);
      setActionLoading(null);
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await Promise.all([refreshInstances(), fetchAccounts()]);
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setRefreshing(false);
    }
  };

  const handleStartAll = async () => {
    const confirmed = await confirmDialog(t("instances.bulkConfirm.startAll"), {
      title: t("common.confirm"),
      okLabel: t("common.confirm"),
      cancelLabel: t("common.cancel"),
    });
    if (!confirmed) return;
    setBulkActionLoading(true);
    try {
      const latest = await refreshInstances();
      const stoppedIds = latest
        .filter((item) => !item.running)
        .map((item) => item.id);
      if (stoppedIds.length === 0) {
        setMessage({
          text: t("instances.messages.allAlreadyRunning", "所有实例已在运行"),
        });
        return;
      }
      replaceStartingInstances(stoppedIds);

      let startedCount = 0;
      for (const id of stoppedIds) {
        const current = await refreshInstances();
        const target = current.find((item) => item.id === id);
        if (!target || target.running) {
          unmarkInstanceStarting(id);
          continue;
        }

        const outcome = await startStoppedInstance(target, {
          showSuccessMessage: false,
          preMarkedStarting: true,
        });
        unmarkInstanceStarting(id);

        if (outcome === "started") {
          startedCount += 1;
          continue;
        }
        if (outcome === "already-running") {
          continue;
        }
        return;
      }

      if (startedCount > 0) {
        setMessage({
          text: t("instances.messages.startedAll", "已启动所有未运行实例"),
        });
      } else {
        setMessage({
          text: t("instances.messages.allAlreadyRunning", "所有实例已在运行"),
        });
      }
    } catch (e) {
      if (handleMissingPathError(e)) {
        return;
      }
      setMessage({ text: String(e), tone: "error" });
    } finally {
      replaceStartingInstances([]);
      setBulkActionLoading(false);
    }
  };

  const handleCloseAll = async () => {
    const confirmed = await confirmDialog(t("instances.bulkConfirm.stopAll"), {
      title: t("common.confirm"),
      okLabel: t("common.confirm"),
      cancelLabel: t("common.cancel"),
    });
    if (!confirmed) return;
    setBulkActionLoading(true);
    try {
      await refreshInstances();
      await closeAllInstances();
      setMessage({ text: t("instances.messages.closedAll", "已关闭所有实例") });
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setBulkActionLoading(false);
    }
  };

  const resolveAccount = (instance: InstanceProfile) => {
    if (!instance.bindAccountId) {
      return { account: null, missing: false };
    }
    const account =
      accounts.find((item) => item.id === instance.bindAccountId) || null;
    return { account, missing: !account };
  };

  const selectedCopySourceInstance = useMemo(() => {
    if (!formCopySourceInstanceId) {
      return instances.find((item) => item.id === defaultInstanceId) || null;
    }
    return (
      instances.find((item) => item.id === formCopySourceInstanceId) || null
    );
  }, [defaultInstanceId, formCopySourceInstanceId, instances]);

  type BaseAccountSelectProps = {
    value: string | null;
    onChange: (nextId: string | null) => void;
    allowUnbound?: boolean;
    allowFollowCurrent?: boolean;
    isFollowingCurrent?: boolean;
    onFollowCurrent?: () => void;
    disabled?: boolean;
    missing?: boolean;
    placeholder?: string;
  };

  const renderAccountMenuItems = ({
    value,
    isFollowingCurrent = false,
    allowFollowCurrent = false,
    allowUnbound = false,
    onFollowCurrent,
    onChange,
    onClose,
    selectedAccount,
  }: {
    value: string | null;
    isFollowingCurrent?: boolean;
    allowFollowCurrent?: boolean;
    allowUnbound?: boolean;
    onFollowCurrent?: () => void;
    onChange: (nextId: string | null) => void;
    onClose: () => void;
    selectedAccount: TAccount | null;
  }) => (
    <>
      {allowFollowCurrent && (
        <button
          type="button"
          className={`account-select-item ${isFollowingCurrent ? "active" : ""}`}
          data-account-select-active={isFollowingCurrent ? "true" : undefined}
          onClick={() => {
            if (onFollowCurrent) {
              onFollowCurrent();
            } else {
              onChange(null);
            }
            onClose();
          }}
        >
          <span className="account-select-email-row">
            <span className="account-select-email">
              {t("instances.form.followCurrent", "跟随当前账号")}
            </span>
            {selectedAccount ? renderAccountBadge?.(selectedAccount) : null}
          </span>
          {selectedAccount ? renderAccountQuotaPreview(selectedAccount) : null}
        </button>
      )}
      {allowUnbound && (
        <button
          type="button"
          className={`account-select-item ${!value && !isFollowingCurrent ? "active" : ""}`}
          data-account-select-active={
            !value && !isFollowingCurrent ? "true" : undefined
          }
          onClick={() => {
            onChange(null);
            onClose();
          }}
        >
          <span className="account-select-email muted">
            {t("instances.form.unbound", "不绑定")}
          </span>
        </button>
      )}
      {accounts.map((account) => (
        <button
          type="button"
          key={account.id}
          className={`account-select-item ${value === account.id && !isFollowingCurrent ? "active" : ""}`}
          data-account-select-active={
            value === account.id && !isFollowingCurrent ? "true" : undefined
          }
          onClick={() => {
            onChange(account.id);
            onClose();
          }}
        >
          <span className="account-select-email-row">
            <span
              className="account-select-email"
              title={maskAccountText(account.email)}
            >
              {maskAccountText(account.email)}
            </span>
            {renderAccountBadge?.(account)}
          </span>
          {renderAccountQuotaPreview(account)}
        </button>
      ))}
    </>
  );

  type InlineAccountSelectProps = BaseAccountSelectProps & {
    onOpenChange?: (open: boolean) => void;
    instanceId?: string;
    currentOpenId?: string | null;
  };

  const InlineAccountSelect = ({
    value,
    onChange,
    allowUnbound = false,
    allowFollowCurrent = false,
    isFollowingCurrent = false,
    onFollowCurrent,
    onOpenChange,
    disabled = false,
    missing = false,
    placeholder,
    instanceId,
    currentOpenId,
  }: InlineAccountSelectProps) => {
    const menuRef = useRef<HTMLDivElement | null>(null);
    const triggerRef = useRef<HTMLButtonElement | null>(null);
    const portalMenuRef = useRef<HTMLDivElement | null>(null);
    const isOpen = instanceId ? currentOpenId === instanceId : false;
    const [portalPos, setPortalPos] =
      useState<AccountSelectPortalPosition | null>(null);

    const updatePortalPos = useCallback(() => {
      setPortalPos(resolveAccountSelectPortalPosition(triggerRef.current));
    }, []);

    useEffect(() => {
      if (!isOpen) return;
      updatePortalPos();

      const handleClick = (event: MouseEvent) => {
        const target = event.target as Node;
        const inTrigger = Boolean(
          menuRef.current && menuRef.current.contains(target),
        );
        const inPortalMenu = Boolean(
          portalMenuRef.current && portalMenuRef.current.contains(target),
        );
        if (!inTrigger && !inPortalMenu) {
          onOpenChange?.(false);
        }
      };
      // 使用 setTimeout 延迟添加监听器，避免与打开菜单的点击事件冲突
      const timer = setTimeout(() => {
        document.addEventListener("click", handleClick);
      }, 0);
      window.addEventListener("resize", updatePortalPos);
      window.addEventListener("scroll", updatePortalPos, true);
      return () => {
        clearTimeout(timer);
        document.removeEventListener("click", handleClick);
        window.removeEventListener("resize", updatePortalPos);
        window.removeEventListener("scroll", updatePortalPos, true);
      };
    }, [isOpen, onOpenChange, updatePortalPos]);

    useEffect(() => {
      if (!isOpen || !portalPos || !portalMenuRef.current) return;

      const frameId = window.requestAnimationFrame(() => {
        const activeItem = portalMenuRef.current?.querySelector<HTMLElement>(
          '[data-account-select-active="true"]',
        );
        activeItem?.scrollIntoView({
          block: "nearest",
          behavior: "smooth",
        });
      });

      return () => {
        window.cancelAnimationFrame(frameId);
      };
    }, [
      accounts.length,
      isFollowingCurrent,
      isOpen,
      portalPos?.placement,
      value,
    ]);

    useEffect(() => {
      if (disabled && isOpen) {
        onOpenChange?.(false);
      }
    }, [disabled, isOpen, onOpenChange]);

    const selectedAccount = accounts.find((item) => item.id === value) || null;
    const basePlaceholder =
      placeholder ||
      (allowUnbound
        ? t("instances.form.unbound", "不绑定")
        : t("instances.form.selectAccount", "选择账号"));
    const selectedLabel = missing
      ? t("instances.quota.accountMissing", "账号不存在")
      : isFollowingCurrent
        ? maskAccountText(selectedAccount?.email) ||
          t("instances.form.followCurrent", "跟随当前账号")
        : maskAccountText(selectedAccount?.email) || basePlaceholder;
    const selectedBadge =
      !missing && selectedAccount
        ? renderAccountBadge?.(selectedAccount)
        : null;
    const selectedQuota = selectedAccount
      ? renderAccountQuotaPreview(selectedAccount)
      : null;

    return (
      <div
        className={`account-select ${disabled ? "disabled" : ""}`}
        ref={menuRef}
      >
        <button
          ref={triggerRef}
          type="button"
          className={`account-select-trigger ${isOpen ? "open" : ""}`}
          onClick={() => {
            if (disabled) return;
            onOpenChange?.(!isOpen);
          }}
          disabled={disabled}
        >
          <span className="account-select-content">
            <span className="account-select-label-row">
              <span className="account-select-label" title={selectedLabel}>
                {selectedLabel}
              </span>
              {selectedBadge}
            </span>
            {selectedQuota && (
              <span className="account-select-meta">{selectedQuota}</span>
            )}
          </span>
          <span className="account-select-arrow">
            <ChevronDown size={14} />
          </span>
        </button>
        {isOpen && !disabled && portalPos
            ? createPortal(
              <div
                className={`instances-page account-select-portal-root ${portalPos.placement === "top" ? "placement-top" : "placement-bottom"}`}
                style={{
                  position: "fixed",
                  top: `${portalPos.top}px`,
                  left: `${portalPos.left}px`,
                  width: `${portalPos.width}px`,
                  ["--account-select-max-height" as string]: `${portalPos.maxHeight}px`,
                  zIndex: ACCOUNT_SELECT_PORTAL_Z_INDEX,
                }}
              >
                <div ref={portalMenuRef} className="account-select-menu">
                  {renderAccountMenuItems({
                    value,
                    isFollowingCurrent,
                    allowFollowCurrent,
                    allowUnbound,
                    onFollowCurrent,
                    onChange,
                    onClose: () => onOpenChange?.(false),
                    selectedAccount,
                  })}
                </div>
              </div>,
              document.body,
            )
          : null}
      </div>
    );
  };

  type FormAccountSelectProps = BaseAccountSelectProps;

  const FormAccountSelect = ({
    value,
    onChange,
    allowUnbound = false,
    allowFollowCurrent = false,
    isFollowingCurrent = false,
    onFollowCurrent,
    disabled = false,
    missing = false,
    placeholder,
  }: FormAccountSelectProps) => {
    const menuRef = useRef<HTMLDivElement | null>(null);
    const triggerRef = useRef<HTMLButtonElement | null>(null);
    const portalMenuRef = useRef<HTMLDivElement | null>(null);
    const [open, setOpen] = useState(false);
    const [portalPos, setPortalPos] =
      useState<AccountSelectPortalPosition | null>(null);

    const updatePortalPos = useCallback(() => {
      setPortalPos(resolveAccountSelectPortalPosition(triggerRef.current));
    }, []);

    useEffect(() => {
      if (!open) return;
      const handleClick = (event: MouseEvent) => {
        const target = event.target as Node;
        const inTrigger = Boolean(
          menuRef.current && menuRef.current.contains(target),
        );
        const inPortalMenu = Boolean(
          portalMenuRef.current && portalMenuRef.current.contains(target),
        );
        if (!inTrigger && !inPortalMenu) {
          setOpen(false);
        }
      };
      updatePortalPos();
      const timer = setTimeout(() => {
        document.addEventListener("click", handleClick);
      }, 0);
      window.addEventListener("resize", updatePortalPos);
      window.addEventListener("scroll", updatePortalPos, true);
      return () => {
        clearTimeout(timer);
        document.removeEventListener("click", handleClick);
        window.removeEventListener("resize", updatePortalPos);
        window.removeEventListener("scroll", updatePortalPos, true);
      };
    }, [open, updatePortalPos]);

    useEffect(() => {
      if (!open || !portalPos || !portalMenuRef.current) return;

      const frameId = window.requestAnimationFrame(() => {
        const activeItem = portalMenuRef.current?.querySelector<HTMLElement>(
          '[data-account-select-active="true"]',
        );
        activeItem?.scrollIntoView({
          block: "nearest",
          behavior: "smooth",
        });
      });

      return () => {
        window.cancelAnimationFrame(frameId);
      };
    }, [accounts.length, isFollowingCurrent, open, portalPos?.placement, value]);

    useEffect(() => {
      if (disabled && open) {
        setOpen(false);
      }
    }, [disabled, open]);

    const selectedAccount = accounts.find((item) => item.id === value) || null;
    const basePlaceholder =
      placeholder ||
      (allowUnbound
        ? t("instances.form.unbound", "不绑定")
        : t("instances.form.selectAccount", "选择账号"));
    const selectedLabel = missing
      ? t("instances.quota.accountMissing", "账号不存在")
      : isFollowingCurrent
        ? maskAccountText(selectedAccount?.email) ||
          t("instances.form.followCurrent", "跟随当前账号")
        : maskAccountText(selectedAccount?.email) || basePlaceholder;
    const selectedBadge =
      !missing && selectedAccount
        ? renderAccountBadge?.(selectedAccount)
        : null;
    const selectedQuota = selectedAccount
      ? renderAccountQuotaPreview(selectedAccount)
      : null;

    return (
      <div
        className={`account-select ${disabled ? "disabled" : ""}`}
        ref={menuRef}
      >
        <button
          ref={triggerRef}
          type="button"
          className={`account-select-trigger ${open ? "open" : ""}`}
          onClick={() => {
            if (disabled) return;
            setOpen((prev) => !prev);
          }}
          disabled={disabled}
        >
          <span className="account-select-content">
            <span className="account-select-label-row">
              <span className="account-select-label" title={selectedLabel}>
                {selectedLabel}
              </span>
              {selectedBadge}
            </span>
            {selectedQuota && (
              <span className="account-select-meta">{selectedQuota}</span>
            )}
          </span>
          <span className="account-select-arrow">
            <ChevronDown size={14} />
          </span>
        </button>
        {open && !disabled && portalPos
          ? createPortal(
              <div
                className={`instances-page account-select-portal-root ${portalPos.placement === "top" ? "placement-top" : "placement-bottom"}`}
                style={{
                  position: "fixed",
                  top: `${portalPos.top}px`,
                  left: `${portalPos.left}px`,
                  width: `${portalPos.width}px`,
                  ["--account-select-max-height" as string]: `${portalPos.maxHeight}px`,
                  zIndex: ACCOUNT_SELECT_PORTAL_Z_INDEX,
                }}
              >
                <div ref={portalMenuRef} className="account-select-menu">
                  {renderAccountMenuItems({
                    value,
                    isFollowingCurrent,
                    allowFollowCurrent,
                    allowUnbound,
                    onFollowCurrent,
                    onChange,
                    onClose: () => setOpen(false),
                    selectedAccount,
                  })}
                </div>
              </div>,
              document.body,
            )
          : null}
      </div>
    );
  };

  type InstanceSelectProps = {
    value: string;
    onChange: (nextId: string) => void;
    disabled?: boolean;
  };

  const InstanceSelect = ({
    value,
    onChange,
    disabled = false,
  }: InstanceSelectProps) => {
    const [open, setOpen] = useState(false);
    const menuRef = useRef<HTMLDivElement | null>(null);

    useEffect(() => {
      if (!open) return;
      const handleClick = (event: MouseEvent) => {
        if (
          menuRef.current &&
          !menuRef.current.contains(event.target as Node)
        ) {
          setOpen(false);
        }
      };
      document.addEventListener("mousedown", handleClick);
      return () => {
        document.removeEventListener("mousedown", handleClick);
      };
    }, [open]);

    useEffect(() => {
      if (disabled && open) {
        setOpen(false);
      }
    }, [disabled, open]);

    const selected =
      sortedInstances.find((item) => item.id === value) ||
      sortedInstances.find((item) => item.isDefault) ||
      null;
    const selectedLabel = selected
      ? selected.isDefault
        ? t("instances.defaultName", "默认实例")
        : selected.name || ""
      : value === "__default__"
        ? t("instances.defaultName", "默认实例")
        : t("instances.form.copySourcePlaceholder", "选择来源实例");

    return (
      <div
        className={`account-select ${disabled ? "disabled" : ""}`}
        ref={menuRef}
      >
        <button
          type="button"
          className={`account-select-trigger ${open ? "open" : ""}`}
          onClick={() => {
            if (disabled) return;
            setOpen((prev) => !prev);
          }}
          disabled={disabled}
        >
          <span className="account-select-label" title={selectedLabel}>
            {selectedLabel}
          </span>
          <span className="account-select-meta">
            <ChevronDown size={14} />
          </span>
        </button>
        {open && !disabled && (
          <div className="account-select-menu">
            {sortedInstances.length === 0 ? (
              <div className="account-select-item active">
                <span className="account-select-email muted">
                  {t("instances.defaultName", "默认实例")}
                </span>
              </div>
            ) : (
              sortedInstances.map((instance) => {
                const label = instance.isDefault
                  ? t("instances.defaultName", "默认实例")
                  : instance.name || "";
                return (
                  <button
                    type="button"
                    key={instance.id}
                    className={`account-select-item ${value === instance.id ? "active" : ""}`}
                    onClick={() => {
                      onChange(instance.id);
                      setOpen(false);
                    }}
                    title={instance.userDataDir}
                  >
                    <span className="account-select-email">{label}</span>
                  </button>
                );
              })
            )}
          </div>
        )}
      </div>
    );
  };

  const handleFormAccountChange = (nextId: string | null) => {
    setFormBindAccountId(nextId ?? "");
  };

  const handleInitGuideStart = async () => {
    if (!initGuideInstance) return;
    const target = initGuideInstance;
    setActionLoading(target.id);
    try {
      const outcome = await startStoppedInstance(target, {
        showSuccessMessage: true,
      });
      if (outcome !== "started") {
        return;
      }
      setInitGuideInstance(null);
      setOpenInlineMenuId(target.id);
    } finally {
      setActionLoading(null);
    }
  };

  const handleInlineBindChange = async (
    instance: InstanceProfile,
    nextId: string | null,
  ) => {
    if (instance.initialized === false) {
      setInitGuideInstance(instance);
      return;
    }
    if (!nextId) return;
    const sameSelection = (instance.bindAccountId || null) === nextId;
    if (sameSelection && !instance.followLocalAccount) return;
    setActionLoading(instance.id);
    try {
      await updateInstance({
        instanceId: instance.id,
        bindAccountId: nextId,
        followLocalAccount: instance.isDefault ? false : undefined,
      });
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setActionLoading(null);
    }
  };

  return (
    <>
      {fileCorruptedError && (
        <FileCorruptedModal
          error={fileCorruptedError}
          onClose={() => setFileCorruptedError(null)}
        />
      )}

      <div className="toolbar instances-toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input
              type="text"
              placeholder={t("instances.search", "搜索实例")}
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
            />
          </div>
          <div className="sort-select">
            <ArrowDownWideNarrow size={14} className="sort-icon" />
            <select
              value={sortField}
              onChange={(event) =>
                setSortField(event.target.value as InstanceSortField)
              }
              aria-label={t("instances.sort.label", "排序")}
            >
              <option value="createdAt">
                {t("instances.sort.createdAt", "按创建时间")}
              </option>
              <option value="lastLaunchedAt">
                {t("instances.sort.lastLaunchedAt", "按启动时间")}
              </option>
            </select>
          </div>
          <button
            className="sort-direction-btn"
            onClick={() =>
              setSortDirection((prev) => (prev === "asc" ? "desc" : "asc"))
            }
            title={
              sortDirection === "asc"
                ? t("instances.sort.ascTooltip", "当前：正序，点击切换为倒序")
                : t("instances.sort.descTooltip", "当前：倒序，点击切换为正序")
            }
            aria-label={t("instances.sort.toggleDirection", "切换排序方向")}
          >
            {sortDirection === "asc" ? "⬆" : "⬇"}
          </button>
          <button
            className="sort-direction-btn"
            onClick={togglePrivacyMode}
            title={
              privacyModeEnabled
                ? t("privacy.showSensitive", "显示邮箱")
                : t("privacy.hideSensitive", "隐藏邮箱")
            }
            aria-label={
              privacyModeEnabled
                ? t("privacy.showSensitive", "显示邮箱")
                : t("privacy.hideSensitive", "隐藏邮箱")
            }
          >
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
        </div>
        <div className="toolbar-right">
          <button
            className="btn btn-primary"
            onClick={openCreateModal}
            title={t("instances.actions.create", "新建实例")}
          >
            <Plus size={16} />
          </button>
          <button
            className="btn btn-secondary"
            onClick={handleStartAll}
            disabled={bulkActionLoading || restartingAll}
            title={t("instances.actions.startAll", "全部启动")}
          >
            <Play size={16} />
          </button>
          {supportsStopControl && (
            <button
              className="btn btn-secondary"
              onClick={handleCloseAll}
              disabled={bulkActionLoading || restartingAll}
              title={t("instances.actions.stopAll", "全部关闭")}
            >
              <Square size={16} />
            </button>
          )}
          <button
            className="btn btn-secondary"
            onClick={handleRefresh}
            disabled={refreshing || bulkActionLoading || restartingAll}
          >
            {t("instances.actions.refresh", "刷新")}
          </button>
        </div>
      </div>

      {message && (
        <div
          className={`action-message${message.tone ? ` ${message.tone}` : ""}`}
        >
          <span className="action-message-text">{message.text}</span>
          <button
            className="action-message-close"
            onClick={() => setMessage(null)}
            aria-label={t("common.close", "关闭")}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {loading ? (
        <div className="loading-state">{t("common.loading", "加载中...")}</div>
      ) : sortedInstances.length === 0 ? (
        <div className="empty-state">
          <h3>{t("instances.empty.title", "还没有实例")}</h3>
          <p>
            {t(
              "instances.empty.desc",
              "创建一个独立配置目录，快速开启多实例。",
            )}
          </p>
          <button className="btn btn-primary" onClick={openCreateModal}>
            <Plus size={16} />
            {t("instances.actions.create", "新建实例")}
          </button>
        </div>
      ) : (
        <div
          className={`instances-list${isGeminiApp ? " instances-list-no-pid" : ""}`}
        >
          <div className="instances-list-header">
            <div></div>
            <div>{t("instances.columns.instance", "实例")}</div>
            <div></div>
            <div>{t("instances.columns.email", "账号")}</div>
            <div>PID</div>
            <div>{t("instances.columns.actions", "操作")}</div>
          </div>
          {filteredInstances.map((instance) => {
            const { missing: accountMissing } = resolveAccount(instance);
            const accountDisabledByInit =
              !instance.isDefault && instance.initialized === false;
            const isInstanceStarting = startingInstanceIdSet.has(instance.id);
            const isInstanceStopping = stoppingInstanceIdSet.has(instance.id);
            const isInstanceBusy =
              actionLoading === instance.id ||
              isInstanceStarting ||
              isInstanceStopping;
            const isTerminalLaunchInstance = usesTerminalLaunch(instance);
            const launchMode = resolveInstanceLaunchMode(instance);
            const statusClass = restartingAll
              ? "restarting"
              : isInstanceStarting
                ? "starting"
                : instance.running
                  ? "running"
                  : isTerminalLaunchInstance && instance.lastLaunchedAt
                    ? "ready"
                    : "stopped";
            const statusLabel = restartingAll
              ? t("instances.status.restarting", "重启中")
              : isInstanceStarting
                ? t("instances.status.starting", "启动中")
                : instance.running
                  ? t("instances.status.running", "运行中")
                  : isTerminalLaunchInstance && instance.lastLaunchedAt
                    ? t("instances.status.ready", "已准备")
                    : t("instances.status.stopped", "未运行");
            const canShowFloatingCard =
              Boolean(instance.bindAccountId) && !accountMissing;
            const floatingCardActionTitle = canShowFloatingCard
              ? t("instances.actions.showFloatingCard", "显示悬浮框")
              : accountMissing
                ? t(
                    "instances.actions.showFloatingCardMissing",
                    "绑定账号不存在，无法显示悬浮框",
                  )
                : t(
                    "instances.actions.showFloatingCardDisabled",
                    "请先绑定账号后再显示悬浮框",
                  );
            return (
              <div
                className={`instance-item ${openInlineMenuId === instance.id ? "dropdown-open" : ""}`}
                key={instance.id}
              >
                <div className="instance-select">
                  {/* Future: checkbox for bulk selection */}
                </div>
                <div className="instance-main-info">
                  <div className="instance-title-row">
                    <span className="instance-name">
                      {instance.isDefault
                        ? t("instances.defaultName", "默认实例")
                        : instance.name}
                    </span>
                    {isCodexApp && (
                      <span
                        className={`instance-launch-mode-badge ${launchMode}`}
                      >
                        {launchMode === "cli"
                          ? t("instances.form.launchModeCli", "CLI")
                          : t("instances.form.launchModeApp", "桌面版")}
                      </span>
                    )}
                  </div>
                  {instance.extraArgs?.trim() && (
                    <div className="instance-sub-info">
                      <span className="info-item" title={instance.extraArgs}>
                        <Terminal size={12} />
                        {t("instances.labels.argsPresent", "有参数")}
                      </span>
                    </div>
                  )}
                </div>

                <div className="instance-status-cell">
                  <span className={`instance-status ${statusClass}`}>
                    {statusLabel}
                  </span>
                </div>

                <div className="instance-account">
                  {accountDisabledByInit ? (
                    <button
                      type="button"
                      className="instance-account-disabled"
                      onClick={() => setInitGuideInstance(instance)}
                    >
                      {t(
                        "instances.labels.pendingInit",
                        "待初始化（先启动一次）",
                      )}
                    </button>
                  ) : (
                    <InlineAccountSelect
                      value={instance.bindAccountId || null}
                      onChange={(nextId) =>
                        handleInlineBindChange(instance, nextId)
                      }
                      disabled={isInstanceBusy}
                      missing={accountMissing}
                      placeholder={t("instances.labels.unbound", "未绑定")}
                      instanceId={instance.id}
                      currentOpenId={openInlineMenuId}
                      onOpenChange={(open) => {
                        setOpenInlineMenuId(open ? instance.id : null);
                      }}
                    />
                  )}
                </div>

                <div className="instance-pid">
                  {instance.running ? (
                    <span className="pid-value">{instance.lastPid ?? "-"}</span>
                  ) : null}
                </div>

                <div className="instance-actions">
                  <button
                    className="icon-button"
                    title={floatingCardActionTitle}
                    onClick={() => void handleShowFloatingCard(instance)}
                    disabled={
                      !canShowFloatingCard ||
                      isInstanceBusy ||
                      restartingAll ||
                      bulkActionLoading
                    }
                  >
                    <Eye size={16} />
                  </button>
                  <button
                    className="icon-button"
                    title={t("instances.actions.start", "启动")}
                    onClick={() => handleStart(instance)}
                    disabled={
                      isInstanceBusy || restartingAll || bulkActionLoading
                    }
                  >
                    <Play size={16} />
                  </button>
                  {!isTerminalLaunchInstance && (
                    <button
                      className="icon-button"
                      title={t("instances.actions.openWindow", "定位窗口")}
                      onClick={() => handleLocateInstance(instance)}
                      disabled={
                        !instance.running ||
                        isInstanceBusy ||
                        restartingAll ||
                        bulkActionLoading
                      }
                    >
                      <ExternalLink size={16} />
                    </button>
                  )}
                  {!isTerminalLaunchInstance && (
                    <button
                      className="icon-button danger"
                      title={t("instances.actions.stop", "停止")}
                      onClick={() => handleStop(instance)}
                      disabled={
                        !instance.running ||
                        isInstanceBusy ||
                        restartingAll ||
                        bulkActionLoading
                      }
                    >
                      <Square size={16} />
                    </button>
                  )}
                  <button
                    className="icon-button"
                    title={t("instances.actions.edit", "编辑")}
                    onClick={() => openEditModal(instance)}
                    disabled={
                      isInstanceBusy || restartingAll || bulkActionLoading
                    }
                  >
                    <Pencil size={16} />
                  </button>
                  <button
                    className="icon-button danger"
                    title={t("common.delete", "删除")}
                    onClick={() => handleDelete(instance)}
                    disabled={
                      instance.isDefault ||
                      isInstanceBusy ||
                      restartingAll ||
                      bulkActionLoading
                    }
                  >
                    <Trash2 size={16} />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {initGuideInstance && (
        <div
          className="modal-overlay"
          onClick={() => setInitGuideInstance(null)}
        >
          <div
            className="modal instance-init-guide-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("instances.initGuide.title", "实例尚未初始化")}</h2>
              <button
                className="modal-close"
                onClick={() => setInitGuideInstance(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">
                {t(
                  "instances.initGuide.desc",
                  "该实例为“空白实例”，当前仅创建了目录，尚未生成实例数据。",
                )}
              </p>
              <div className="instance-init-guide-box">
                {t(
                  "instances.initGuide.tip",
                  "请先启动一次实例，初始化完成后即可绑定账号。",
                )}
              </div>
              <div className="form-group">
                <label>
                  {t("instances.runningDialog.pathLabel", "实例目录")}
                </label>
                <input
                  className="form-input"
                  value={initGuideInstance.userDataDir}
                  disabled
                />
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setInitGuideInstance(null)}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-primary"
                onClick={handleInitGuideStart}
                disabled={
                  actionLoading === initGuideInstance.id ||
                  startingInstanceIdSet.has(initGuideInstance.id)
                }
              >
                {t("instances.initGuide.startNow", "立即启动")}
              </button>
            </div>
          </div>
        </div>
      )}

      {deleteConfirmInstance && (
        <div
          className="modal-overlay"
          onClick={() => setDeleteConfirmInstance(null)}
        >
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("instances.delete.title", "删除实例")}</h2>
              <button
                className="modal-close"
                onClick={() => setDeleteConfirmInstance(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">
                {t(
                  "instances.delete.message",
                  "确认删除实例 {{name}}？将移除配置并删除实例目录。",
                  {
                    name: deleteConfirmInstance.name,
                  },
                )}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setDeleteConfirmInstance(null)}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-danger"
                onClick={handleConfirmDelete}
                disabled={actionLoading === deleteConfirmInstance.id}
              >
                {t("common.delete", "删除")}
              </button>
            </div>
          </div>
        </div>
      )}

      {runningNoticeInstance && (
        <div
          className="modal-overlay"
          onClick={() => setRunningNoticeInstance(null)}
        >
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("instances.runningDialog.title", "实例已在运行")}</h2>
              <button
                className="modal-close"
                onClick={() => setRunningNoticeInstance(null)}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p className="form-hint">
                {t(
                  "instances.runningDialog.desc",
                  "实例已在运行中，可立马前往或关闭后重启。",
                )}
              </p>
              <div className="form-group">
                <label>
                  {t("instances.runningDialog.pathLabel", "实例目录")}
                </label>
                <input
                  className="form-input"
                  value={runningNoticeInstance.userDataDir}
                  disabled
                />
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={handleOpenRunningInstance}
              >
                {t("instances.runningDialog.go", "立马前往")}
              </button>
              <button className="btn btn-danger" onClick={handleForceRestart}>
                {t("instances.runningDialog.restart", "关闭并重启")}
              </button>
            </div>
          </div>
        </div>
      )}

      {showModal && (
        <div className="modal-overlay" onClick={closeModal}>
          <div className="modal modal-lg" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>
                {editing
                  ? t("instances.modal.editTitle", "编辑实例")
                  : t("instances.modal.createTitle", "新建实例")}
              </h2>
              <button
                className="modal-close"
                onClick={closeModal}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>{t("instances.form.name", "实例名称")}</label>
                <input
                  className="form-input"
                  value={formName}
                  onChange={(e) => handleNameChange(e.target.value)}
                  placeholder={t(
                    "instances.form.namePlaceholder",
                    "例如：工作账号",
                  )}
                  disabled={Boolean(editing?.isDefault)}
                />
              </div>

              {!editing && (
                <div className="form-group">
                  <label>{t("instances.form.initMode", "初始化方式")}</label>
                  <div className="instance-init-mode-group">
                    <label
                      className={`instance-init-mode-option ${formInitMode === "copy" ? "active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="instance-init-mode"
                        checked={formInitMode === "copy"}
                        onChange={() => setFormInitMode("copy")}
                      />
                      <span>
                        {t(
                          "instances.form.initModeCopy",
                          "复制来源实例（默认）",
                        )}
                      </span>
                    </label>
                    <label
                      className={`instance-init-mode-option ${formInitMode === "empty" ? "active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="instance-init-mode"
                        checked={formInitMode === "empty"}
                        onChange={() => setFormInitMode("empty")}
                      />
                      <span>
                        {t(
                          "instances.form.initModeEmpty",
                          "空白实例（不复制）",
                        )}
                      </span>
                    </label>
                    <label
                      className={`instance-init-mode-option ${formInitMode === "existingDir" ? "active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="instance-init-mode"
                        checked={formInitMode === "existingDir"}
                        onChange={() => {
                          setFormInitMode("existingDir");
                          setFormPath("");
                        }}
                      />
                      <span>
                        {t(
                          "instances.form.initModeExistingDir",
                          "使用已存在目录",
                        )}
                      </span>
                    </label>
                  </div>
                  {formInitMode === "empty" && (
                    <div className="instance-init-note">
                      {t(
                        "instances.form.emptyInitHint",
                        "选择无需复制实例，只会创建空白目录。需要启动一次后，才可以进行账号绑定。",
                      )}
                    </div>
                  )}
                </div>
              )}

              {!hidePathFieldInEditModal && (
                <div className="form-group">
                  <label>{t("instances.form.path", "实例目录")}</label>
                  <div className="instance-path-row">
                    <input
                      className="form-input"
                      value={formPath}
                      onChange={(e) => setFormPath(e.target.value)}
                      placeholder={t(
                        "instances.form.pathPlaceholder",
                        "选择实例目录",
                      )}
                      disabled={Boolean(editing)}
                    />
                    {!editing && (
                      <button
                        className="btn btn-secondary"
                        onClick={handleSelectPath}
                      >
                        <FolderOpen size={16} />
                        {t("instances.actions.selectPath", "选择目录")}
                      </button>
                    )}
                  </div>
                  {!editing && formInitMode !== "existingDir" && (
                    <p className="form-hint">
                      {t(
                        "instances.form.pathAutoHint",
                        "修改名称时自动更新路径，也可手动选择",
                      )}
                    </p>
                  )}
                  {editing && (
                    <p className="form-hint">
                      {t("instances.form.pathReadOnly", "编辑时不可修改路径")}
                    </p>
                  )}
                </div>
              )}

              {supportsLaunchModeSelect && (
                <div className="form-group">
                  <label>{t("instances.form.launchMode", "启动方式")}</label>
                  <div className="instance-init-mode-group">
                    <label
                      className={`instance-init-mode-option ${formLaunchMode === "app" ? "active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="instance-launch-mode"
                        checked={formLaunchMode === "app"}
                        onChange={() => setFormLaunchMode("app")}
                      />
                      <span>{t("instances.form.launchModeApp", "桌面版")}</span>
                    </label>
                    <label
                      className={`instance-init-mode-option ${formLaunchMode === "cli" ? "active" : ""}`}
                    >
                      <input
                        type="radio"
                        name="instance-launch-mode"
                        checked={formLaunchMode === "cli"}
                        onChange={() => setFormLaunchMode("cli")}
                      />
                      <span>{t("instances.form.launchModeCli", "CLI")}</span>
                    </label>
                  </div>
                </div>
              )}

              {showWorkingDirField && (
                <div className="form-group">
                  <label>{t("instances.form.workingDir", "工作目录")}</label>
                  <div className="instance-path-row">
                    <input
                      className="form-input"
                      value={formWorkingDir}
                      onChange={(e) => setFormWorkingDir(e.target.value)}
                      placeholder={t(
                        "instances.form.workingDirPlaceholder",
                        "默认当前路径",
                      )}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={handleSelectWorkingDir}
                    >
                      <FolderOpen size={16} />
                      {t("instances.actions.selectPath", "选择目录")}
                    </button>
                  </div>
                  <p className="form-hint">
                    {t(
                      "instances.form.workingDirDesc",
                      "启动时将首先切换到此目录",
                    )}
                  </p>
                </div>
              )}

              {!editing && formInitMode === "copy" && (
                <div className="form-group">
                  <label>
                    {t("instances.form.copySource", "复制来源实例")}
                  </label>
                  <InstanceSelect
                    value={formCopySourceInstanceId}
                    onChange={setFormCopySourceInstanceId}
                  />
                  <p className="form-hint">
                    {t(
                      "instances.form.copySourceDesc",
                      "从指定实例复制配置与登录信息",
                    )}
                  </p>
                  {selectedCopySourceInstance?.running && (
                    <p className="form-hint warning">
                      {t(
                        "instances.form.copySourceRunningHint",
                        "该实例正在运行，建议先关闭以避免数据不一致",
                      )}
                    </p>
                  )}
                </div>
              )}

              {!editing ? (
                <div className="form-group">
                  <label>
                    {t("instances.form.bindInject", "绑定账号")}
                    {formInitMode === "existingDir"
                      ? `（${t("instances.form.optional", "可选")}）`
                      : ""}
                  </label>
                  {formInitMode === "empty" ? (
                    <>
                      <FormAccountSelect
                        value={null}
                        onChange={() => {}}
                        disabled
                        placeholder={t(
                          "instances.form.bindAfterInit",
                          "初始化后可绑定",
                        )}
                      />
                      <p className="form-hint">
                        {t(
                          "instances.form.bindDisabledHint",
                          "空白实例需先启动一次生成实例数据后，才可绑定账号。",
                        )}
                      </p>
                    </>
                  ) : (
                    <FormAccountSelect
                      value={formBindAccountId || null}
                      onChange={handleFormAccountChange}
                    />
                  )}
                </div>
              ) : (
                <div className="form-group">
                  <label>{t("instances.form.bindAccount", "绑定账号")}</label>
                  {editing?.initialized === false && !editing.isDefault ? (
                    <>
                      <FormAccountSelect
                        value={null}
                        onChange={() => {}}
                        disabled
                        placeholder={t(
                          "instances.form.bindAfterInit",
                          "初始化后可绑定",
                        )}
                      />
                      <p className="form-hint">
                        {t(
                          "instances.form.bindDisabledHint",
                          "空白实例需先启动一次生成实例数据后，才可绑定账号。",
                        )}
                      </p>
                    </>
                  ) : (
                    <FormAccountSelect
                      value={formBindAccountId || null}
                      onChange={handleFormAccountChange}
                      missing={Boolean(
                        formBindAccountId &&
                        !accounts.find((item) => item.id === formBindAccountId),
                      )}
                    />
                  )}
                </div>
              )}

              <div className="form-group">
                <label>{t("instances.form.extraArgs", "自定义启动参数")}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={formExtraArgs}
                  onChange={(e) => setFormExtraArgs(e.target.value)}
                  placeholder={t(
                    "instances.form.extraArgsPlaceholder",
                    "例如：--disable-gpu --log-level=2",
                  )}
                />
                <p className="form-hint">
                  {t(
                    "instances.form.extraArgsDesc",
                    "按空格分隔参数，支持引号包裹",
                  )}
                </p>
              </div>
              {formError && (
                <div className="form-error" ref={formErrorRef}>
                  {formError}
                </div>
              )}
            </div>

            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeModal}>
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-primary"
                onClick={handleSubmit}
                disabled={
                  actionLoading === "create" ||
                  (editing ? actionLoading === editing.id : false)
                }
              >
                {editing
                  ? t("common.save", "保存")
                  : t("instances.actions.create", "新建实例")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
