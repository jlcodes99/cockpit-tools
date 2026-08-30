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
  Copy,
  Trash2,
  Terminal,
  FolderOpen,
  Square,
  ChevronDown,
  ChevronLeft,
  X,
  Search,
  ArrowDownWideNarrow,
  RefreshCw,
  ExternalLink,
  Eye,
  EyeOff,
} from "lucide-react";
import { confirm as confirmDialog, open } from "@tauri-apps/plugin-dialog";
import md5 from "blueimp-md5";
import {
  CODEX_API_SERVICE_BIND_ID,
  CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
  buildCodexProviderGatewayBindId,
  CodexInstanceApiRoute,
  CodexInstanceModelRouting,
  InstanceInitMode,
  InstanceLaunchMode,
  InstanceProfile,
} from "../types/instance";
import type { PlatformId } from "../types/platform";
import type {
  CodexExperimentalModelDefinition,
  CodexQuickConfig,
} from "../types/codex";
import {
  FileCorruptedModal,
  parseFileCorruptedError,
  type FileCorruptedError,
} from "./FileCorruptedModal";
import { ModalErrorMessage, useModalErrorState } from "./ModalErrorMessage";
import { scrollElementIntoView } from "../utils/reducedMotion";
import { useEscClose } from "../hooks/useEscClose";
import { useEnterConfirm } from "../hooks/useEnterConfirm";
import { CodexExperimentalModelEditor } from "./codex/CodexExperimentalModelEditor";
import {
  CodexModelRoutingFields,
  CODEX_MODEL_ROUTE_NAMESPACE_PATTERN,
  CODEX_RESERVED_MODEL_ROUTE_NAMESPACES,
  collectRouteUpstreamModels,
  eligibleCodexModelRoutingAccounts,
  createCodexModelSourceResolver,
  shortCodexRouteAccountLabel,
  syncExperimentalModelsWithRouting,
  toggleRouteModelInRoutes,
} from "./codex/CodexModelRoutingFields";
import type { InstanceStoreState } from "../stores/createInstanceStore";
import { showInstanceFloatingCardWindow } from "../services/floatingCardService";
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled,
} from "../utils/privacy";
import {
  getCodexInstanceQuickConfig,
  openCodexInstanceConfigToml,
  saveCodexInstanceConfiguration,
  saveCodexInstanceQuickConfig,
} from "../services/codexInstanceService";
import { CodexSpeedSelect } from "./codex/CodexSpeedSelect";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import type { CodexAppSpeed } from "../types/codex";
import { getCodexExperimentalModelErrorMessage } from "../utils/codexExperimentalModel";
import { isCodexInstanceAccountConflict } from "../utils/codexInstanceLaunchConflict";
import { presentWindowsOperationError } from "../utils/windowsOperationDialog";

type MessageState = { text: string; tone?: "error" };
type AccountLike = {
  id: string;
  email: string;
  tags?: string[] | null;
  auth_mode?: string;
  openai_api_key?: string | null;
  api_wire_api?: string | null;
  api_base_url?: string | null;
  api_provider_mode?: string | null;
  api_provider_id?: string | null;
  api_provider_name?: string | null;
  api_model_catalog?: string[] | null;
};
type InstanceSortField = "createdAt" | "lastLaunchedAt";
type SortDirection = "asc" | "desc";
type StartInstanceOutcome =
  "started" | "already-running" | "missing-path" | "failed" | "cancelled";
type AccountSelectPortalPosition = {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: "top" | "bottom";
};

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

type AccountMenuItemsRenderArgs<TAccount extends AccountLike> = {
  visibleAccounts: TAccount[];
  availableTags: string[];
  searchValue: string;
  onSearchChange: (value: string) => void;
  tagFilter: string[];
  onToggleTagFilter: (tag: string) => void;
  onClearTagFilter: () => void;
  value: string | null;
  isFollowingCurrent?: boolean;
  allowFollowCurrent?: boolean;
  allowUnbound?: boolean;
  onFollowCurrent?: () => void;
  onChange: (nextId: string | null) => void;
  onClose: () => void;
  selectedAccount: TAccount | null;
};

type InlineAccountSelectProps<TAccount extends AccountLike> =
  BaseAccountSelectProps & {
    accounts: TAccount[];
    launchMode: InstanceLaunchMode;
    filterAccountsForLaunchMode: (
      source: TAccount[],
      launchMode: InstanceLaunchMode,
    ) => TAccount[];
    getAccountSearchText?: (account: TAccount) => string;
    resolveAccountDisplayText: (account?: TAccount | null) => string;
    isApiServiceBindId: (value?: string | null) => boolean;
    resolveBoundAccount: (bindAccountId?: string | null) => {
      account: TAccount | null;
    };
    renderAccountQuotaPreview: (account: TAccount) => ReactNode;
    renderAccountBadge?: (account: TAccount) => ReactNode;
    maskAccountText: (value?: string | null) => string;
    resolveApiServiceLabel: () => string;
    renderAccountMenuItems: (
      args: AccountMenuItemsRenderArgs<TAccount>,
    ) => ReactNode;
    unboundLabel: string;
    selectAccountLabel: string;
    missingAccountLabel: string;
    followCurrentLabel: string;
    onOpenChange?: (open: boolean) => void;
    instanceId?: string;
    currentOpenId?: string | null;
  };

interface InstancesManagerProps<TAccount extends AccountLike> {
  instanceStore: InstanceStoreState;
  accounts: TAccount[];
  fetchAccounts: () => Promise<void>;
  renderAccountQuotaPreview: (account: TAccount) => ReactNode;
  renderAccountBadge?: (account: TAccount) => ReactNode;
  getAccountDisplayText?: (account: TAccount) => string;
  getAccountSearchText?: (account: TAccount) => string;
  appType?:
    | "antigravity"
    | "antigravity_ide"
    | "codex"
    | "claude"
    | "vscode"
    | "windsurf"
    | "kiro"
    | "cursor"
    | "grok"
    | "codebuddy"
    | "codebuddy_cn"
    | "qoder"
    | "trae"
    | "trae_solo"
    | "trae_cn"
    | "trae_solo_cn"
    | "workbuddy"
    | "zcode";
  onInstanceStarted?: (instance: InstanceProfile) => void | Promise<void>;
  onBeforeStart?: (instance: InstanceProfile) => boolean | Promise<boolean>;
  onInstanceStartError?: (
    error: unknown,
    instance: InstanceProfile,
  ) => boolean | Promise<boolean>;
  resolveStartSuccessMessage?: (instance: InstanceProfile) => string;
  isAccountAllowedForLaunchMode?: (
    account: TAccount,
    launchMode: InstanceLaunchMode,
  ) => boolean;
  toolbarExtraActions?: ReactNode;
}

const INSTANCE_AUTO_REFRESH_INTERVAL_MS = 10_000;
const ACCOUNT_SELECT_PORTAL_GAP = 8;
const ACCOUNT_SELECT_PORTAL_SAFE_MARGIN = 12;
const ACCOUNT_SELECT_PORTAL_MAX_HEIGHT = 320;
const ACCOUNT_SELECT_PORTAL_MIN_HEIGHT = 140;
const ACCOUNT_SELECT_PORTAL_Z_INDEX = 10020;

const normalizeInstanceAccountTag = (tag: string) => tag.trim().toLowerCase();

const collectInstanceAccountTags = <TAccount extends AccountLike>(
  accounts: TAccount[],
): string[] => {
  const values = new Set<string>();
  accounts.forEach((account) => {
    (account.tags || []).forEach((tag) => {
      const normalized = normalizeInstanceAccountTag(tag);
      if (normalized) {
        values.add(normalized);
      }
    });
  });
  return Array.from(values).sort((left, right) => left.localeCompare(right));
};

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
    rect.top - ACCOUNT_SELECT_PORTAL_GAP - ACCOUNT_SELECT_PORTAL_SAFE_MARGIN;
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

const isSameAccountSelectPortalPosition = (
  left: AccountSelectPortalPosition | null,
  right: AccountSelectPortalPosition | null,
) =>
  left?.top === right?.top &&
  left?.left === right?.left &&
  left?.width === right?.width &&
  left?.maxHeight === right?.maxHeight &&
  left?.placement === right?.placement;

const InlineAccountSelect = <TAccount extends AccountLike>({
  value,
  onChange,
  accounts,
  launchMode,
  filterAccountsForLaunchMode,
  getAccountSearchText,
  resolveAccountDisplayText,
  isApiServiceBindId,
  resolveBoundAccount,
  renderAccountQuotaPreview,
  renderAccountBadge,
  maskAccountText,
  resolveApiServiceLabel,
  renderAccountMenuItems,
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
  unboundLabel,
  selectAccountLabel,
  missingAccountLabel,
  followCurrentLabel,
}: InlineAccountSelectProps<TAccount>) => {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const portalMenuRef = useRef<HTMLDivElement | null>(null);
  const activeItemScrolledRef = useRef(false);
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = Boolean(instanceId);
  const isOpen = isControlled ? currentOpenId === instanceId : internalOpen;
  const setOpen = useCallback(
    (nextOpen: boolean) => {
      if (!isControlled) {
        setInternalOpen(nextOpen);
      }
      onOpenChange?.(nextOpen);
    },
    [isControlled, onOpenChange],
  );
  const [portalPos, setPortalPos] =
    useState<AccountSelectPortalPosition | null>(null);
  const [searchValue, setSearchValue] = useState("");
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const selectableAccounts = useMemo(
    () => filterAccountsForLaunchMode(accounts, launchMode),
    [accounts, filterAccountsForLaunchMode, launchMode],
  );

  const availableTags = useMemo(
    () => collectInstanceAccountTags(selectableAccounts),
    [selectableAccounts],
  );
  const visibleAccounts = useMemo(() => {
    const normalizedQuery = searchValue.trim().toLowerCase();
    const selectedTags = new Set(tagFilter.map(normalizeInstanceAccountTag));
    return selectableAccounts.filter((account) => {
      if (selectedTags.size > 0) {
        const accountTags = (account.tags || [])
          .map(normalizeInstanceAccountTag)
          .filter(Boolean);
        if (!accountTags.some((tag) => selectedTags.has(tag))) {
          return false;
        }
      }
      if (!normalizedQuery) return true;
      const haystack = [
        resolveAccountDisplayText(account),
        account.email,
        getAccountSearchText ? getAccountSearchText(account) : "",
        ...(account.tags || []),
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(normalizedQuery);
    });
  }, [
    getAccountSearchText,
    resolveAccountDisplayText,
    searchValue,
    selectableAccounts,
    tagFilter,
  ]);

  const toggleTagFilter = useCallback((tag: string) => {
    setTagFilter((prev) =>
      prev.includes(tag) ? prev.filter((item) => item !== tag) : [...prev, tag],
    );
  }, []);

  const updatePortalPos = useCallback((event?: Event) => {
    const eventTarget = event?.target;
    if (
      event?.type === "scroll" &&
      eventTarget instanceof Node &&
      portalMenuRef.current?.contains(eventTarget)
    ) {
      return;
    }
    setPortalPos((prev) => {
      const next = resolveAccountSelectPortalPosition(triggerRef.current);
      return isSameAccountSelectPortalPosition(prev, next) ? prev : next;
    });
  }, []);

  useEffect(() => {
    if (isOpen) return;
    activeItemScrolledRef.current = false;
    setSearchValue("");
    setTagFilter([]);
  }, [isOpen]);

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
        setOpen(false);
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
  }, [isOpen, setOpen, updatePortalPos]);

  useEffect(() => {
    if (!isOpen || !portalPos || !portalMenuRef.current) return;
    if (activeItemScrolledRef.current) return;
    activeItemScrolledRef.current = true;

    const frameId = window.requestAnimationFrame(() => {
      const activeItem = portalMenuRef.current?.querySelector<HTMLElement>(
        '[data-account-select-active="true"]',
      );
      activeItem?.scrollIntoView({
        block: "nearest",
        behavior: "auto",
      });
    });

    return () => {
      window.cancelAnimationFrame(frameId);
    };
  }, [isOpen, portalPos?.placement]);

  useEffect(() => {
    if (disabled && isOpen) {
      setOpen(false);
    }
  }, [disabled, isOpen, setOpen]);

  const isApiServiceSelected = isApiServiceBindId(value);
  const selectedAccount = resolveBoundAccount(value).account;
  const basePlaceholder =
    placeholder || (allowUnbound ? unboundLabel : selectAccountLabel);
  const selectedLabel = missing
    ? missingAccountLabel
    : isFollowingCurrent
      ? maskAccountText(resolveAccountDisplayText(selectedAccount)) ||
        followCurrentLabel
      : isApiServiceSelected
        ? resolveApiServiceLabel()
        : maskAccountText(resolveAccountDisplayText(selectedAccount)) ||
          basePlaceholder;
  const selectedBadge =
    !missing && selectedAccount ? renderAccountBadge?.(selectedAccount) : null;
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
          setOpen(!isOpen);
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
                  visibleAccounts,
                  availableTags,
                  searchValue,
                  onSearchChange: setSearchValue,
                  tagFilter,
                  onToggleTagFilter: toggleTagFilter,
                  onClearTagFilter: () => setTagFilter([]),
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
    case "claude":
      return "claude_manager";
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
  getAccountDisplayText,
  getAccountSearchText,
  appType = "antigravity",
  onInstanceStarted,
  onBeforeStart,
  onInstanceStartError,
  resolveStartSuccessMessage,
  isAccountAllowedForLaunchMode,
  toolbarExtraActions,
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
  const {
    message: deleteInstanceError,
    scrollKey: deleteInstanceErrorScrollKey,
    report: reportDeleteInstanceError,
    clear: clearDeleteInstanceError,
  } = useModalErrorState();
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
  const [formAppSpeed, setFormAppSpeed] = useState<CodexAppSpeed>("standard");
  const [formBindAccountId, setFormBindAccountId] = useState<string>("");
  const [formModelRoutingEnabled, setFormModelRoutingEnabled] = useState(false);
  const [formModelRoutes, setFormModelRoutes] = useState<
    CodexInstanceApiRoute[]
  >([]);
  const [formCodexQuickConfig, setFormCodexQuickConfig] =
    useState<CodexQuickConfig | null>(null);
  const [
    formExperimentalModelCatalogEnabled,
    setFormExperimentalModelCatalogEnabled,
  ] = useState(false);
  const [formExperimentalModels, setFormExperimentalModels] = useState<
    CodexExperimentalModelDefinition[]
  >([]);
  const [formExperimentalDefaultModelId, setFormExperimentalDefaultModelId] =
    useState<string | null>(null);
  const [formExperimentalModelsError, setFormExperimentalModelsError] =
    useState<string | null>(null);
  const [formCodexQuickConfigLoading, setFormCodexQuickConfigLoading] =
    useState(false);
  const [formCodexQuickConfigError, setFormCodexQuickConfigError] = useState<
    string | null
  >(null);
  const [formCodexOpenConfigLoading, setFormCodexOpenConfigLoading] =
    useState(false);
  const [formCopySourceInstanceId, setFormCopySourceInstanceId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const formErrorRef = useRef<HTMLDivElement | null>(null);
  const copyAutoNameRef = useRef("");
  const lastAppliedCopySourceRef = useRef("");
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
  const isGrokApp = appType === "grok";
  const supportsInstanceInitialization = !isGrokApp;
  const isCodexApp = appType === "codex";
  const isClaudeApp = appType === "claude";
  const isCliOnlyApp = isGrokApp;
  const supportsLaunchModeSelect = isCodexApp || isClaudeApp;
  const modelRoutingProviderAccounts = useMemo(
    () => (isCodexApp ? eligibleCodexModelRoutingAccounts(accounts) : []),
    [accounts, isCodexApp],
  );
  const resolveFormModelSource = useMemo(
    () =>
      createCodexModelSourceResolver(
        formModelRoutes,
        accounts,
        t,
        getAccountDisplayText
          ? (account) => getAccountDisplayText(account as TAccount)
          : undefined,
      ),
    [accounts, formModelRoutes, getAccountDisplayText, t],
  );
  const formAvailableChannels = useMemo(() => {
    if (!isCodexApp) return [];
    const providerAccounts = eligibleCodexModelRoutingAccounts(accounts);
    return formModelRoutes
      .filter((route) => route.enabled)
      .map((route) => {
        const provider = providerAccounts.find(
          (acc) => acc.id === route.providerAccountId,
        );
        const upstreamModels = collectRouteUpstreamModels(route, provider);
        const name = shortCodexRouteAccountLabel(
          provider,
          getAccountDisplayText?.(provider as TAccount) || provider?.email,
        );
        return {
          id: route.id,
          namespace: route.namespace,
          providerName: name,
          models: route.selectedModels ?? upstreamModels,
        };
      })
      .filter((group) => group.models.length > 0);
  }, [accounts, formModelRoutes, getAccountDisplayText, isCodexApp]);
  const resolveInstanceLaunchMode = (
    instance?: InstanceProfile | null,
  ): InstanceLaunchMode => {
    if (isCliOnlyApp) {
      return "cli";
    }
    if (isCodexApp || isClaudeApp) {
      return instance?.launchMode ?? "app";
    }
    return "app";
  };
  const usesTerminalLaunch = (instance: InstanceProfile) =>
    isCliOnlyApp ||
    ((isCodexApp || isClaudeApp) &&
      resolveInstanceLaunchMode(instance) === "cli");
  const supportsStopControl = instances.some(
    (item) => !usesTerminalLaunch(item),
  );
  const hidePathFieldInEditModal = isCliOnlyApp && Boolean(editing?.isDefault);
  const showWorkingDirField =
    isCliOnlyApp || (supportsLaunchModeSelect && formLaunchMode === "cli");
  const floatingCardPlatformId = useMemo(
    () => resolveFloatingCardPlatformId(appType),
    [appType],
  );
  const resolveApiServiceLabel = useCallback(
    () => t("codex.localAccess.title", "API 服务"),
    [t],
  );
  const isApiServiceBindId = useCallback(
    (value?: string | null) =>
      isCodexApp && value === CODEX_API_SERVICE_BIND_ID,
    [isCodexApp],
  );
  const parseProviderGatewayBindAccountId = useCallback(
    (value?: string | null) => {
      if (!isCodexApp) return null;
      const trimmed = value?.trim() || "";
      if (!trimmed.startsWith(CODEX_PROVIDER_GATEWAY_BIND_PREFIX)) return null;
      const accountId = trimmed
        .slice(CODEX_PROVIDER_GATEWAY_BIND_PREFIX.length)
        .trim();
      return accountId || null;
    },
    [isCodexApp],
  );
  const shouldBindAccountViaProviderGateway = useCallback(
    (account?: TAccount | null) =>
      isCodexApp &&
      account?.auth_mode === "apikey" &&
      account.api_wire_api === "chat_completions",
    [isCodexApp],
  );
  const resolveBindAccountValue = useCallback(
    (accountId?: string | null) => {
      if (!accountId) return null;
      if (isApiServiceBindId(accountId)) return accountId;
      if (parseProviderGatewayBindAccountId(accountId)) return accountId;
      const account = accounts.find((item) => item.id === accountId) || null;
      if (account && shouldBindAccountViaProviderGateway(account)) {
        return buildCodexProviderGatewayBindId(account.id);
      }
      return accountId;
    },
    [
      accounts,
      isApiServiceBindId,
      parseProviderGatewayBindAccountId,
      shouldBindAccountViaProviderGateway,
    ],
  );
  const resolveBoundAccount = useCallback(
    (bindAccountId?: string | null) => {
      if (!bindAccountId) {
        return {
          account: null,
          accountId: null,
          missing: false,
          isApiService: false,
          isProviderGateway: false,
        };
      }
      if (isApiServiceBindId(bindAccountId)) {
        return {
          account: null,
          accountId: null,
          missing: false,
          isApiService: true,
          isProviderGateway: false,
        };
      }
      const providerGatewayAccountId =
        parseProviderGatewayBindAccountId(bindAccountId);
      const targetAccountId = providerGatewayAccountId || bindAccountId;
      const account =
        accounts.find((item) => item.id === targetAccountId) || null;
      return {
        account,
        accountId: targetAccountId,
        missing: !account,
        isApiService: false,
        isProviderGateway: Boolean(providerGatewayAccountId),
      };
    },
    [accounts, isApiServiceBindId, parseProviderGatewayBindAccountId],
  );
  const filterAccountsForLaunchMode = useCallback(
    (source: TAccount[], launchMode: InstanceLaunchMode) =>
      isAccountAllowedForLaunchMode
        ? source.filter((account) =>
            isAccountAllowedForLaunchMode(account, launchMode),
          )
        : source,
    [isAccountAllowedForLaunchMode],
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
  const resolveAccountDisplayText = useCallback(
    (account?: TAccount | null) => {
      if (!account) return "";
      const value = getAccountDisplayText?.(account) ?? account.email;
      return value.trim() || account.email;
    },
    [getAccountDisplayText],
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
      if (openInlineMenuId || showModal) return;
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
  }, [fetchAccounts, openInlineMenuId, refreshInstances, showModal]);

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
    scrollElementIntoView(formErrorRef.current, { block: "end" });
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
      const { account, isApiService } = resolveBoundAccount(
        instance.bindAccountId,
      );
      const accountText = isApiService
        ? resolveApiServiceLabel()
        : account
          ? getAccountSearchText
            ? getAccountSearchText(account)
            : resolveAccountDisplayText(account)
          : "";
      const haystack = [displayName, accountText, instance.userDataDir || ""]
        .join(" ")
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [
    getAccountSearchText,
    resolveApiServiceLabel,
    resolveBoundAccount,
    resolveAccountDisplayText,
    searchQuery,
    sortedInstances,
    t,
  ]);

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
    setFormInitMode(isGrokApp ? "empty" : "copy");
    setFormLaunchMode(isCliOnlyApp ? "cli" : "app");
    setFormAppSpeed("standard");
    setFormBindAccountId("");
    setFormModelRoutingEnabled(false);
    setFormModelRoutes([]);
    setFormCodexQuickConfig(null);
    setFormExperimentalModelCatalogEnabled(false);
    setFormExperimentalModels([]);
    setFormExperimentalModelsError(null);
    setFormCodexQuickConfigLoading(false);
    setFormCodexQuickConfigError(null);
    setFormCodexOpenConfigLoading(false);
    setFormCopySourceInstanceId(defaultInstanceId);
    setFormError(null);
    setPathAuto(true);
  };

  const openCreateModal = () => {
    copyAutoNameRef.current = "";
    lastAppliedCopySourceRef.current = "";
    setOpenInlineMenuId(null);
    resetForm(true);
    setEditing(null);
    setShowModal(true);
  };

  const openDuplicateModal = (instance: InstanceProfile) => {
    copyAutoNameRef.current = "";
    lastAppliedCopySourceRef.current = "";
    setOpenInlineMenuId(null);
    resetForm(true);
    setEditing(null);
    setFormInitMode("copy");
    setFormLaunchMode(resolveInstanceLaunchMode(instance));
    setFormCopySourceInstanceId(instance.id);
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

  useEffect(() => {
    if (!isAccountAllowedForLaunchMode || !formBindAccountId) return;
    const selected = resolveBoundAccount(formBindAccountId).account;
    if (!selected) return;
    if (isAccountAllowedForLaunchMode(selected, formLaunchMode)) return;
    setFormBindAccountId("");
  }, [
    formBindAccountId,
    formLaunchMode,
    isAccountAllowedForLaunchMode,
    resolveBoundAccount,
  ]);

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
    setFormAppSpeed(instance.appSpeed ?? "standard");
    setFormBindAccountId(instance.bindAccountId || "");
    setFormModelRoutingEnabled(Boolean(instance.modelRouting?.enabled));
    setFormModelRoutes(
      instance.modelRouting?.routes?.map((route) => ({ ...route })) ?? [],
    );
    setFormCodexQuickConfig(null);
    setFormExperimentalModelCatalogEnabled(false);
    setFormExperimentalModels([]);
    setFormExperimentalModelsError(null);
    setFormCodexQuickConfigLoading(isCodexApp);
    setFormCodexQuickConfigError(null);
    setFormCodexOpenConfigLoading(false);
    setFormError(null);
    setPathAuto(false);
    setShowModal(true);
  };

  useEffect(() => {
    if (!isCodexApp) return;
    const handleEditBoundAccount = (event: Event) => {
      const instanceId = (event as CustomEvent<{ instanceId?: string }>).detail
        ?.instanceId;
      if (!instanceId) return;
      const target = instances.find((instance) => instance.id === instanceId);
      if (target) openEditModal(target);
    };
    window.addEventListener(
      "codex:edit-instance-account",
      handleEditBoundAccount as EventListener,
    );
    return () => {
      window.removeEventListener(
        "codex:edit-instance-account",
        handleEditBoundAccount as EventListener,
      );
    };
  }, [instances, isCodexApp]);

  useEffect(() => {
    if (!isCodexApp || !onInstanceStarted) return;
    const handleTransferredLaunch = (event: Event) => {
      const instance = (event as CustomEvent<{ instance?: InstanceProfile }>)
        .detail?.instance;
      if (!instance) return;
      void Promise.resolve(onInstanceStarted(instance)).catch((error) => {
        setMessage({ text: String(error), tone: "error" });
      });
    };
    window.addEventListener(
      "codex:instance-launch-transferred",
      handleTransferredLaunch as EventListener,
    );
    return () => {
      window.removeEventListener(
        "codex:instance-launch-transferred",
        handleTransferredLaunch as EventListener,
      );
    };
  }, [isCodexApp, onInstanceStarted]);

  const closeModal = () => {
    setOpenInlineMenuId(null);
    setShowModal(false);
    resetForm();
    setEditing(null);
  };

  const clearDeleteConfirm = useCallback(() => {
    setDeleteConfirmInstance(null);
    clearDeleteInstanceError();
  }, [clearDeleteInstanceError]);

  const dismissDeleteConfirm = useCallback(() => {
    if (deleteConfirmInstance && actionLoading === deleteConfirmInstance.id) {
      return;
    }
    clearDeleteConfirm();
  }, [actionLoading, clearDeleteConfirm, deleteConfirmInstance]);

  useEscClose(showModal, closeModal);
  useEscClose(!!initGuideInstance, () => setInitGuideInstance(null));
  useEscClose(!!deleteConfirmInstance, dismissDeleteConfirm);
  useEscClose(!!runningNoticeInstance, () => setRunningNoticeInstance(null));

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
    const isCreateEmpty =
      !editing && supportsInstanceInitialization && formInitMode === "empty";

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

    const isExistingDir =
      !editing &&
      supportsInstanceInitialization &&
      formInitMode === "existingDir";

    if (
      !editing &&
      supportsInstanceInitialization &&
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

    if (
      !editing &&
      supportsInstanceInitialization &&
      !isCreateEmpty &&
      !isExistingDir &&
      !formBindAccountId
    ) {
      setFormError(t("instances.form.bindRequired", "请选择要绑定的账号"));
      setFormErrorTick((prev) => prev + 1);
      return;
    }

    if (
      editing &&
      isCodexApp &&
      formExperimentalModelCatalogEnabled &&
      formExperimentalModelsError
    ) {
      setFormError(formExperimentalModelsError);
      setFormErrorTick((prev) => prev + 1);
      return;
    }

    let nextModelRouting: CodexInstanceModelRouting | null = null;
    let nextExperimentalModels = formExperimentalModels;
    let nextExperimentalModelCatalogEnabled =
      formExperimentalModelCatalogEnabled;
    if (isCodexApp && formModelRoutingEnabled) {
      if (
        editing &&
        (formCodexQuickConfigLoading ||
          !formCodexQuickConfig?.experimental_model_catalog_available)
      ) {
        setFormError(
          formExperimentalModelUnavailableMessage ||
            t(
              "instances.form.modelRouting.catalogUnavailable",
              "当前实例的可见模型目录不可由 Cockpit 管理，暂不能启用模型路由。",
            ),
        );
        setFormErrorTick((prev) => prev + 1);
        return;
      }
      if (formLaunchMode !== "app") {
        setFormError(
          t(
            "instances.form.modelRouting.desktopOnly",
            "混合模型路由仅支持桌面版实例。",
          ),
        );
        setFormErrorTick((prev) => prev + 1);
        return;
      }
      const boundAccount = resolveBoundAccount(formBindAccountId).account;
      if (
        !boundAccount ||
        boundAccount.auth_mode?.trim().toLowerCase() === "apikey" ||
        isApiServiceBindId(formBindAccountId) ||
        Boolean(parseProviderGatewayBindAccountId(formBindAccountId))
      ) {
        setFormError(
          t(
            "instances.form.modelRouting.oauthRequired",
            "混合模型路由需要绑定一个直接登录的 OAuth 订阅账号。",
          ),
        );
        setFormErrorTick((prev) => prev + 1);
        return;
      }
      if (formModelRoutes.length === 0) {
        setFormError(
          t(
            "instances.form.modelRouting.routeRequired",
            "请至少添加一个 API 模型路由。",
          ),
        );
        setFormErrorTick((prev) => prev + 1);
        return;
      }

      const namespaces = new Set<string>();
      const normalizedRoutes: CodexInstanceApiRoute[] = [];
      for (const route of formModelRoutes) {
        const namespace = route.namespace.trim().toLowerCase();
        if (
          !CODEX_MODEL_ROUTE_NAMESPACE_PATTERN.test(namespace) ||
          CODEX_RESERVED_MODEL_ROUTE_NAMESPACES.has(namespace)
        ) {
          setFormError(
            t(
              "instances.form.modelRouting.invalidNamespace",
              "命名空间需为 2-32 位小写字母、数字、下划线或连字符，且不能使用保留名称。",
            ),
          );
          setFormErrorTick((prev) => prev + 1);
          return;
        }
        if (namespaces.has(namespace)) {
          setFormError(
            t(
              "instances.form.modelRouting.duplicateNamespace",
              "模型路由命名空间不能重复。",
            ),
          );
          setFormErrorTick((prev) => prev + 1);
          return;
        }
        namespaces.add(namespace);
        const provider = modelRoutingProviderAccounts.find(
          (account) => account.id === route.providerAccountId,
        );
        if (!provider) {
          setFormError(
            t(
              "instances.form.modelRouting.providerRequired",
              "每个模型路由都必须选择一个 API 账号。",
            ),
          );
          setFormErrorTick((prev) => prev + 1);
          return;
        }
        if (
          route.enabled &&
          route.selectedModels !== undefined &&
          route.selectedModels.filter((model) => model.trim()).length === 0
        ) {
          setFormError(
            t(
              "instances.form.modelRouting.modelRequired",
              "每个已启用的 API 路由至少需要选择一个模型。",
            ),
          );
          setFormErrorTick((prev) => prev + 1);
          return;
        }
        normalizedRoutes.push({
          ...route,
          namespace,
          providerAccountId: provider.id,
        });
      }
      if (!normalizedRoutes.some((route) => route.enabled)) {
        setFormError(
          t(
            "instances.form.modelRouting.enabledRouteRequired",
            "请至少启用一个 API 模型路由。",
          ),
        );
        setFormErrorTick((prev) => prev + 1);
        return;
      }

      nextModelRouting = {
        enabled: true,
        version: 1,
        routes: normalizedRoutes,
      };
      nextExperimentalModels = syncExperimentalModelsWithRouting(
        formExperimentalModels,
        normalizedRoutes,
        accounts,
        true,
      );
      nextExperimentalModelCatalogEnabled = true;
    } else if (editing && isCodexApp && editing.modelRouting?.enabled) {
      nextExperimentalModels = syncExperimentalModelsWithRouting(
        formExperimentalModels,
        [],
        accounts,
        false,
      );
    }
    const modelRoutingChanged = Boolean(
      editing &&
        isCodexApp &&
        JSON.stringify(editing.modelRouting ?? null) !==
          JSON.stringify(nextModelRouting),
    );

    try {
      const nextLaunchMode = supportsLaunchModeSelect
        ? formLaunchMode
        : undefined;
      const nextWorkingDir = showWorkingDirField ? formWorkingDir : null;
      if (editing) {
        let restartAfterSave = false;
        if (isCodexApp && modelRoutingChanged && editing.running) {
          restartAfterSave = await confirmDialog(
            t(
              "instances.form.modelRouting.runningSaveMessage",
              "此 Codex 实例正在运行。立即替换本地服务会中断当前对话。\n\n选择“保存并重启 Codex”立即生效；选择“仅保存，稍后生效”不会影响当前会话。",
            ),
            {
              title: t(
                "instances.form.modelRouting.runningSaveTitle",
                "路由配置何时生效？",
              ),
              okLabel: t(
                "instances.form.modelRouting.saveAndRestart",
                "保存并重启 Codex",
              ),
              cancelLabel: t(
                "instances.form.modelRouting.saveForLater",
                "仅保存，稍后生效",
              ),
              kind: "warning",
            },
          );
        }
        setActionLoading(editing.id);
        const updatePayload: {
          instanceId: string;
          name?: string;
          workingDir?: string | null;
          extraArgs?: string;
          bindAccountId?: string | null;
          modelRouting?: CodexInstanceModelRouting | null;
          followLocalAccount?: boolean;
          launchMode?: InstanceLaunchMode;
          appSpeed?: CodexAppSpeed;
          deferBindAccountApplication?: boolean;
        } = {
          instanceId: editing.id,
          workingDir: nextWorkingDir,
          extraArgs: formExtraArgs,
          launchMode: nextLaunchMode,
          appSpeed: isCodexApp ? formAppSpeed : undefined,
        };
        if (modelRoutingChanged) {
          updatePayload.modelRouting = nextModelRouting;
          if (editing.running) {
            updatePayload.deferBindAccountApplication = true;
          }
        }
        if (!isEditingDefault) {
          updatePayload.name = formName.trim();
        }
        const canEditBind =
          isGrokApp || !(editing.initialized === false && !isEditingDefault);
        if (canEditBind) {
          const nextBindId = resolveBindAccountValue(formBindAccountId);
          if (nextBindId !== (editing.bindAccountId ?? null)) {
            updatePayload.bindAccountId = nextBindId;
          }
        }
        if (isEditingDefault) {
          updatePayload.followLocalAccount = false;
        }

        const shouldSaveQuickConfig =
          isCodexApp && (formCodexQuickConfigDirty || modelRoutingChanged);
        if (shouldSaveQuickConfig) {
          await saveCodexInstanceConfiguration({
            ...updatePayload,
            experimentalModelCatalogEnabled:
              nextExperimentalModelCatalogEnabled,
            experimentalModelCatalogModels: nextExperimentalModels,
            experimentalModelCatalogDefaultModelId:
              formExperimentalDefaultModelId,
          });
          await refreshInstances();
        } else {
          await updateInstance(updatePayload);
        }
        if (restartAfterSave) {
          try {
            await stopInstance(editing.id);
            await startInstance(editing.id);
          } catch (restartError) {
            if (!formCodexQuickConfig) throw restartError;
            try {
              await saveCodexInstanceConfiguration({
                instanceId: editing.id,
                bindAccountId: editing.bindAccountId ?? null,
                modelRouting: editing.modelRouting ?? null,
                deferBindAccountApplication: true,
                experimentalModelCatalogEnabled:
                  formCodexQuickConfig.experimental_model_catalog_enabled,
                experimentalModelCatalogModels:
                  formCodexQuickConfig.experimental_model_catalog_models,
                experimentalModelCatalogDefaultModelId:
                  formCodexQuickConfig.experimental_model_catalog_default_model_id ??
                  null,
              });
              await startInstance(editing.id);
            } catch (rollbackError) {
              throw new Error(
                `${String(restartError)}\n${t(
                  "instances.form.modelRouting.restartRollbackFailed",
                  "新路由启动失败，恢复原配置后仍无法启动：",
                )}${String(rollbackError)}`,
              );
            }
            throw new Error(
              t("instances.form.modelRouting.restartRolledBack", {
                defaultValue:
                  "新路由启动失败，已恢复原配置并重新启动 Codex：{{error}}",
                error: String(restartError).replace(/^Error:\s*/, ""),
              }),
            );
          }
        }
        setMessage({ text: t("instances.messages.updated", "实例已更新") });
      } else {
        setActionLoading("create");
        const created = await createInstance({
          name: formName.trim(),
          userDataDir: formPath.trim(),
          workingDir: nextWorkingDir,
          extraArgs: formExtraArgs,
          initMode: isGrokApp ? "empty" : formInitMode,
          launchMode: nextLaunchMode,
          appSpeed: isCodexApp ? formAppSpeed : undefined,
          bindAccountId: isCreateEmpty
            ? null
            : resolveBindAccountValue(formBindAccountId),
          modelRouting: nextModelRouting,
          copySourceInstanceId: formCopySourceInstanceId || defaultInstanceId,
        });
        if (
          created?.id &&
          nextModelRouting &&
          nextExperimentalModelCatalogEnabled
        ) {
          await saveCodexInstanceQuickConfig(
            created.id,
            undefined,
            undefined,
            true,
            nextExperimentalModels,
            formExperimentalDefaultModelId,
          );
        }
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
      setFormError(getCodexExperimentalModelErrorMessage(t, e) ?? String(e));
      setFormErrorTick((prev) => prev + 1);
    } finally {
      setActionLoading(null);
    }
  };

  const handleDelete = (instance: InstanceProfile) => {
    clearDeleteInstanceError();
    setDeleteConfirmInstance(instance);
  };

  const handleConfirmDelete = async () => {
    if (!deleteConfirmInstance) return;
    const target = deleteConfirmInstance;
    clearDeleteInstanceError();
    setActionLoading(target.id);
    try {
      await deleteInstance(target.id);
      setMessage({ text: t("instances.messages.deleted", "实例已删除") });
      clearDeleteConfirm();
    } catch (e) {
      const errorMessage = String(e)
        .replace(/^Error:\s*/, "")
        .trim();
      reportDeleteInstanceError(errorMessage || t("common.failed", "失败"));
    } finally {
      setActionLoading(null);
    }
  };

  useEnterConfirm(
    !!deleteConfirmInstance && actionLoading !== deleteConfirmInstance?.id,
    () => {
      void handleConfirmDelete();
    },
  );

  const handleMissingPathError = (error: unknown, instanceId?: string) => {
    const message = String(error ?? "");
    const missingPathPrefix = "APP_PATH_NOT_FOUND:";
    const multiInstanceExePrefix = "CLAUDE_MULTI_INSTANCE_REQUIRES_EXE:";
    const isMissingPath = message.startsWith(missingPathPrefix);
    const isMultiInstanceExeRequired = message.startsWith(
      multiInstanceExePrefix,
    );
    if (!isMissingPath && !isMultiInstanceExeRequired) {
      return false;
    }
    const rawApp = isMultiInstanceExeRequired
      ? message.slice(multiInstanceExePrefix.length)
      : message.slice(missingPathPrefix.length);
    const app =
      rawApp === "codex" ||
      rawApp === "claude" ||
      rawApp === "antigravity" ||
      rawApp === "vscode" ||
      rawApp === "windsurf" ||
      rawApp === "kiro" ||
      rawApp === "cursor" ||
      rawApp === "grok" ||
      rawApp === "codebuddy" ||
      rawApp === "codebuddy_cn" ||
      rawApp === "qoder" ||
      rawApp === "zcode"
        ? rawApp
        : appType;
    const runtimeTarget =
      appType === "antigravity" || appType === "antigravity_ide"
        ? appType
        : undefined;
    const retry = instanceId
      ? { kind: "instance" as const, instanceId, runtimeTarget }
      : { kind: "default" as const, runtimeTarget };
    window.dispatchEvent(
      new CustomEvent("app-path-missing", { detail: { app, retry } }),
    );
    return true;
  };

  const handleCodexManagedStoreLaunchError = (error: unknown) => {
    const message = String(error ?? "").replace(/^Error:\s*/, "");
    const prefix = "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:";
    if (appType !== "codex" || !message.startsWith(prefix)) {
      return false;
    }

    const detail = message.slice(prefix.length).trim();
    setMessage({
      text: t(
        "instances.messages.codexManagedStoreLaunchUnsafe",
        "Windows Store 无法可靠传递实例目录，已阻止打开默认账号。请将该实例的启动方式切换为 CLI 后重试。详情：{{detail}}",
        { detail },
      ),
      tone: "error",
    });
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

      if (onBeforeStart) {
        try {
          const allowed = await onBeforeStart(instance);
          if (!allowed) {
            return "cancelled";
          }
        } catch (error) {
          setMessage({ text: String(error), tone: "error" });
          return "failed";
        }
      }

      if (!preMarkedStarting) {
        markInstanceStarting(instance.id);
      }
      const flowStartedAt = performance.now();
      console.info("[Instance Start][UI] button loading started", {
        instanceId: instance.id,
        instanceName: instance.name,
      });

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
        if (isCodexApp && isCodexInstanceAccountConflict(e)) {
          return "failed";
        }
        if (onInstanceStartError) {
          try {
            if (await onInstanceStartError(e, instance)) {
              return "failed";
            }
          } catch (callbackError) {
            setMessage({ text: String(callbackError), tone: "error" });
            return "failed";
          }
        }
        if (handleMissingPathError(e, instance.id)) {
          return "missing-path";
        }
        if (handleCodexManagedStoreLaunchError(e)) {
          return "failed";
        }
        const retryStart = async () => {
          const startedInstance = await startInstance(instance.id);
          await Promise.resolve(onInstanceStarted?.(startedInstance));
          triggerDelayedRefreshAfterStart();
        };
        if (
          presentWindowsOperationError({
            error: e,
            operation: "launch_app",
            retry: retryStart,
            manualContinue: retryStart,
          })
        ) {
          return "failed";
        }
        if (isCodexApp) {
          return "failed";
        }
        setMessage({ text: String(e), tone: "error" });
        return "failed";
      } finally {
        if (!preMarkedStarting) {
          unmarkInstanceStarting(instance.id);
        }
        console.info("[Instance Start][UI] button loading finished", {
          instanceId: instance.id,
          instanceName: instance.name,
          elapsedMs: Math.round(performance.now() - flowStartedAt),
        });
      }
    },
    [
      handleMissingPathError,
      handleCodexManagedStoreLaunchError,
      isCodexApp,
      markInstanceStarting,
      onBeforeStart,
      onInstanceStartError,
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
      const retryStop = async () => {
        await stopInstance(instance.id);
        await refreshInstances();
      };
      if (
        presentWindowsOperationError({
          error: e,
          operation: "stop_process",
          retry: retryStop,
          manualContinue: retryStop,
        })
      ) {
        return;
      }
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
      if (
        presentWindowsOperationError({
          error: e,
          operation: "open_path",
          retry: async () => {
            await openInstanceWindow(runningNoticeInstance.id);
            setRunningNoticeInstance(null);
          },
        })
      ) {
        return;
      }
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
      if (
        presentWindowsOperationError({
          error: e,
          operation: "open_path",
          retry: async () => {
            await openInstanceWindow(instance.id);
          },
        })
      ) {
        return;
      }
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setActionLoading(null);
    }
  };

  const handleShowFloatingCard = async (instance: InstanceProfile) => {
    const { accountId, missing } = resolveAccount(instance);
    if (!instance.bindAccountId || !accountId || missing) {
      return;
    }
    try {
      await showInstanceFloatingCardWindow({
        platformId: floatingCardPlatformId,
        instanceId: instance.id,
        instanceName: instance.isDefault
          ? t("instances.defaultName", "默认实例")
          : instance.name || t("instances.defaultName", "默认实例"),
        boundAccountId: accountId,
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
        if (outcome === "already-running" || outcome === "cancelled") {
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
    return resolveBoundAccount(instance.bindAccountId);
  };

  const selectedCopySourceInstance = useMemo(() => {
    if (!formCopySourceInstanceId) {
      return instances.find((item) => item.id === defaultInstanceId) || null;
    }
    return (
      instances.find((item) => item.id === formCopySourceInstanceId) || null
    );
  }, [defaultInstanceId, formCopySourceInstanceId, instances]);
  const availableCopySourceInstances = useMemo(
    () =>
      sortedInstances.filter(
        (instance) =>
          instance.isDefault ||
          resolveInstanceLaunchMode(instance) === formLaunchMode,
      ),
    [formLaunchMode, sortedInstances],
  );

  useEffect(() => {
    if (editing || formInitMode !== "copy") return;
    if (!formCopySourceInstanceId) {
      setFormCopySourceInstanceId(defaultInstanceId);
      return;
    }
    const selected = availableCopySourceInstances.find(
      (instance) => instance.id === formCopySourceInstanceId,
    );
    if (!selected) {
      setFormCopySourceInstanceId(defaultInstanceId);
    }
  }, [
    availableCopySourceInstances,
    defaultInstanceId,
    editing,
    formCopySourceInstanceId,
    formInitMode,
  ]);

  const applyFormCodexQuickConfig = useCallback(
    (nextConfig: CodexQuickConfig) => {
      setFormCodexQuickConfig(nextConfig);
      setFormExperimentalModelCatalogEnabled(
        nextConfig.experimental_model_catalog_enabled,
      );
      setFormExperimentalModels(nextConfig.experimental_model_catalog_models);
      setFormExperimentalDefaultModelId(
        nextConfig.experimental_model_catalog_default_model_id ?? null,
      );
      setFormExperimentalModelsError(null);
    },
    [],
  );

  const applyCopySourceSettings = useCallback(
    (source: InstanceProfile) => {
      setFormLaunchMode(resolveInstanceLaunchMode(source));
      setFormAppSpeed(source.appSpeed ?? "standard");
      setFormExtraArgs(source.extraArgs || "");
      setFormWorkingDir(source.workingDir || "");
      setFormBindAccountId(source.bindAccountId || "");
      setFormModelRoutingEnabled(Boolean(source.modelRouting?.enabled));
      setFormModelRoutes(
        source.modelRouting?.routes?.map((route) => ({ ...route })) ?? [],
      );
      const sourceName = source.isDefault
        ? t("instances.defaultName", "默认实例")
        : source.name?.trim() || source.id;
      const nextName = t("instances.form.copyName", "{{name}} 副本", {
        name: sourceName,
      });
      setFormName((current) => {
        if (!current.trim() || current === copyAutoNameRef.current) {
          copyAutoNameRef.current = nextName;
          return nextName;
        }
        return current;
      });
      if (!isCodexApp) return;
      setFormCodexQuickConfigError(null);
      setFormCodexQuickConfigLoading(true);
      void getCodexInstanceQuickConfig(source.id)
        .then((config) => {
          applyFormCodexQuickConfig(config);
        })
        .catch((error) => {
          setFormCodexQuickConfigError(
            String(error).replace(/^Error:\s*/, ""),
          );
        })
        .finally(() => {
          setFormCodexQuickConfigLoading(false);
        });
    },
    [applyFormCodexQuickConfig, isCodexApp, t],
  );

  useEffect(() => {
    if (editing || !showModal || formInitMode !== "copy") return;
    const source = selectedCopySourceInstance;
    if (!source) return;
    if (lastAppliedCopySourceRef.current === source.id) return;
    lastAppliedCopySourceRef.current = source.id;
    applyCopySourceSettings(source);
  }, [
    applyCopySourceSettings,
    editing,
    formInitMode,
    selectedCopySourceInstance,
    showModal,
  ]);

  const formCodexQuickConfigDirty = useMemo(() => {
    if (!formCodexQuickConfig) return false;
    return (
      formCodexQuickConfig.experimental_model_catalog_enabled !==
        formExperimentalModelCatalogEnabled ||
      JSON.stringify(formCodexQuickConfig.experimental_model_catalog_models) !==
        JSON.stringify(formExperimentalModels) ||
      (formCodexQuickConfig.experimental_model_catalog_default_model_id ??
        null) !== formExperimentalDefaultModelId
    );
  }, [
    formCodexQuickConfig,
    formExperimentalModelCatalogEnabled,
    formExperimentalDefaultModelId,
    formExperimentalModels,
  ]);
  const formExperimentalModelUnavailableMessage = useMemo(() => {
    const reason =
      formCodexQuickConfig?.experimental_model_catalog_unavailable_reason;
    if (!reason) return null;
    if (reason === "catalog_conflict") {
      return t(
        "codex.experimentalModelCatalog.unavailable.catalogConflict",
        "已有其他 model_catalog_json，禁止覆盖。",
      );
    }
    return null;
  }, [formCodexQuickConfig, t]);
  const handleOpenFormCodexConfigToml = useCallback(async () => {
    if (!editing) return;
    setFormCodexQuickConfigError(null);
    setFormCodexOpenConfigLoading(true);
    try {
      await openCodexInstanceConfigToml(editing.id);
    } catch (error) {
      setFormCodexQuickConfigError(
        t("instances.form.codexQuickConfig.openFailed", {
          defaultValue: "打开 config.toml 失败：{{error}}",
          error: String(error),
        }),
      );
    } finally {
      setFormCodexOpenConfigLoading(false);
    }
  }, [editing, t]);

  useEffect(() => {
    if (!isCodexApp || !showModal || !editing) return;
    let active = true;
    setFormCodexQuickConfigLoading(true);
    setFormCodexQuickConfigError(null);
    void getCodexInstanceQuickConfig(editing.id)
      .then((quickConfig) => {
        if (!active) return;
        applyFormCodexQuickConfig(quickConfig);
        if (editing.modelRouting?.enabled && editing.modelRouting.routes?.length) {
          setFormExperimentalModels(
            syncExperimentalModelsWithRouting(
              quickConfig.experimental_model_catalog_models,
              editing.modelRouting?.routes ?? [],
              accounts,
              true,
            ),
          );
        }
      })
      .catch((error) => {
        if (!active) return;
        setFormCodexQuickConfigError(
          t("instances.form.codexQuickConfig.loadFailed", {
            defaultValue: "加载当前 Codex 配置失败：{{error}}",
            error: String(error),
          }),
        );
      })
      .finally(() => {
        if (!active) return;
        setFormCodexQuickConfigLoading(false);
      });
    return () => {
      active = false;
    };
  }, [applyFormCodexQuickConfig, editing, isCodexApp, showModal, t]);

  const renderAccountMenuItems = ({
    visibleAccounts,
    availableTags,
    searchValue,
    onSearchChange,
    tagFilter,
    onToggleTagFilter,
    onClearTagFilter,
    value,
    isFollowingCurrent = false,
    allowFollowCurrent = false,
    allowUnbound = false,
    onFollowCurrent,
    onChange,
    onClose,
    selectedAccount,
  }: AccountMenuItemsRenderArgs<TAccount>) => (
    <>
      <div className="account-select-menu-toolbar">
        <label className="account-select-search-box">
          <Search size={14} />
          <input
            type="text"
            value={searchValue}
            onChange={(event) => onSearchChange(event.target.value)}
            placeholder={t("accounts.search", "搜索账号...")}
          />
        </label>
        {availableTags.length > 0 ? (
          <div className="account-select-tag-filter">
            <span className="account-select-tag-filter-label">
              {t("accounts.filterTags", "标签筛选")}
            </span>
            <div className="account-select-tag-filter-list">
              {availableTags.map((tag) => (
                <button
                  key={tag}
                  type="button"
                  className={`account-select-tag-pill ${
                    tagFilter.includes(tag) ? "active" : ""
                  }`}
                  onClick={() => onToggleTagFilter(tag)}
                >
                  {tag}
                </button>
              ))}
              {tagFilter.length > 0 ? (
                <button
                  type="button"
                  className="account-select-tag-clear"
                  onClick={onClearTagFilter}
                >
                  {t("accounts.clearFilter", "清空筛选")}
                </button>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
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
      {isCodexApp && (
        <button
          type="button"
          className={`account-select-item ${value === CODEX_API_SERVICE_BIND_ID && !isFollowingCurrent ? "active" : ""}`}
          data-account-select-active={
            value === CODEX_API_SERVICE_BIND_ID && !isFollowingCurrent
              ? "true"
              : undefined
          }
          onClick={() => {
            onChange(CODEX_API_SERVICE_BIND_ID);
            onClose();
          }}
        >
          <span className="account-select-email">
            {resolveApiServiceLabel()}
          </span>
        </button>
      )}
      {visibleAccounts.map((account) => {
        const bindValue = resolveBindAccountValue(account.id) ?? account.id;
        const active = value === bindValue && !isFollowingCurrent;
        const displayText = resolveAccountDisplayText(account);
        return (
          <button
            type="button"
            key={account.id}
            className={`account-select-item ${active ? "active" : ""}`}
            data-account-select-active={active ? "true" : undefined}
            onClick={() => {
              onChange(bindValue);
              onClose();
            }}
          >
            <span className="account-select-email-row">
              <span
                className="account-select-email"
                title={maskAccountText(displayText)}
              >
                {maskAccountText(displayText)}
              </span>
              {renderAccountBadge?.(account)}
            </span>
            {renderAccountQuotaPreview(account)}
          </button>
        );
      })}
      {visibleAccounts.length === 0 &&
      !isCodexApp &&
      !allowUnbound &&
      !allowFollowCurrent ? (
        <div className="account-select-empty">
          {t("common.noData", "暂无数据")}
        </div>
      ) : null}
    </>
  );

  const renderFormAccountSelect = (props: BaseAccountSelectProps) => (
    <InlineAccountSelect
      {...props}
      accounts={accounts}
      launchMode={formLaunchMode}
      filterAccountsForLaunchMode={filterAccountsForLaunchMode}
      getAccountSearchText={getAccountSearchText}
      resolveAccountDisplayText={resolveAccountDisplayText}
      isApiServiceBindId={isApiServiceBindId}
      resolveBoundAccount={resolveBoundAccount}
      renderAccountQuotaPreview={renderAccountQuotaPreview}
      renderAccountBadge={renderAccountBadge}
      maskAccountText={maskAccountText}
      resolveApiServiceLabel={resolveApiServiceLabel}
      renderAccountMenuItems={renderAccountMenuItems}
      unboundLabel={t("instances.form.unbound", "不绑定")}
      selectAccountLabel={t("instances.form.selectAccount", "选择账号")}
      missingAccountLabel={t("instances.quota.accountMissing", "账号不存在")}
      followCurrentLabel={t("instances.form.followCurrent", "跟随当前账号")}
    />
  );

  // 注意：不要把带 useState 的下拉组件定义在 render 内部（否则父级重渲染会重置 open）。
  // 复制来源实例使用模块级 SingleSelectDropdown（portal + 稳定类型）。
  const copySourceOptions = useMemo(
    () =>
      availableCopySourceInstances.map((instance) => ({
        value: instance.id,
        label: instance.isDefault
          ? t("instances.defaultName", "默认实例")
          : instance.name || instance.id,
      })),
    [availableCopySourceInstances, t],
  );

  const handleFormAccountChange = (nextId: string | null) => {
    setFormBindAccountId(resolveBindAccountValue(nextId) ?? "");
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
    if (!isGrokApp && instance.initialized === false) {
      setInitGuideInstance(instance);
      return;
    }
    if (!nextId) return;
    const normalizedNextId = resolveBindAccountValue(nextId);
    const sameSelection = (instance.bindAccountId || null) === normalizedNextId;
    if (sameSelection && !instance.followLocalAccount) return;
    setActionLoading(instance.id);
    try {
      await updateInstance({
        instanceId: instance.id,
        bindAccountId: normalizedNextId,
        followLocalAccount: instance.isDefault ? false : undefined,
        deferBindAccountApplication: isCodexApp,
      });
    } catch (e) {
      setMessage({ text: String(e), tone: "error" });
    } finally {
      setActionLoading(null);
    }
  };

  const handleInlineSpeedChange = async (
    instance: InstanceProfile,
    speed: CodexAppSpeed,
  ) => {
    if (!isCodexApp) return;
    setActionLoading(instance.id);
    try {
      await updateInstance({
        instanceId: instance.id,
        appSpeed: speed,
      });
      setMessage({ text: t("instances.messages.speedUpdated", "速度已更新") });
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
            className="btn btn-primary icon-only"
            onClick={openCreateModal}
            title={t("instances.actions.create", "新建实例")}
            aria-label={t("instances.actions.create", "新建实例")}
          >
            <Plus size={16} />
          </button>
          {!isGrokApp && (
            <button
              className="btn btn-secondary icon-only"
              onClick={handleStartAll}
              disabled={bulkActionLoading || restartingAll}
              title={t("instances.actions.startAll", "全部启动")}
              aria-label={t("instances.actions.startAll", "全部启动")}
            >
              <Play size={16} />
            </button>
          )}
          {supportsStopControl && (
            <button
              className="btn btn-secondary icon-only"
              onClick={handleCloseAll}
              disabled={bulkActionLoading || restartingAll}
              title={t("instances.actions.stopAll", "全部关闭")}
              aria-label={t("instances.actions.stopAll", "全部关闭")}
            >
              <Square size={16} />
            </button>
          )}
          <button
            className="btn btn-secondary icon-only"
            onClick={handleRefresh}
            disabled={refreshing || bulkActionLoading || restartingAll}
            title={t("instances.actions.refresh", "刷新")}
            aria-label={t("instances.actions.refresh", "刷新")}
          >
            <RefreshCw size={16} className={refreshing ? "icon-spin" : ""} />
          </button>
          {toolbarExtraActions}
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

      {loading && instances.length === 0 ? (
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
          className={`instances-list${
            isCodexApp ? " instances-list-codex" : ""
          }`}
        >
          <div className="instances-list-header">
            <div></div>
            <div>{t("instances.columns.instance", "实例")}</div>
            <div></div>
            <div>{t("instances.columns.email", "账号")}</div>
            {isCodexApp && <div>{t("instances.columns.speed", "速度")}</div>}
            <div>PID</div>
            <div>{t("instances.columns.actions", "操作")}</div>
          </div>
          {filteredInstances.map((instance) => {
            const {
              missing: accountMissing,
              isApiService: accountIsApiService,
            } = resolveAccount(instance);
            const accountDisabledByInit =
              !isGrokApp &&
              !instance.isDefault &&
              instance.initialized === false;
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
              Boolean(instance.bindAccountId) &&
              !accountMissing &&
              !accountIsApiService;
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
                    {(isCodexApp || isClaudeApp) && (
                      <span
                        className={`instance-launch-mode-badge ${launchMode}`}
                      >
                        {launchMode === "cli"
                          ? t("instances.form.launchModeCli", "CLI")
                        : t("instances.form.launchModeApp", "桌面版")}
                      </span>
                    )}
                    {isCodexApp && instance.modelRouting?.enabled && (
                      <span className="instance-model-routing-badge">
                        {t(
                          "instances.form.modelRouting.statusBadge",
                          "混合路由",
                        )}
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
                      accounts={accounts}
                      launchMode={launchMode}
                      filterAccountsForLaunchMode={filterAccountsForLaunchMode}
                      getAccountSearchText={getAccountSearchText}
                      resolveAccountDisplayText={resolveAccountDisplayText}
                      isApiServiceBindId={isApiServiceBindId}
                      resolveBoundAccount={resolveBoundAccount}
                      renderAccountQuotaPreview={renderAccountQuotaPreview}
                      renderAccountBadge={renderAccountBadge}
                      maskAccountText={maskAccountText}
                      resolveApiServiceLabel={resolveApiServiceLabel}
                      renderAccountMenuItems={renderAccountMenuItems}
                      disabled={isInstanceBusy}
                      missing={accountMissing}
                      placeholder={t("instances.labels.unbound", "未绑定")}
                      unboundLabel={t("instances.form.unbound", "不绑定")}
                      selectAccountLabel={t(
                        "instances.form.selectAccount",
                        "选择账号",
                      )}
                      missingAccountLabel={t(
                        "instances.quota.accountMissing",
                        "账号不存在",
                      )}
                      followCurrentLabel={t(
                        "instances.form.followCurrent",
                        "跟随当前账号",
                      )}
                      instanceId={instance.id}
                      currentOpenId={openInlineMenuId}
                      onOpenChange={(open) => {
                        setOpenInlineMenuId(open ? instance.id : null);
                      }}
                    />
                  )}
                </div>

                {isCodexApp && (
                  <div className="instance-speed">
                    <CodexSpeedSelect
                      value={instance.appSpeed ?? "standard"}
                      onChange={(speed) =>
                        void handleInlineSpeedChange(instance, speed)
                      }
                      busy={isInstanceBusy}
                      compact
                      preferredPlacement="top"
                      ariaLabel={t("codex.speed.title", "速度")}
                    />
                  </div>
                )}

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
                    title={t("instances.actions.duplicate", "复制实例")}
                    onClick={() => openDuplicateModal(instance)}
                    disabled={
                      isInstanceBusy || restartingAll || bulkActionLoading
                    }
                  >
                    <Copy size={16} />
                  </button>
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
        <div className="modal-overlay">
          <div
            className="modal instance-init-guide-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setInitGuideInstance(null)}
                title={t("common.back", "返回")}
                aria-label={t("common.back", "返回")}
              >
                <ChevronLeft size={14} />
              </button>
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
        <div className="modal-overlay">
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("instances.delete.title", "删除实例")}</h2>
              <button
                className="modal-close"
                onClick={dismissDeleteConfirm}
                disabled={actionLoading === deleteConfirmInstance.id}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage
                message={deleteInstanceError}
                scrollKey={deleteInstanceErrorScrollKey}
              />
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
                onClick={dismissDeleteConfirm}
                disabled={actionLoading === deleteConfirmInstance.id}
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
        <div className="modal-overlay">
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
        <div className="modal-overlay">
          <div
            className="modal modal-lg instance-editor-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={closeModal}
                title={t("common.back", "返回")}
                aria-label={t("common.back", "返回")}
              >
                <ChevronLeft size={14} />
              </button>
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

              {!editing && supportsInstanceInitialization && (
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

              {isCodexApp && (
                <div className="form-group">
                  <label>{t("instances.form.appSpeed", "速度")}</label>
                  <CodexSpeedSelect
                    value={formAppSpeed}
                    onChange={setFormAppSpeed}
                    preferredPlacement="bottom"
                    ariaLabel={t("codex.speed.title", "速度")}
                  />
                  <p className="form-hint">
                    {t(
                      "instances.form.appSpeedDesc",
                      "启动官方 Codex 前写入对应速度",
                    )}
                  </p>
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

              {!editing &&
                supportsInstanceInitialization &&
                formInitMode === "copy" && (
                  <div className="form-group">
                    <label>
                      {t("instances.form.copySource", "复制来源实例")}
                    </label>
                    <SingleSelectDropdown
                      value={formCopySourceInstanceId}
                      onChange={setFormCopySourceInstanceId}
                      options={
                        copySourceOptions.length > 0
                          ? copySourceOptions
                          : [
                              {
                                value: "__default__",
                                label: t("instances.defaultName", "默认实例"),
                              },
                            ]
                      }
                      placeholder={t(
                        "instances.form.copySourcePlaceholder",
                        "选择来源实例",
                      )}
                      ariaLabel={t("instances.form.copySource", "复制来源实例")}
                      className="instance-copy-source-select"
                    />
                    <p className="form-hint">
                      {t(
                        "instances.form.copySourceDesc",
                        "会复制来源实例的目录、绑定账号、启动方式和混合路由，不必重新配一遍。",
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
                  {supportsInstanceInitialization &&
                  formInitMode === "empty" ? (
                    <>
                      {renderFormAccountSelect({
                        value: null,
                        onChange: () => {},
                        disabled: true,
                        placeholder: t(
                          "instances.form.bindAfterInit",
                          "初始化后可绑定",
                        ),
                      })}
                      <p className="form-hint">
                        {t(
                          "instances.form.bindDisabledHint",
                          "空白实例需先启动一次生成实例数据后，才可绑定账号。",
                        )}
                      </p>
                    </>
                  ) : (
                    renderFormAccountSelect({
                      value: formBindAccountId || null,
                      onChange: handleFormAccountChange,
                    })
                  )}
                </div>
              ) : (
                <div className="form-group">
                  <label>{t("instances.form.bindAccount", "绑定账号")}</label>
                  {!isGrokApp &&
                  editing?.initialized === false &&
                  !editing.isDefault ? (
                    <>
                      {renderFormAccountSelect({
                        value: null,
                        onChange: () => {},
                        disabled: true,
                        placeholder: t(
                          "instances.form.bindAfterInit",
                          "初始化后可绑定",
                        ),
                      })}
                      <p className="form-hint">
                        {t(
                          "instances.form.bindDisabledHint",
                          "空白实例需先启动一次生成实例数据后，才可绑定账号。",
                        )}
                      </p>
                    </>
                  ) : (
                    renderFormAccountSelect({
                      value: formBindAccountId || null,
                      onChange: handleFormAccountChange,
                      missing: Boolean(
                        formBindAccountId &&
                        !isApiServiceBindId(formBindAccountId) &&
                        resolveBoundAccount(formBindAccountId).missing,
                      ),
                    })
                  )}
                </div>
              )}

              {isCodexApp &&
                formLaunchMode === "app" &&
                (editing || formInitMode !== "empty") && (
                  <CodexModelRoutingFields
                    enabled={formModelRoutingEnabled}
                    routes={formModelRoutes}
                    accounts={accounts}
                    running={Boolean(editing?.running)}
                    hint={
                      editing
                        ? undefined
                        : t(
                            "instances.form.modelRouting.createHint",
                            "新建时请复制默认实例，这样不用先走一遍 Codex 安装。空白实例才需要先启动一次。",
                          )
                    }
                    onEnabledChange={(enabled) => {
                      setFormModelRoutingEnabled(enabled);
                      if (
                        !editing &&
                        enabled &&
                        !formName.trim()
                      ) {
                        setFormName(
                          t(
                            "instances.form.modelRouting.defaultInstanceName",
                            "混合路由",
                          ),
                        );
                      }
                      setFormExperimentalModels((prevModels) =>
                        syncExperimentalModelsWithRouting(
                          prevModels,
                          formModelRoutes,
                          accounts,
                          enabled,
                        ),
                      );
                    }}
                    onRoutesChange={(nextRoutes) => {
                      setFormModelRoutes(nextRoutes);
                      setFormExperimentalModels((prevModels) =>
                        syncExperimentalModelsWithRouting(
                          prevModels,
                          nextRoutes,
                          accounts,
                          formModelRoutingEnabled,
                        ),
                      );
                    }}
                    onAccountsRefresh={fetchAccounts}
                    getAccountDisplayText={
                      getAccountDisplayText
                        ? (account) => getAccountDisplayText(account as TAccount)
                        : undefined
                    }
                  />
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

              {isCodexApp && editing && (
                <div className="form-group instance-codex-quick-config">
                  <div className="instance-codex-quick-header">
                    <label>
                      {t("codex.experimentalModelCatalog.title", "可见模型")}
                    </label>
                    <button
                      type="button"
                      className="btn btn-secondary instance-codex-quick-open-btn"
                      onClick={() => void handleOpenFormCodexConfigToml()}
                      disabled={
                        formCodexOpenConfigLoading ||
                        formCodexQuickConfigLoading
                      }
                    >
                      <FolderOpen size={14} />
                      {formCodexOpenConfigLoading
                        ? t("common.loading", "加载中...")
                        : t(
                            "instances.form.codexQuickConfig.openConfig",
                            "打开 config.toml",
                          )}
                    </button>
                  </div>
                  {formCodexQuickConfigLoading ? (
                    <p className="form-hint">
                      {t("common.loading", "加载中...")}
                    </p>
                  ) : (
                    <>
                      <div className="instance-codex-experimental-model">
                        <div className="instance-codex-experimental-model__copy">
                          <label htmlFor="instance-codex-experimental-model-catalog">
                            {t(
                              "codex.experimentalModelCatalog.title",
                              "可见模型",
                            )}
                          </label>
                          <p className="form-hint">
                            {t(
                              "codex.experimentalModelCatalog.description",
                              "统一管理可见模型、推理强度、上下文窗口和压缩阈值。",
                            )}
                          </p>
                          {formExperimentalModelCatalogEnabled && (
                            <p className="form-hint">
                              {t(
                                "codex.experimentalModelCatalog.enabledHint",
                                "启用后使用当前可见模型列表，重启 Codex 生效。",
                              )}
                            </p>
                          )}
                          {formExperimentalModelUnavailableMessage && (
                            <div className="form-error instance-codex-experimental-model__error">
                              {formExperimentalModelUnavailableMessage}
                            </div>
                          )}
                        </div>
                        <label className="instance-codex-experimental-model__switch">
                          <input
                            id="instance-codex-experimental-model-catalog"
                            type="checkbox"
                            checked={formExperimentalModelCatalogEnabled}
                            onChange={(event) => {
                              setFormCodexQuickConfigError(null);
                              setFormExperimentalModelCatalogEnabled(
                                event.target.checked,
                              );
                            }}
                            disabled={
                              actionLoading === editing.id ||
                              (!formExperimentalModelCatalogEnabled &&
                                !formCodexQuickConfig?.experimental_model_catalog_available)
                            }
                          />
                          <span className="instance-codex-experimental-model__switch-track" />
                        </label>
                      </div>
                      {formExperimentalModelCatalogEnabled && (
                        <CodexExperimentalModelEditor
                          models={formExperimentalModels}
                          defaultModelId={formExperimentalDefaultModelId}
                          mode="summary"
                          availableChannels={formAvailableChannels}
                          resolveModelSource={resolveFormModelSource}
                          onChange={(models) => {
                            setFormExperimentalModels(models);
                            setFormCodexQuickConfigError(null);
                          }}
                          onDefaultModelChange={(modelId) => {
                            setFormExperimentalDefaultModelId(modelId);
                            setFormCodexQuickConfigError(null);
                          }}
                          onValidationChange={setFormExperimentalModelsError}
                          onModelRemoved={(removedId) => {
                            setFormModelRoutes((prevRoutes) =>
                              toggleRouteModelInRoutes(prevRoutes, removedId, accounts, "remove"),
                            );
                          }}
                          onModelAdded={(addedId) => {
                            setFormModelRoutes((prevRoutes) =>
                              toggleRouteModelInRoutes(prevRoutes, addedId, accounts, "add"),
                            );
                          }}
                          disabled={actionLoading === editing.id}
                        />
                      )}
                      {formCodexQuickConfigError && (
                        <div className="form-error">
                          {formCodexQuickConfigError}
                        </div>
                      )}
                    </>
                  )}
                </div>
              )}
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
