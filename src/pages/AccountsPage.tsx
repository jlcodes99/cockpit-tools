import { useState, useEffect, useMemo, useRef, useCallback, Fragment, MouseEvent as ReactMouseEvent } from 'react'
import { createPortal } from 'react-dom'
import {
  RefreshCw,
  Upload,
  Trash2,
  X,
  Globe,
  Check,
  Lock,
  AlertTriangle,
  CircleAlert,
  Play,
  RotateCw,
  GripVertical,
  Eye,
  EyeOff,
  Tag,
  FolderOpen,
  FolderPlus,
  LogOut,
  Pencil,
  FileText,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useAccountStore } from '../stores/useAccountStore'
import * as accountService from '../services/accountService'
import { Account } from '../types/account'
import { Page } from '../types/navigation'
import {
  getAntigravityTierBadge,
  getQuotaClass,
  formatResetTimeDisplay,
} from '../utils/account'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import { useModalErrorState } from '../components/ModalErrorMessage'
import { useEscClose } from '../hooks/useEscClose'
import { useEnterConfirm } from '../hooks/useEnterConfirm'
import {
  AccountGroup,
  getAccountGroups,
  assignAccountsToGroup,
  removeAccountsFromGroup,
  removeAccountIdsFromAllGroups,
  deleteGroup,
  renameGroup,
} from '../services/accountGroupService'
import {
  GroupSettings,
  DisplayGroup,
  getDisplayGroups,
  calculateOverallQuota,
  calculateGroupQuota,
  updateGroupOrder
} from '../services/groupService'
import {
  getAntigravityQuotaDisplayItems,
} from '../presentation/platformAccountPresentation'
import {
  ANTIGRAVITY_RESET_SORT_PREFIX,
  DEFAULT_ANTIGRAVITY_SORT_BY,
  createAntigravityAccountComparator,
  normalizeAntigravitySortBy,
  normalizeAntigravitySortDirection,
} from '../utils/antigravityAccountSort'
import {
  mergeIdListsPreferExisting,
  subscribeUserMemory,
} from '../utils/userMemory'
import styles from '../styles/CompactView.module.css'
import { parseFileCorruptedError, type FileCorruptedError } from '../components/FileCorruptedModal'
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled,
  PRIVACY_MODE_CHANGED_EVENT
} from '../utils/privacy'
import { useExportJsonModal } from '../hooks/useExportJsonModal'
import type { MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown'
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from '../hooks/usePagination'
import {
  accountMatchesTagFilters,
  accountMatchesTypeFilters,
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
  collectAvailableAccountTags,
  normalizeAccountTag,
  type AccountFilterType,
} from '../utils/accountFilters'
import { loadWakeupOfficialLsVersionMode } from '../utils/wakeupOfficialLsVersion'
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
} from '../utils/accountValidityFilter'
import {
  FEATURE_UNLOCK_CHANGED_EVENT,
  type FeatureUnlockChangedDetail,
  isAntigravitySeamlessSwitchFeatureUnlocked,
} from '../utils/featureUnlocks'
import {
  consumeQueuedExternalProviderImportForPlatform,
  EXTERNAL_PROVIDER_IMPORT_EVENT,
  normalizeAntigravityExternalImportToken,
} from '../utils/externalProviderImport'
import {
  ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
  type AccountsOverviewFilterPersistenceChangedDetail,
  readAccountsOverviewFilterField,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence'
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget'
import {
  getMfaOtpToken,
  getMfaTimeRemaining,
  loadSavedMfaRecords,
  parseMfaCredentialInput,
  upsertSavedMfaRecord,
  type MfaRecord,
} from '../utils/mfaVault'
import { findFirstMailVerificationCode } from '../utils/mailVerificationCode'
import { AccountsOverviewView } from "./AccountsOverviewView";
import {
  ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH,
  ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID,
  ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
  ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG,
  ANTIGRAVITY_FILTER_FIELD_SORT_BY,
  ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
  ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
  ANTIGRAVITY_FILTER_FIELD_VIEW_MODE,
  ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
  ANTIGRAVITY_TOKEN_BATCH_EXAMPLE,
  ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE,
  DEFAULT_FILTER_TYPES,
  DEFAULT_TAG_FILTER,
  EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM,
  buildAntigravityAccountNoteForm,
  buildAntigravityAccountNoteUpdate,
  buildVerificationHistoryMaps,
  formatAntigravityMailPreviewTime,
  formatMfaRecordOption,
  getAntigravityAccountNoteTitle,
  hasAntigravityAccountNoteDetails,
  hasAntigravityAccountNoteFormDetails,
  isPendingAntigravityAccount,
  readAntigravityCustomSortActive,
  readAntigravityCustomSortOrder,
  writeAntigravityCustomSortActive,
  writeAntigravityCustomSortOrder,
  type AccountsFilterType,
  type AntigravityAccountNoteFormState,
  type AntigravityAccountNoteMailPreviewState,
  type ExtensionImportProgressPayload,
  type VerificationDetailRecord,
  type VerificationHistoryBatch,
  type ViewMode,
} from './antigravityAccountOverviewModel';


interface AccountsPageProps {
  onNavigate?: (page: Page) => void
}

type AntigravitySwitchHistoryItem = accountService.AntigravitySwitchHistoryItem

export type { AccountsFilterType } from './antigravityAccountOverviewModel';

export function useAccountsPageController({ onNavigate }: AccountsPageProps) {
  const { t, i18n } = useTranslation()
  const antigravityRuntimeTarget = useAntigravityRuntimeTarget()
  const locale = i18n.language || 'zh-CN'
  const untaggedKey = '__untagged__'
  const {
    accounts,
    currentAccountsByTarget,
    loading,
    error: storeError,
    fetchAccounts,
    fetchCurrentAccount,
    deleteAccounts,
    refreshQuota,
    refreshAllQuotas,
    startOAuthLogin,
    switchAccount,
    updateAccountTags,
    updateAccountNotes
  } = useAccountStore()
  const currentAccount = currentAccountsByTarget[antigravityRuntimeTarget] ?? null

  const formatSwitchError = useCallback((error: unknown) => String(error), [])

  // ─── 验证状态标记 ────────────────────────────────────────────────────
  // 优先读 disabled_reason（新版后端写入），没有则回退到验证历史（向后兼容）
  const [verificationStatusMap, setVerificationStatusMap] = useState<Record<string, string>>({})
  const [verificationDetailMap, setVerificationDetailMap] = useState<Record<string, VerificationDetailRecord>>({})

  const loadVerificationHistory = useCallback(async () => {
    const requestId = verificationHistoryRequestIdRef.current + 1
    verificationHistoryRequestIdRef.current = requestId

    try {
      const batches = await invoke<VerificationHistoryBatch[]>('wakeup_verification_load_history')
      if (verificationHistoryRequestIdRef.current !== requestId) {
        return
      }
      const { statusMap, detailMap } = buildVerificationHistoryMaps(batches || [])
      setVerificationStatusMap(statusMap)
      setVerificationDetailMap(detailMap)
    } catch (error) {
      if (verificationHistoryRequestIdRef.current !== requestId) {
        return
      }
      console.error('Failed to load verification history:', error)
    }
  }, [])

  const getVerificationBadge = useCallback((account: Account) => {
    // 优先从 disabled_reason 读（新版），回退到验证历史（旧数据兼容）
    const reason = account.disabled_reason || verificationStatusMap[account.id]
    if (reason === 'verification_required') {
      return { label: t('wakeup.errorUi.verificationRequiredTitle', 'Need Verify'), className: 'is-warning' }
    }
    if (reason === 'tos_violation') {
      return { label: t('wakeup.errorUi.tosViolationTitle', 'TOS'), className: 'is-tos-violation' }
    }
    return null
  }, [verificationStatusMap, t])

  // 文件损坏错误状态
  const [fileCorruptedError, setFileCorruptedError] = useState<FileCorruptedError | null>(null)

  // 监听 store 的 error 变化，检测文件损坏
  useEffect(() => {
    if (storeError) {
      const corrupted = parseFileCorruptedError(storeError)
      if (corrupted) {
        setFileCorruptedError(corrupted)
      }
    }
  }, [storeError])

  const initialFilterPersistenceEnabled = readAccountsOverviewFilterPersistenceEnabled(
    ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
  )
  const [filterPersistenceEnabled, setFilterPersistenceEnabled] = useState<boolean>(
    initialFilterPersistenceEnabled,
  )

  // View mode — always remember layout independently of filter-memory switch (#1200)
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    const saved = readAccountsOverviewFilterField<unknown>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_VIEW_MODE,
      'grid',
    )
    return saved === 'grid' || saved === 'list' || saved === 'compact'
      ? saved
      : 'grid'
  })
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault()
  )

  const handleViewModeChange = (mode: ViewMode) => {
    setViewMode(mode)
  }

  const togglePrivacyMode = () => {
    setPrivacyModeEnabled((prev) => {
      const next = !prev
      persistPrivacyModeEnabled(next)
      return next
    })
  }

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled]
  )

  // 筛选
  const [searchQuery, setSearchQuery] = useState('')
  const [filterTypes, setFilterTypes] = useState<AccountsFilterType[]>(() => {
    if (initialFilterPersistenceEnabled) {
      const saved = readAccountsOverviewFilterField<unknown>(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
        null,
      )
      if (saved !== null) {
        return readAccountsOverviewFilterStringArray(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
          ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
        ) as AccountsFilterType[]
      }
    }
    return DEFAULT_FILTER_TYPES
  })
  const [tagFilter, setTagFilter] = useState<string[]>(() => {
    if (initialFilterPersistenceEnabled) {
      const saved = readAccountsOverviewFilterField<unknown>(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
        null,
      )
      if (saved !== null) {
        return readAccountsOverviewFilterStringArray(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
          ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
        )
      }
    }
    return DEFAULT_TAG_FILTER
  })
  const [groupByTag, setGroupByTag] = useState<boolean>(() =>
    initialFilterPersistenceEnabled
      ? Boolean(
          readAccountsOverviewFilterField<unknown>(
            ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
            ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG,
            false,
          ),
        )
      : false,
  )

  const toggleFilterTypeValue = useCallback((value: AccountsFilterType) => {
    setFilterTypes((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value)
      }
      return [...prev, value]
    })
  }, [])

  const clearFilterTypes = useCallback(() => {
    setFilterTypes([])
  }, [])

  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [showAddModal, setShowAddModal] = useState(false)
  const [addTab, setAddTab] = useState<'oauth' | 'token' | 'import'>('oauth')
  const [refreshing, setRefreshing] = useState<Set<string>>(new Set())
  const [refreshingAll, setRefreshingAll] = useState(false)
  const [wakeupRunning, setWakeupRunning] = useState(false)
  const [switching, setSwitching] = useState<string | null>(null)
  const [importing, setImporting] = useState(false)
  const [refreshWarnings, setRefreshWarnings] = useState<
    Record<string, { kind: 'auth' | 'error'; message: string }>
  >({})
  const [refreshResult, setRefreshResult] = useState<Record<string, 'success' | 'error'>>({})
  const [message, setMessage] = useState<{
    text: string
    tone?: 'error'
  } | null>(null)
  const [includeExportSensitiveNotes, setIncludeExportSensitiveNotes] = useState(false)
  const includeExportSensitiveNotesRef = useRef(false)
  const exportAccountIdsRef = useRef<string[]>([])
  const exportSensitiveRefreshSeqRef = useRef(0)
  const [showSwitchHistoryModal, setShowSwitchHistoryModal] = useState(false)
  const [switchHistoryLoading, setSwitchHistoryLoading] = useState(false)
  const [switchHistoryClearing, setSwitchHistoryClearing] = useState(false)
  const [switchHistoryClearConfirmOpen, setSwitchHistoryClearConfirmOpen] = useState(false)
  const [switchHistory, setSwitchHistory] = useState<AntigravitySwitchHistoryItem[]>([])
  const [antigravitySeamlessSwitchUnlocked, setAntigravitySeamlessSwitchUnlocked] = useState(
    isAntigravitySeamlessSwitchFeatureUnlocked,
  )
  const exportModal = useExportJsonModal({
    exportFilePrefix: 'accounts_export',
    exportJsonByIds: async (ids) => {
      const raw = await accountService.exportAccounts(ids)
      if (includeExportSensitiveNotesRef.current) return raw
      try {
        const parsed = JSON.parse(raw) as unknown
        const strip = (value: unknown): unknown => {
          if (Array.isArray(value)) return value.map(strip)
          if (!value || typeof value !== 'object') return value
          const copy = { ...(value as Record<string, unknown>) }
          delete copy.two_factor_secret
          delete copy.account_password
          delete copy.phone_number
          delete copy.mail_url
          return copy
        }
        return JSON.stringify(strip(parsed), null, 2)
      } catch {
        return raw
      }
    },
    onError: (error) => {
      setMessage({
        text: t('messages.exportFailed', { error: String(error) }),
        tone: 'error',
      })
    },
  })
  const exporting = exportModal.preparing
  const [addStatus, setAddStatus] = useState<
    'idle' | 'loading' | 'success' | 'error'
  >('idle')
  const [addMessage, setAddMessage] = useState('')
  const [oauthUrl, setOauthUrl] = useState('')
  const [oauthUrlCopied, setOauthUrlCopied] = useState(false)
  const [oauthCallbackInput, setOauthCallbackInput] = useState('')
  const [oauthCallbackSubmitting, setOauthCallbackSubmitting] = useState(false)
  const [oauthCallbackError, setOauthCallbackError] = useState<string | null>(null)
  const [tokenInput, setTokenInput] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState<{
    ids: string[]
    message: string
  } | null>(null)
  const {
    message: deleteConfirmError,
    scrollKey: deleteConfirmErrorScrollKey,
    set: setDeleteConfirmError,
  } = useModalErrorState()
  const [deleting, setDeleting] = useState(false)
  const [groupDeleteConfirm, setGroupDeleteConfirm] = useState<{
    id: string
    name: string
  } | null>(null)
  const {
    message: groupDeleteError,
    scrollKey: groupDeleteErrorScrollKey,
    set: setGroupDeleteError,
  } = useModalErrorState()
  const [deletingGroup, setDeletingGroup] = useState(false)
  const [removingGroupAccountIds, setRemovingGroupAccountIds] = useState<Set<string>>(new Set())
  const [tagDeleteConfirm, setTagDeleteConfirm] = useState<{
    tag: string
    count: number
  } | null>(null)
  const {
    message: tagDeleteConfirmError,
    scrollKey: tagDeleteConfirmErrorScrollKey,
    set: setTagDeleteConfirmError,
  } = useModalErrorState()
  const [deletingTag, setDeletingTag] = useState(false)

  // Quota Detail Modal
  const [showQuotaModal, setShowQuotaModal] = useState<string | null>(null)
  const [showErrorModal, setShowErrorModal] = useState<string | null>(null)
  const [showVerificationErrorModal, setShowVerificationErrorModal] = useState<string | null>(null)

  // 标签编辑弹窗
  const [showTagModal, setShowTagModal] = useState<string | null>(null)

  // 账号备注弹窗
  const [editingAccountNoteId, setEditingAccountNoteId] = useState<string | null>(null)
  const [oauthAccountNoteMode, setOauthAccountNoteMode] = useState(false)
  const [pendingOAuthAccount, setPendingOAuthAccount] = useState<Account | null>(null)
  const [pendingOAuthEmailInput, setPendingOAuthEmailInput] = useState('')
  const [savingPendingOAuthAccount, setSavingPendingOAuthAccount] = useState(false)
  const [pendingOAuthEmailError, setPendingOAuthEmailError] = useState<string | null>(null)
  const [oauthAccountNoteForm, setOauthAccountNoteForm] = useState<AntigravityAccountNoteFormState>(
    EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM,
  )
  const [editingAccountNoteForm, setEditingAccountNoteForm] = useState<AntigravityAccountNoteFormState>(
    EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM,
  )
  const [savingAccountNote, setSavingAccountNote] = useState(false)
  const [accountNoteSecretVisible, setAccountNoteSecretVisible] = useState(true)
  const [accountNotePasswordVisible, setAccountNotePasswordVisible] = useState(true)
  const [accountNoteCopiedKey, setAccountNoteCopiedKey] = useState<string | null>(null)
  const [accountNoteFieldError, setAccountNoteFieldError] = useState<string | null>(null)
  const [savedMfaRecords, setSavedMfaRecords] = useState<MfaRecord[]>([])
  const [accountNoteMfaPickerOpen, setAccountNoteMfaPickerOpen] = useState(false)
  const [mfaTimeRemaining, setMfaTimeRemaining] = useState(getMfaTimeRemaining)
  const [accountNoteMailPreview, setAccountNoteMailPreview] = useState<AntigravityAccountNoteMailPreviewState | null>(null)
  const [accountNoteMailPreviewLoading, setAccountNoteMailPreviewLoading] = useState(false)
  const [accountNoteMailPreviewError, setAccountNoteMailPreviewError] = useState<string | null>(null)
  const accountNoteMailPreviewSeqRef = useRef(0)
  const accountNoteMailPreviewSnapshotRef = useRef<{ mailUrl: string; code: string } | null>(null)
  const {
    message: accountNoteError,
    scrollKey: accountNoteErrorScrollKey,
    set: setAccountNoteError,
  } = useModalErrorState()
  const editingAccountNoteAccount = useMemo(
    () => accounts.find((account) => account.id === editingAccountNoteId) || null,
    [accounts, editingAccountNoteId]
  )
  const activeAccountNoteForm = oauthAccountNoteMode || pendingOAuthAccount ? oauthAccountNoteForm : editingAccountNoteForm
  const activeAccountNoteEmail = oauthAccountNoteMode
    ? pendingOAuthAccount?.email ?? pendingOAuthEmailInput.trim()
    : editingAccountNoteAccount?.email ?? ''

  const openPendingOAuthAccount = useCallback((account: Account) => {
    setPendingOAuthAccount(account)
    setOauthAccountNoteMode(false)
    setEditingAccountNoteId(null)
    setEditingAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
    setAccountNoteError(null)
    setShowAddModal(true)
    setAddTab('oauth')
    setOauthAccountNoteForm(buildAntigravityAccountNoteForm(account))
    setPendingOAuthEmailInput(account.email)
    setAddStatus('idle')
    setAddMessage('')
  }, [setAccountNoteError])

  const resetAccountNoteMailPreview = useCallback(() => {
    accountNoteMailPreviewSeqRef.current += 1
    accountNoteMailPreviewSnapshotRef.current = null
    setAccountNoteMailPreview(null)
    setAccountNoteMailPreviewError(null)
    setAccountNoteMailPreviewLoading(false)
  }, [])

  const fetchAccountNoteMailPreviewForUrl = useCallback(async (rawUrl: string) => {
    const mailUrl = rawUrl.trim()
    accountNoteMailPreviewSeqRef.current += 1
    const requestSeq = accountNoteMailPreviewSeqRef.current
    setAccountNoteMailPreview(null)
    setAccountNoteMailPreviewError(null)
    if (!mailUrl) {
      accountNoteMailPreviewSnapshotRef.current = null
      setAccountNoteMailPreviewLoading(false)
      return
    }
    setAccountNoteMailPreviewLoading(true)
    try {
      const response = await accountService.fetchAccountNoteMailUrl(mailUrl)
      if (accountNoteMailPreviewSeqRef.current !== requestSeq) return
      const preview = findFirstMailVerificationCode(response.body)
      if (!preview) {
        setAccountNoteMailPreviewError(t('accounts.accountNote.mailPreviewNoCode', '未匹配到连续 6 位验证码'))
        return
      }
      const previous = accountNoteMailPreviewSnapshotRef.current
      const status = previous?.mailUrl === mailUrl
        ? previous.code === preview.code ? 'unchanged' : 'changed'
        : 'initial'
      accountNoteMailPreviewSnapshotRef.current = { mailUrl, code: preview.code }
      setAccountNoteMailPreview({ ...preview, fetchedAt: Date.now(), truncated: response.truncated, status })
    } catch (error) {
      if (accountNoteMailPreviewSeqRef.current !== requestSeq) return
      const rawError = String(error).replace(/^Error:\s*/, '')
      const httpError = rawError.match(/^MAIL_PREVIEW_HTTP_FAILED:(\d+)$/)
      const detail = rawError === 'MAIL_URL_EMPTY'
        ? t('accounts.accountNote.mailPreviewUrlRequired', '请输入邮件地址')
        : rawError === 'MAIL_URL_INVALID'
          ? t('accounts.accountNote.mailPreviewUrlInvalid', '邮件地址格式无效，请输入完整的 http:// 或 https:// 地址')
          : rawError === 'MAIL_URL_UNSUPPORTED_SCHEME'
            ? t('accounts.accountNote.mailPreviewUnsupportedProtocol', '邮件地址仅支持 http 或 https 协议')
            : httpError
              ? t('accounts.accountNote.mailPreviewHttpFailed', { defaultValue: '邮件地址请求失败：HTTP {{status}}', status: httpError[1] })
              : rawError.replace(/^MAIL_PREVIEW_[A-Z_]+:\s*/, '')
      setAccountNoteMailPreviewError(t('accounts.accountNote.mailPreviewFetchFailed', { defaultValue: '读取邮件失败：{{error}}', error: detail }))
    } finally {
      if (accountNoteMailPreviewSeqRef.current === requestSeq) setAccountNoteMailPreviewLoading(false)
    }
  }, [t])

  useEffect(() => {
    const timer = window.setInterval(() => setMfaTimeRemaining(getMfaTimeRemaining()), 1000)
    return () => window.clearInterval(timer)
  }, [])

  const [displayGroups, setDisplayGroups] = useState<DisplayGroup[]>([])
  const [displayGroupsLoaded, setDisplayGroupsLoaded] = useState(false)

  // ─── 账号分组（文件夹）────────────────────────────────────
  const [accountGroups, setAccountGroups] = useState<AccountGroup[]>([])
  const [activeGroupId, setActiveGroupId] = useState<string | null>(() => {
    if (!initialFilterPersistenceEnabled) {
      return null
    }
    const saved = readAccountsOverviewFilterField<string | null>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID,
      null,
    )
    return typeof saved === 'string' && saved.trim() ? saved : null
  })
  const [addTargetGroupId, setAddTargetGroupId] = useState<string | null>(null)
  const [showAccountGroupModal, setShowAccountGroupModal] = useState(false)
  const [showAddToGroupModal, setShowAddToGroupModal] = useState(false)
  const [groupAccountPickerGroupId, setGroupAccountPickerGroupId] = useState<string | null>(null)
  const [groupQuickAddGroupId, setGroupQuickAddGroupId] = useState<string | null>(null)

  const reloadAccountGroups = useCallback(async () => {
    setAccountGroups(await getAccountGroups())
  }, [])

  useEffect(() => {
    reloadAccountGroups()
  }, [reloadAccountGroups])

  const activeGroup = useMemo(() => {
    if (!activeGroupId) return null
    return accountGroups.find((g) => g.id === activeGroupId) || null
  }, [accountGroups, activeGroupId])

  const addTargetGroup = useMemo(() => {
    if (!addTargetGroupId) return null
    return accountGroups.find((group) => group.id === addTargetGroupId) || null
  }, [accountGroups, addTargetGroupId])

  const resolveValidAccountGroupId = useCallback(
    (groupId?: string | null) => {
      const normalized = groupId?.trim()
      if (!normalized) return null
      return accountGroups.some((group) => group.id === normalized) ? normalized : null
    },
    [accountGroups],
  )

  const assignAccountsToAddTargetGroup = useCallback(
    async (
      targetAccounts: Array<Account | null | undefined>,
      targetGroupId = addTargetGroupId,
    ) => {
      const resolvedGroupId = resolveValidAccountGroupId(targetGroupId)
      if (!resolvedGroupId) return

      const accountIds = Array.from(
        new Set(
          targetAccounts
            .map((account) => account?.id?.trim())
            .filter((id): id is string => Boolean(id)),
        ),
      )
      if (accountIds.length === 0) return

      await assignAccountsToGroup(resolvedGroupId, accountIds)
      await reloadAccountGroups()
    },
    [addTargetGroupId, reloadAccountGroups, resolveValidAccountGroupId],
  )

  const groupAccountPickerGroup = useMemo(() => {
    if (!groupAccountPickerGroupId) return null
    return accountGroups.find((group) => group.id === groupAccountPickerGroupId) || null
  }, [accountGroups, groupAccountPickerGroupId])

  const groupQuickAddGroup = useMemo(() => {
    if (!groupQuickAddGroupId) return null
    return accountGroups.find((group) => group.id === groupQuickAddGroupId) || null
  }, [accountGroups, groupQuickAddGroupId])

  // 离开已删除的分组
  useEffect(() => {
    if (activeGroupId && !accountGroups.find((g) => g.id === activeGroupId)) {
      setActiveGroupId(null)
    }
  }, [accountGroups, activeGroupId])

  useEffect(() => {
    if (groupQuickAddGroupId && !accountGroups.find((group) => group.id === groupQuickAddGroupId)) {
      setGroupQuickAddGroupId(null)
    }
  }, [accountGroups, groupQuickAddGroupId])
  const [sortBy, setSortBy] = useState<string>(() => {
    if (readAntigravityCustomSortActive()) {
      return 'custom'
    }
    if (!initialFilterPersistenceEnabled) {
      return DEFAULT_ANTIGRAVITY_SORT_BY
    }
    return normalizeAntigravitySortBy(
      readAccountsOverviewFilterField<unknown>(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_SORT_BY,
        DEFAULT_ANTIGRAVITY_SORT_BY,
      ) as string,
    )
  })
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>(() => {
    if (!initialFilterPersistenceEnabled) {
      return 'desc'
    }
    return normalizeAntigravitySortDirection(
      readAccountsOverviewFilterField<unknown>(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
        'desc',
      ) as string | null,
    )
  })

  const [customSortOrder, setCustomSortOrder] = useState<string[]>(
    readAntigravityCustomSortOrder
  )
  const [showCustomSortModal, setShowCustomSortModal] = useState(false)
  const [draggedCustomSortAccountId, setDraggedCustomSortAccountId] = useState<string | null>(null)
  const [customSortDropTargetId, setCustomSortDropTargetId] = useState<string | null>(null)

  // Compact view model sorting
  const [compactGroupOrder, setCompactGroupOrder] = useState<string[]>([])
  const [draggedGroupId, setDraggedGroupId] = useState<string | null>(null)
  const [hiddenGroups, setHiddenGroups] = useState<Set<string>>(new Set())
  const [groupColors, setGroupColors] = useState<Record<string, number>>({})
  const [showColorPicker, setShowColorPicker] = useState<string | null>(null)
  const [colorPickerPos, setColorPickerPos] = useState<{
    top: number
    left: number
  } | null>(null)

  // Available color options
  const colorOptions = [
    { index: 0, color: '#8b5cf6', name: 'Purple' },
    { index: 1, color: '#3b82f6', name: 'Blue' },
    { index: 2, color: '#14b8a6', name: 'Teal' },
    { index: 3, color: '#f59e0b', name: 'Orange' },
    { index: 4, color: '#ec4899', name: 'Pink' },
    { index: 5, color: '#ef4444', name: 'Red' },
    { index: 6, color: '#22c55e', name: 'Green' },
    { index: 7, color: '#6366f1', name: 'Indigo' }
  ]

  const showAddModalRef = useRef(showAddModal)
  const addTabRef = useRef(addTab)
  const oauthUrlRef = useRef(oauthUrl)
  const addStatusRef = useRef(addStatus)
  const oauthAccountNoteFormRef = useRef(oauthAccountNoteForm)
  const addTargetGroupIdRef = useRef<string | null>(null)
  const verificationHistoryRequestIdRef = useRef(0)
  const colorPickerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    showAddModalRef.current = showAddModal
    addTabRef.current = addTab
    oauthUrlRef.current = oauthUrl
    addStatusRef.current = addStatus
    oauthAccountNoteFormRef.current = oauthAccountNoteForm
    addTargetGroupIdRef.current = addTargetGroupId
  }, [showAddModal, addTab, oauthUrl, addStatus, oauthAccountNoteForm, addTargetGroupId])

  useEffect(() => {
    const handleFeatureUnlockChanged = (event: Event) => {
      const detail = (event as CustomEvent<FeatureUnlockChangedDetail>).detail
      if (!detail || detail.feature !== 'antigravity.seamless_switch') {
        return
      }
      setAntigravitySeamlessSwitchUnlocked(Boolean(detail.unlocked))
    }

    window.addEventListener(FEATURE_UNLOCK_CHANGED_EVENT, handleFeatureUnlockChanged as EventListener)
    return () => {
      window.removeEventListener(
        FEATURE_UNLOCK_CHANGED_EVENT,
        handleFeatureUnlockChanged as EventListener,
      )
    }
  }, [])

  useEffect(() => {
    if (antigravitySeamlessSwitchUnlocked) {
      return
    }
    if (showSwitchHistoryModal) {
      setShowSwitchHistoryModal(false)
    }
    if (switchHistoryClearConfirmOpen) {
      setSwitchHistoryClearConfirmOpen(false)
    }
  }, [antigravitySeamlessSwitchUnlocked, showSwitchHistoryModal, switchHistoryClearConfirmOpen])

  // 获取账号的配额数据 (modelId -> percentage)
  const getAccountQuotas = (account: Account): Record<string, number> => {
    const quotas: Record<string, number> = {}
    if (account.quota?.models) {
      for (const model of account.quota.models) {
        quotas[model.name] = model.percentage
      }
    }
    return quotas
  }

  const getQuotaDisplayItems = (account: Account) =>
    getAntigravityQuotaDisplayItems(account, displayGroups)

  const getAvailableAICreditsDisplay = (account: Account): string => {
    const credits = account.quota?.credits || []
    if (credits.length === 0) return ''

    let total = 0
    let hasValidAmount = false

    for (const credit of credits) {
      if (credit.credit_amount == null) continue
      const parsed = Number.parseFloat(String(credit.credit_amount).replace(/,/g, '').trim())
      if (!Number.isFinite(parsed)) continue
      total += parsed
      hasValidAmount = true
    }

    if (!hasValidAmount) return ''
    return total.toFixed(2).replace(/\.?0+$/, '')
  }

  const loadPersistedOverviewFilters = useCallback(() => {
    const savedViewMode = readAccountsOverviewFilterField<unknown>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_VIEW_MODE,
      'grid',
    )
    if (savedViewMode === 'grid' || savedViewMode === 'list' || savedViewMode === 'compact') {
      setViewMode(savedViewMode)
    }

    const savedFilterTypes = readAccountsOverviewFilterField<unknown>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
      null,
    )
    setFilterTypes(
      savedFilterTypes !== null
        ? (readAccountsOverviewFilterStringArray(
            ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
            ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
          ) as AccountsFilterType[])
        : DEFAULT_FILTER_TYPES
    )

    const savedTagFilter = readAccountsOverviewFilterField<unknown>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
      null,
    )
    setTagFilter(
      savedTagFilter !== null
        ? readAccountsOverviewFilterStringArray(
            ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
            ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
          )
        : DEFAULT_TAG_FILTER
    )

    setGroupByTag(
      Boolean(
        readAccountsOverviewFilterField<unknown>(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
          ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG,
          false,
        ),
      ),
    )

    const savedActiveGroupId = readAccountsOverviewFilterField<string | null>(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID,
      null,
    )
    setActiveGroupId(
      typeof savedActiveGroupId === 'string' && savedActiveGroupId.trim()
        ? savedActiveGroupId
        : null,
    )

    setSortBy(
      normalizeAntigravitySortBy(
        readAccountsOverviewFilterField<unknown>(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
          ANTIGRAVITY_FILTER_FIELD_SORT_BY,
          DEFAULT_ANTIGRAVITY_SORT_BY,
        ) as string,
      ),
    )

    setSortDirection(
      normalizeAntigravitySortDirection(
        readAccountsOverviewFilterField<unknown>(
          ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
          ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
          'desc',
        ) as string | null,
      ),
    )
  }, [])

  const resetOverviewFilters = useCallback(() => {
    setViewMode('grid')
    setFilterTypes([])
    setTagFilter([])
    setGroupByTag(false)
    setActiveGroupId(null)
    setSortBy(DEFAULT_ANTIGRAVITY_SORT_BY)
    setSortDirection('desc')
  }, [])

  useEffect(() => {
    const handleConfigUpdated = () => {
      const nextFilterPersistenceEnabled = readAccountsOverviewFilterPersistenceEnabled(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      )
      setFilterPersistenceEnabled(nextFilterPersistenceEnabled)
      if (nextFilterPersistenceEnabled) {
        loadPersistedOverviewFilters()
      } else {
        resetOverviewFilters()
      }
      setPrivacyModeEnabled(isPrivacyModeEnabledByDefault())
    }

    const handlePrivacyModeChanged = (event: Event) => {
      const isEnabled = (event as CustomEvent<boolean>).detail
      setPrivacyModeEnabled(isEnabled)
    }

    const handleFilterPersistenceChanged = (event: Event) => {
      const detail = (event as CustomEvent<AccountsOverviewFilterPersistenceChangedDetail>).detail
      if (!detail || detail.scope !== ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE) {
        return
      }
      setFilterPersistenceEnabled(Boolean(detail.enabled))
    }
    window.addEventListener('config-updated', handleConfigUpdated)
    window.addEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener)
    window.addEventListener(
      ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
      handleFilterPersistenceChanged as EventListener,
    )
    return () => {
      window.removeEventListener('config-updated', handleConfigUpdated)
      window.removeEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener)
      window.removeEventListener(
        ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
        handleFilterPersistenceChanged as EventListener,
      )
    }
  }, [loadPersistedOverviewFilters, resetOverviewFilters])

  useEffect(() => {
    // Always persist layout mode so switching tabs does not reset list/card view (#1200)
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_VIEW_MODE,
      viewMode,
    )
  }, [viewMode])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_SORT_BY,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_SORT_BY,
      sortBy,
    )
  }, [filterPersistenceEnabled, sortBy])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION,
      sortDirection,
    )
  }, [filterPersistenceEnabled, sortDirection])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES,
      filterTypes,
    )
  }, [filterPersistenceEnabled, filterTypes])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_TAG_FILTER,
      tagFilter,
    )
  }, [filterPersistenceEnabled, tagFilter])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG,
      groupByTag,
    )
  }, [filterPersistenceEnabled, groupByTag])

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(
        ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
        ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID,
      )
      return
    }
    writeAccountsOverviewFilterField(
      ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE,
      ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID,
      activeGroupId,
    )
  }, [activeGroupId, filterPersistenceEnabled])

  useEffect(() => {
    return subscribeUserMemory(() => {
      setCustomSortOrder((prev) =>
        mergeIdListsPreferExisting(readAntigravityCustomSortOrder(), prev),
      )
    })
  }, [])

  // Sync customSortOrder when accounts load or change
  useEffect(() => {
    if (accounts.length === 0) {
      return
    }
    const accountIds = accounts.map((account) => account.id)
    setCustomSortOrder((prev) => {
      const next = [...prev]
      const seen = new Set(next)
      for (const accountId of accountIds) {
        if (!seen.has(accountId)) {
          next.push(accountId)
          seen.add(accountId)
        }
      }
      const unchanged =
        next.length === prev.length &&
        next.every((accountId, index) => accountId === prev[index])
      return unchanged ? prev : next
    })
  }, [accounts])

  useEffect(() => {
    writeAntigravityCustomSortOrder(customSortOrder)
  }, [customSortOrder])

  useEffect(() => {
    writeAntigravityCustomSortActive(sortBy === 'custom')
  }, [sortBy])

  useEffect(() => {
    if (!showCustomSortModal || !draggedCustomSortAccountId) return
    const handleMouseUp = () => {
      setDraggedCustomSortAccountId(null)
      setCustomSortDropTargetId(null)
    }
    window.addEventListener('mouseup', handleMouseUp)
    return () => window.removeEventListener('mouseup', handleMouseUp)
  }, [showCustomSortModal, draggedCustomSortAccountId])

  useEffect(() => {
    if (!showCustomSortModal) {
      setDraggedCustomSortAccountId(null)
      setCustomSortDropTargetId(null)
    }
  }, [showCustomSortModal])

  const isCustomSortActive = sortBy === 'custom'
  const customSortAccounts = useMemo(() => {
    const accountMap = new Map(
      accounts.map((account) => [account.id, account])
    )
    const result: Account[] = []
    const seen = new Set<string>()

    customSortOrder.forEach((accountId) => {
      const account = accountMap.get(accountId)
      if (!account || seen.has(accountId)) return
      result.push(account)
      seen.add(accountId)
    })

    accounts.forEach((account) => {
      if (seen.has(account.id)) return
      result.push(account)
      seen.add(account.id)
    })

    return result
  }, [accounts, customSortOrder])

  const customSortAccountIds = useMemo(
    () => customSortAccounts.map((account) => account.id),
    [customSortAccounts]
  )

  const moveCustomSortAccount = useCallback(
    (accountId: string, direction: 'up' | 'down') => {
      const currentIndex = customSortAccountIds.indexOf(accountId)
      if (currentIndex < 0) return
      const targetIndex =
        direction === 'up' ? currentIndex - 1 : currentIndex + 1
      if (targetIndex < 0 || targetIndex >= customSortAccountIds.length) return
      const next = [...customSortAccountIds]
      const [moved] = next.splice(currentIndex, 1)
      next.splice(targetIndex, 0, moved)
      setCustomSortOrder(next)
    },
    [customSortAccountIds]
  )

  const stopCustomSortDragging = useCallback(() => {
    setDraggedCustomSortAccountId(null)
    setCustomSortDropTargetId(null)
  }, [])

  const handleCustomSortDragStart = useCallback(
    (event: ReactMouseEvent, accountId: string) => {
      if (event.button !== 0) return
      event.preventDefault()
      event.stopPropagation()
      setDraggedCustomSortAccountId(accountId)
      setCustomSortDropTargetId(null)
    },
    []
  )

  const handleCustomSortDragMove = useCallback(
    (targetAccountId: string) => {
      if (!draggedCustomSortAccountId) return
      if (draggedCustomSortAccountId === targetAccountId) {
        setCustomSortDropTargetId(null)
        return
      }
      const fromIndex = customSortAccountIds.indexOf(
        draggedCustomSortAccountId
      )
      const toIndex = customSortAccountIds.indexOf(targetAccountId)
      if (fromIndex < 0 || toIndex < 0) return
      setCustomSortDropTargetId(targetAccountId)
      const next = [...customSortAccountIds]
      const [moved] = next.splice(fromIndex, 1)
      next.splice(toIndex, 0, moved)
      setCustomSortOrder(next)
    },
    [customSortAccountIds, draggedCustomSortAccountId]
  )

  const resetCustomSortOrder = useCallback(() => {
    setCustomSortOrder(accounts.map((account) => account.id))
  }, [accounts])

  const handleSortByChange = useCallback(
    (value: string) => {
      setSortBy(value)
      if (value === 'custom') {
        setShowCustomSortModal(true)
      }
    },
    [setSortBy]
  )

  useEffect(() => {
    if (!displayGroupsLoaded) {
      return
    }
    const normalizedSortBy = normalizeAntigravitySortBy(sortBy)
    if (
      normalizedSortBy === 'overall' ||
      normalizedSortBy === 'created_at' ||
      normalizedSortBy === 'default' ||
      normalizedSortBy === 'custom'
    ) {
      return
    }

    if (normalizedSortBy.startsWith(ANTIGRAVITY_RESET_SORT_PREFIX)) {
      const targetGroupId = normalizedSortBy.slice(ANTIGRAVITY_RESET_SORT_PREFIX.length)
      if (displayGroups.some((group) => group.id === targetGroupId)) {
        return
      }
      setSortBy(DEFAULT_ANTIGRAVITY_SORT_BY)
      return
    }

    if (!displayGroups.some((group) => group.id === normalizedSortBy)) {
      setSortBy(DEFAULT_ANTIGRAVITY_SORT_BY)
    }
  }, [displayGroups, displayGroupsLoaded, sortBy])

  const customSortOrderIndex = useMemo(() => {
    const map = new Map<string, number>()
    customSortOrder.forEach((accountId, index) => {
      map.set(accountId, index)
    })
    return map
  }, [customSortOrder])

  const accountSortComparator = useMemo(
    () =>
      createAntigravityAccountComparator({
        sortBy,
        sortDirection,
        displayGroups,
        currentAccountId: currentAccount?.id ?? null,
        customSortOrderIndex,
      }),
    [currentAccount?.id, displayGroups, sortBy, sortDirection, customSortOrderIndex]
  )

  const availableTags = useMemo(() => collectAvailableAccountTags(accounts), [accounts])

  const isAbnormalAccount = useCallback(
    (account: Account): boolean => {
      const isDisabled = account.disabled
      const isForbidden = Boolean(account.quota?.is_forbidden)
      const hasWarning = Boolean(refreshWarnings[account.email])
      const verificationReason = account.disabled_reason || verificationStatusMap[account.id]
      const hasVerificationIssue =
        verificationReason === 'verification_required' || verificationReason === 'tos_violation'
      return isDisabled || isForbidden || hasWarning || hasVerificationIssue
    },
    [refreshWarnings, verificationStatusMap]
  )

  const validAccountCount = useMemo(
    () => accounts.reduce((count, account) => (isAbnormalAccount(account) ? count : count + 1), 0),
    [accounts, isAbnormalAccount]
  )

  // 筛选后的账号
  const filteredAccounts = useMemo(() => {
    let result = [...accounts]

    // 分组过滤（进入分组后只显示该组的账号）
    if (activeGroup) {
      const groupAccountSet = new Set(activeGroup.accountIds)
      result = result.filter((acc) => groupAccountSet.has(acc.id))
    } else {
      // 主界面：隐藏所有已被归入文件夹的账号
      const allGroupedIds = new Set<string>()
      for (const group of accountGroups) {
        for (const id of group.accountIds) {
          allGroupedIds.add(id)
        }
      }
      if (allGroupedIds.size > 0) {
        result = result.filter((acc) => !allGroupedIds.has(acc.id))
      }
    }

    // 搜索过滤
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase()
      result = result.filter((acc) => acc.email.toLowerCase().includes(query))
    }

    // 类型过滤（多选）
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes)
      if (requireValidAccounts) {
        result = result.filter((acc) => !isAbnormalAccount(acc))
      }
      if (selectedTypes.size > 0) {
        result = result.filter((acc) =>
          accountMatchesTypeFilters(
            acc,
            selectedTypes as Set<AccountFilterType>,
            verificationStatusMap
          )
        )
      }
    }

    // 标签过滤
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeAccountTag))
      result = result.filter((acc) => accountMatchesTagFilters(acc, selectedTags))
    }
    result.sort(accountSortComparator)
    return result
  }, [
    accounts,
    searchQuery,
    filterTypes,
    tagFilter,
    accountSortComparator,
    verificationStatusMap,
    isAbnormalAccount,
    activeGroup,
    accountGroups,
  ])

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>
    const groups = new Map<string, typeof filteredAccounts>()
    const selectedTags = new Set(tagFilter.map(normalizeAccountTag))

    filteredAccounts.forEach((account) => {
      const tags = (account.tags || []).map(normalizeAccountTag).filter(Boolean)
      const matchedTags =
        selectedTags.size > 0
          ? tags.filter((tag) => selectedTags.has(tag))
          : tags

      if (matchedTags.length === 0) {
        if (!groups.has(untaggedKey)) groups.set(untaggedKey, [])
        groups.get(untaggedKey)?.push(account)
        return
      }

      matchedTags.forEach((tag) => {
        if (!groups.has(tag)) groups.set(tag, [])
        groups.get(tag)?.push(account)
      })
    })

    return Array.from(groups.entries()).sort(([aKey], [bKey]) => {
      if (aKey === untaggedKey) return -1
      if (bKey === untaggedKey) return 1
      return aKey.localeCompare(bKey)
    })
  }, [filteredAccounts, groupByTag, tagFilter, untaggedKey])

  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('accounts'),
  })
  const paginatedAccounts = pagination.pageItems
  const paginatedIds = useMemo(
    () => paginatedAccounts.map((account) => account.id),
    [paginatedAccounts]
  )
  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts]
  )
  const allPaginatedSelected = useMemo(
    () => isEveryIdSelected(selected, paginatedIds),
    [paginatedIds, selected]
  )

  const hasVisibleAccountGroups = useMemo(
    () => !activeGroupId && !groupByTag && accountGroups.length > 0,
    [activeGroupId, groupByTag, accountGroups]
  )

  // 统计数量
  const tierCounts = useMemo(
    () => buildAccountTierCounts(accounts, verificationStatusMap),
    [accounts, verificationStatusMap]
  )

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      ...buildAccountTierFilterOptions(t, tierCounts),
      buildValidAccountsFilterOption(t, validAccountCount),
    ],
    [
      t,
      tierCounts.FREE,
      tierCounts.PRO,
      tierCounts.TOS_VIOLATION,
      tierCounts.ULTRA,
      tierCounts.UNKNOWN,
      tierCounts.VERIFICATION_REQUIRED,
      validAccountCount,
    ]
  )

  // 加载显示用分组配置
  const loadDisplayGroups = async () => {
    try {
      const groups = await getDisplayGroups()
      setDisplayGroups(groups)
      // Initialize compact mode group order
      setCompactGroupOrder(groups.map((g) => g.id))

      // Load custom settings from localStorage
      const savedOrder = localStorage.getItem('compactGroupOrder')
      const savedColors = localStorage.getItem('compactGroupColors')
      const savedHidden = localStorage.getItem('compactHiddenGroups')

      if (savedOrder) {
        try {
          const order = JSON.parse(savedOrder)
          // 确保所有分组都在排序中
          const validOrder = order.filter((id: string) =>
            groups.some((g) => g.id === id)
          )
          const missingGroups = groups
            .filter((g) => !validOrder.includes(g.id))
            .map((g) => g.id)
          setCompactGroupOrder([...validOrder, ...missingGroups])
        } catch (e) {
          console.error('Failed to parse saved order:', e)
        }
      }

      if (savedColors) {
        try {
          setGroupColors(JSON.parse(savedColors))
        } catch (e) {
          console.error('Failed to parse saved colors:', e)
        }
      }

      if (savedHidden) {
        try {
          setHiddenGroups(new Set(JSON.parse(savedHidden)))
        } catch (e) {
          console.error('Failed to parse saved hidden groups:', e)
        }
      }
    } catch (e) {
      console.error('Failed to load display groups:', e)
    } finally {
      setDisplayGroupsLoaded(true)
    }
  }

  // 获取按紧凑模式排序后的分组
  const getOrderedDisplayGroups = () => {
    if (compactGroupOrder.length === 0) return displayGroups
    return compactGroupOrder
      .map((id) => displayGroups.find((g) => g.id === id))
      .filter((g): g is DisplayGroup => g !== undefined)
  }

  // 获取模型颜色索引
  const getGroupColorIndex = (groupId: string, fallbackIndex: number) => {
    return groupColors[groupId] ?? fallbackIndex
  }

  // 切换模型显示/隐藏
  const toggleGroupVisibility = (groupId: string) => {
    setHiddenGroups((prev) => {
      const next = new Set(prev)
      if (next.has(groupId)) {
        next.delete(groupId)
      } else {
        next.add(groupId)
      }
      // Save to localStorage
      localStorage.setItem('compactHiddenGroups', JSON.stringify([...next]))
      return next
    })
  }

  // Set group color
  const setGroupColor = (groupId: string, colorIndex: number) => {
    setGroupColors((prev) => {
      const next = { ...prev, [groupId]: colorIndex }
      // Save to localStorage
      localStorage.setItem('compactGroupColors', JSON.stringify(next))
      return next
    })
    setShowColorPicker(null)
    setColorPickerPos(null)
  }

  // Open color picker with position calculation
  const openColorPicker = useCallback(
    (e: React.MouseEvent, groupId: string, isOpen: boolean) => {
      e.stopPropagation()
      if (isOpen) {
        setShowColorPicker(null)
        setColorPickerPos(null)
      } else {
        const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
        setColorPickerPos({
          top: rect.bottom + 6,
          left: rect.left + rect.width / 2
        })
        setShowColorPicker(groupId)
      }
    },
    []
  )

  // Drag-and-drop sorting handler - using mouse events for smooth animation
  const handleDragStart = (e: React.MouseEvent, groupId: string) => {
    e.preventDefault()
    e.stopPropagation()
    setDraggedGroupId(groupId)
  }

  const handleDragMove = (targetGroupId: string) => {
    if (!draggedGroupId || draggedGroupId === targetGroupId) return

    const newOrder = [...compactGroupOrder]
    const draggedIndex = newOrder.indexOf(draggedGroupId)
    const targetIndex = newOrder.indexOf(targetGroupId)

    if (draggedIndex !== -1 && targetIndex !== -1) {
      newOrder.splice(draggedIndex, 1)
      newOrder.splice(targetIndex, 0, draggedGroupId)
      setCompactGroupOrder(newOrder)
    }
  }

  const handleDragEnd = async () => {
    if (draggedGroupId && compactGroupOrder.length > 0) {
      // Persist order to backend and localStorage
      try {
        await updateGroupOrder(compactGroupOrder)
        localStorage.setItem(
          'compactGroupOrder',
          JSON.stringify(compactGroupOrder)
        )
      } catch (e) {
        console.error('Failed to save group order:', e)
      }
    }
    setDraggedGroupId(null)
  }

  useEffect(() => {
    fetchAccounts()
    fetchCurrentAccount(antigravityRuntimeTarget)
    loadDisplayGroups()
    loadVerificationHistory()

    let unlisten: UnlistenFn | undefined

    listen<string>('accounts:refresh', async () => {
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      const latestAccounts = useAccountStore.getState().accounts
      const accountsWithoutQuota = latestAccounts.filter(
        (acc) => !acc.pending_oauth && !acc.quota?.models?.length
      )
      if (accountsWithoutQuota.length > 0) {
        await Promise.allSettled(
          accountsWithoutQuota.map((acc) => refreshQuota(acc.id, antigravityRuntimeTarget))
        )
        await fetchAccounts()
      }
      await loadVerificationHistory()
    }).then((fn) => {
      unlisten = fn
    })

    return () => {
      if (unlisten) unlisten()
    }
  }, [fetchAccounts, fetchCurrentAccount, loadVerificationHistory, refreshQuota])

  // Click outside to close color picker
  useEffect(() => {
    if (!showColorPicker) return

    const handleClickOutside = (e: MouseEvent) => {
      if (
        colorPickerRef.current &&
        !colorPickerRef.current.contains(e.target as Node)
      ) {
        setShowColorPicker(null)
        setColorPickerPos(null)
      }
    }

    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [showColorPicker])

  useEffect(() => {
    let unlistenUrl: UnlistenFn | undefined
    let unlistenCallback: UnlistenFn | undefined

    listen<string>('oauth-url-generated', (event) => {
      setOauthUrl(String(event.payload || ''))
    }).then((fn) => {
      unlistenUrl = fn
    })

    listen('oauth-callback-received', async () => {
      if (!showAddModalRef.current) return
      if (addTabRef.current !== 'oauth') return
      if (addStatusRef.current === 'loading') return
      if (!oauthUrlRef.current) return

      setOauthCallbackSubmitting(false)
      setOauthCallbackError(null)
      setAddStatus('loading')
      setAddMessage(t('accounts.oauth.authorizing'))
      try {
        const newAccount = await accountService.completeOAuthLogin(
          buildAntigravityAccountNoteUpdate(oauthAccountNoteFormRef.current),
        )
        await fetchAccounts()
        await fetchCurrentAccount(antigravityRuntimeTarget)
        await assignAccountsToAddTargetGroup([newAccount], addTargetGroupIdRef.current)
        setAddStatus('success')
        setAddMessage(t('accounts.oauth.success'))
        setTimeout(() => {
          setShowAddModal(false)
          setAddStatus('idle')
          setAddMessage('')
          setOauthUrl('')
        }, 1200)
      } catch (e) {
        setAddStatus('error')
        setAddMessage(t('accounts.oauth.failed', { error: String(e) }))
      }
    }).then((fn) => {
      unlistenCallback = fn
    })

    return () => {
      if (unlistenUrl) unlistenUrl()
      if (unlistenCallback) unlistenCallback()
    }
  }, [assignAccountsToAddTargetGroup, fetchAccounts, fetchCurrentAccount])

  useEffect(() => {
    if (!showAddModal || addTab !== 'oauth' || oauthUrl) return
    accountService
      .prepareOAuthUrl()
      .then((url) => {
        if (typeof url === 'string' && url.length > 0) {
          setOauthUrl(url)
          setOauthCallbackError(null)
        }
      })
      .catch((e) => {
        console.error('准备 OAuth 链接失败:', e)
      })
  }, [showAddModal, addTab, oauthUrl])

  useEffect(() => {
    if (showAddModal && addTab === 'oauth') return
    if (!oauthUrl) return
    accountService.cancelOAuthLogin().catch(() => { })
    setOauthUrl('')
    setOauthUrlCopied(false)
  }, [showAddModal, addTab, oauthUrl])

  useEffect(() => {
    return () => {
      if (!showAddModalRef.current || addTabRef.current !== 'oauth') return
      accountService.cancelOAuthLogin().catch(() => { })
    }
  }, [])

  const handleRefresh = async (accountId: string) => {
    setRefreshing((prev) => new Set(prev).add(accountId))
    try {
      await refreshQuota(accountId, antigravityRuntimeTarget)
      setRefreshResult((prev) => ({ ...prev, [accountId]: 'success' }))
      setTimeout(() => setRefreshResult((prev) => { const next = { ...prev }; delete next[accountId]; return next }), 2000)
    } catch (e) {
      console.error(e)
      setRefreshResult((prev) => ({ ...prev, [accountId]: 'error' }))
      setTimeout(() => setRefreshResult((prev) => { const next = { ...prev }; delete next[accountId]; return next }), 2000)
    } finally {
      await loadVerificationHistory()
      setRefreshing((prev) => { const next = new Set(prev); next.delete(accountId); return next })
    }
  }

  const handleRefreshAll = async () => {
    setRefreshingAll(true)
    try {
      if (activeGroup) {
        // 分组内刷新：只刷新该组的账号
        const groupAccountIds = new Set(activeGroup.accountIds)
        const groupAccounts = accounts.filter((acc) => groupAccountIds.has(acc.id))
        await Promise.allSettled(
          groupAccounts.map((acc) => refreshQuota(acc.id, antigravityRuntimeTarget))
        )
      } else {
        const stats = await refreshAllQuotas()
        setRefreshWarnings(buildWarningMapFromDetails(stats.details || []))
      }
    } catch (e) {
      console.error(e)
    } finally {
      await loadVerificationHistory()
      setRefreshingAll(false)
    }
  }

  const handleWakeupSelected = async () => {
    if (selected.size === 0 || wakeupRunning) return
    setWakeupRunning(true)
    setMessage(null)
    const selectedIdSet = new Set(selected)
    const selectedAccounts = accounts.filter((account) => selectedIdSet.has(account.id))
    try {
      const models = await invoke<Array<{ id: string }>>('fetch_available_models')
      const model = models.find((item) => item.id)?.id
      if (!model) {
        throw new Error(t('wakeup.notice.testMissingModel'))
      }
      const officialLsVersionMode = loadWakeupOfficialLsVersionMode()
      const results = await Promise.allSettled(
        selectedAccounts.map((account) =>
          invoke('trigger_wakeup', {
            accountId: account.id,
            model,
            prompt: undefined,
            maxOutputTokens: 0,
            cancelScopeId: undefined,
            officialLsVersionMode,
          }),
        ),
      )
      const failed = results.filter((result) => result.status === 'rejected').length
      const success = results.length - failed
      setMessage({
        text:
          failed > 0
            ? t('messages.actionFailed', {
                action: t('wakeup.runTest'),
                error: `${success}/${results.length}`,
              })
            : t('messages.actionSuccess', { action: t('wakeup.runTest') }),
        tone: failed > 0 ? 'error' : undefined,
      })
    } catch (error) {
      setMessage({
        text: t('messages.actionFailed', {
          action: t('wakeup.runTest'),
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      })
    } finally {
      setWakeupRunning(false)
    }
  }

  const handleDelete = (accountId: string) => {
    setDeleteConfirmError(null)
    setDeleteConfirm({
      ids: [accountId],
      message: t('messages.deleteConfirm')
    })
  }

  const handleBatchDelete = () => {
    if (selected.size === 0) return
    setDeleteConfirmError(null)
    setDeleteConfirm({
      ids: Array.from(selected),
      message: t('messages.batchDeleteConfirm', { count: selected.size })
    })
  }

  const confirmDelete = async () => {
    if (!deleteConfirm || deleting) return
    setDeleting(true)
    setDeleteConfirmError(null)
    try {
      await deleteAccounts(deleteConfirm.ids)
      void removeAccountIdsFromAllGroups(deleteConfirm.ids)
      setCustomSortOrder((prev) =>
        prev.filter((accountId) => !deleteConfirm.ids.includes(accountId)),
      )
      setSelected((prev) => {
        if (prev.size === 0) return prev
        const next = new Set(prev)
        deleteConfirm.ids.forEach((id) => next.delete(id))
        return next
      })
      setDeleteConfirm(null)
      setDeleteConfirmError(null)
      // 删除成功后清掉页顶红色报错（#1160）
      setMessage(null)
    } catch (error) {
      setDeleteConfirmError(
        t('messages.actionFailed', {
          action: t('common.delete'),
          error: String(error),
        })
      )
    } finally {
      setDeleting(false)
    }
  }

  const resetAddModalState = useCallback(() => {
    setAddStatus('idle')
    setAddMessage('')
    setTokenInput('')
    setOauthUrlCopied(false)
    setOauthCallbackInput('')
    setOauthCallbackSubmitting(false)
    setOauthCallbackError(null)
  }, [])

  const openAddModal = useCallback((tab: 'oauth' | 'token' | 'import') => {
    setAddTargetGroupId(resolveValidAccountGroupId(activeGroupId))
    setAddTab(tab)
    setShowAddModal(true)
    setPendingOAuthAccount(null)
    setPendingOAuthEmailInput('')
    setOauthAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
    setPendingOAuthEmailError(null)
    resetAddModalState()
  }, [activeGroupId, resetAddModalState, resolveValidAccountGroupId])

  const consumeExternalProviderImport = useCallback(() => {
    const request = consumeQueuedExternalProviderImportForPlatform('antigravity')
    if (!request) {
      console.info('[ExternalImport][AccountsPage] 当前无 antigravity 待处理导入请求')
      return
    }
    console.info('[ExternalImport][AccountsPage] 消费到导入请求，准备打开导入弹框', {
      page: request.page,
      autoImport: request.autoImport,
      tokenLength: request.token.length,
      source: request.source ?? null,
    })
    openAddModal('token')
    const normalizedTokenInput = normalizeAntigravityExternalImportToken(request.token)
    setTokenInput(normalizedTokenInput)
    setAddStatus('idle')
    setAddMessage('')
    console.info('[ExternalImport][AccountsPage] 已打开导入弹框并写入 tokenInput', {
      normalizedLength: normalizedTokenInput.length,
      normalizedLooksLikeJson: normalizedTokenInput.trim().startsWith('{'),
    })
  }, [openAddModal])

  useEffect(() => {
    const handleExternalImportEvent = () => {
      console.info('[ExternalImport][AccountsPage] 收到前端外部导入事件')
      consumeExternalProviderImport()
    }
    console.info('[ExternalImport][AccountsPage] 初始化时尝试消费外部导入队列')
    consumeExternalProviderImport()
    window.addEventListener(EXTERNAL_PROVIDER_IMPORT_EVENT, handleExternalImportEvent)
    return () => {
      window.removeEventListener(EXTERNAL_PROVIDER_IMPORT_EVENT, handleExternalImportEvent)
    }
  }, [consumeExternalProviderImport])

  const closeAddModal = () => {
    if (addStatus === 'loading') {
      accountService.cancelOAuthLogin().catch(() => { })
    }
    setShowAddModal(false)
    setAddTargetGroupId(null)
    resetAddModalState()
    setOauthUrl('')
    setPendingOAuthAccount(null)
    setPendingOAuthEmailInput('')
    setPendingOAuthEmailError(null)
    setOauthAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
  }

  useEscClose(showAddModal, closeAddModal);
  useEscClose(showSwitchHistoryModal, () => setShowSwitchHistoryModal(false));
  useEscClose(Boolean(deleteConfirm) && !deleting, () => {
    setDeleteConfirm(null)
    setDeleteConfirmError(null)
  });
  useEnterConfirm(Boolean(deleteConfirm) && !deleting, () => {
    void confirmDelete()
  });
  useEscClose(Boolean(groupDeleteConfirm) && !deletingGroup, () => {
    setGroupDeleteConfirm(null)
    setGroupDeleteError(null)
  });
  useEnterConfirm(Boolean(groupDeleteConfirm) && !deletingGroup, () => {
    void confirmDeleteGroup()
  });
  useEscClose(Boolean(tagDeleteConfirm) && !deletingTag, () => {
    setTagDeleteConfirm(null)
    setTagDeleteConfirmError(null)
  });
  useEnterConfirm(Boolean(tagDeleteConfirm) && !deletingTag, () => {
    void confirmDeleteTag()
  });

  const runModalAction = async (
    label: string,
    action: () => Promise<void>,
    closeOnSuccess = true
  ) => {
    setAddStatus('loading')
    setAddMessage(t('messages.actionRunning', { action: label }))
    try {
      await action()
      setAddStatus('success')
      setAddMessage(t('messages.actionSuccess', { action: label }))
      if (closeOnSuccess) {
        setTimeout(() => {
          setShowAddModal(false)
          resetAddModalState()
        }, 1200)
      }
    } catch (e) {
      setAddStatus('error')
      setAddMessage(
        t('messages.actionFailed', { action: label, error: String(e) })
      )
    }
  }

  const handleOAuthStart = async () => {
    await runModalAction(t('modals.import.oauthAction'), async () => {
      const account = await startOAuthLogin(buildAntigravityAccountNoteUpdate(oauthAccountNoteForm))
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      await assignAccountsToAddTargetGroup([account])
    })
  }

  const handleOAuthComplete = async () => {
    await runModalAction(t('modals.import.oauthAction'), async () => {
      const account = await accountService.completeOAuthLogin(
        buildAntigravityAccountNoteUpdate(oauthAccountNoteForm),
      )
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      await assignAccountsToAddTargetGroup([account])
    })
  }

  const handleSavePendingOAuthAccount = async () => {
    if (savingPendingOAuthAccount) return
    const email = pendingOAuthEmailInput.trim()
    setPendingOAuthEmailError(null)
    if (!email || !email.includes('@')) {
      setPendingOAuthEmailError(t('codex.pendingAuth.emailRequired', '请输入账号邮箱'))
      return
    }
    const rawSecret = oauthAccountNoteForm.twoFactorSecret.trim()
    const parsedSecret = rawSecret ? parseMfaCredentialInput(rawSecret) : null
    if (rawSecret && !parsedSecret) {
      setAccountNoteFieldError(t('accounts.accountNote.twoFactorSecretInvalid', '2FA 秘钥格式无效，请输入 Base32 secret 或 otpauth:// 链接'))
      openOAuthAccountNoteModal()
      return
    }
    setSavingPendingOAuthAccount(true)
    setAddStatus('loading')
    setAddMessage(t('codex.pendingAuth.saving', '正在保存待授权账号...'))
    try {
      const account = await accountService.createPendingOAuthAccount(email, {
        ...buildAntigravityAccountNoteUpdate(oauthAccountNoteForm),
        twoFactorSecret: parsedSecret?.secret ?? rawSecret,
      })
      setOauthAccountNoteForm((previous) => ({
        ...previous,
        twoFactorSecret: parsedSecret?.secret ?? rawSecret,
      }))
      await fetchAccounts()
      await assignAccountsToAddTargetGroup([account])
      setPendingOAuthAccount(account)
      setAddStatus('success')
      setAddMessage(t('codex.pendingAuth.saved', '待授权账号已保存'))
      window.setTimeout(() => {
        setShowAddModal(false)
        resetAddModalState()
        setOauthUrl('')
        setPendingOAuthAccount(null)
        setPendingOAuthEmailInput('')
        setOauthAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
      }, 900)
    } catch (error) {
      setAddStatus('error')
      setAddMessage(t('codex.pendingAuth.saveFailed', { defaultValue: '保存待授权账号失败：{{error}}', error: String(error).replace(/^Error:\s*/, '') }))
    } finally {
      setSavingPendingOAuthAccount(false)
    }
  }

  const handlePendingOAuthStart = async () => {
    if (!pendingOAuthAccount) return
    await runModalAction(t('modals.import.oauthAction'), async () => {
      const account = await accountService.startOAuthLogin(buildAntigravityAccountNoteUpdate(oauthAccountNoteForm))
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      await assignAccountsToAddTargetGroup([account])
    })
  }

  const handlePendingOAuthComplete = async () => {
    if (!pendingOAuthAccount) return
    await runModalAction(t('modals.import.oauthAction'), async () => {
      const account = await accountService.completeOAuthLogin(buildAntigravityAccountNoteUpdate(oauthAccountNoteForm))
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      await assignAccountsToAddTargetGroup([account])
    })
  }

  const handleSwitch = async (accountId: string) => {
    setMessage(null)
    const targetAccount = accounts.find((account) => account.id === accountId)
    if (isPendingAntigravityAccount(targetAccount)) {
      if (targetAccount) openPendingOAuthAccount(targetAccount)
      return
    }
    setSwitching(accountId)
    try {
      const account = await switchAccount(accountId, antigravityRuntimeTarget)
      await fetchCurrentAccount(antigravityRuntimeTarget)
      setMessage({ text: t('messages.switched', { email: maskAccountText(account.email) }) })
    } catch (e) {
      const raw = formatSwitchError(e)
      if (!raw.startsWith('APP_PATH_NOT_FOUND:')) {
        setMessage({
          text: t('messages.switchFailed', { error: raw }),
          tone: 'error'
        })
      }
    }
    setSwitching(null)
  }

  const loadSwitchHistory = useCallback(async () => {
    setSwitchHistoryLoading(true)
    try {
      const items = await accountService.loadAntigravitySwitchHistory()
      setSwitchHistory(items)
    } catch (error) {
      setMessage({
        text: t('accounts.switchHistory.loadFailed', { error: String(error) }),
        tone: 'error',
      })
    } finally {
      setSwitchHistoryLoading(false)
    }
  }, [t])

  const openSwitchHistoryModal = async () => {
    if (!antigravitySeamlessSwitchUnlocked) {
      return
    }
    setShowSwitchHistoryModal(true)
    setSwitchHistoryClearConfirmOpen(false)
    await loadSwitchHistory()
  }

  const handleClearSwitchHistory = () => {
    if (switchHistoryClearing || switchHistoryLoading || switchHistory.length === 0) {
      return
    }
    setSwitchHistoryClearConfirmOpen(true)
  }

  const confirmClearSwitchHistory = async () => {
    setSwitchHistoryClearing(true)
    try {
      await accountService.clearAntigravitySwitchHistory()
      setSwitchHistory([])
      setSwitchHistoryClearConfirmOpen(false)
    } catch (error) {
      setSwitchHistoryClearConfirmOpen(false)
      setMessage({
        text: t('accounts.switchHistory.clearFailed', { error: String(error) }),
        tone: 'error',
      })
    } finally {
      setSwitchHistoryClearing(false)
    }
  }

  const formatSwitchHistoryStage = (stage?: string | null) => {
    if (stage === 'local') {
      return t('accounts.switchHistory.stageLocal', '本地落盘')
    }
    if (stage === 'client_start') {
      return t('accounts.switchHistory.stageClientStart', '启动客户端')
    }
    if (stage === 'seamless') {
      return t('accounts.switchHistory.stageSeamless', '扩展无感')
    }
    return t('accounts.switchHistory.stageUnknown', '未知阶段')
  }

  const formatSwitchHistoryTrigger = (triggerType?: string | null) => {
    if (triggerType === 'auto') {
      return t('accounts.switchHistory.triggerAuto', '自动切换')
    }
    if (triggerType === 'manual') {
      return t('accounts.switchHistory.triggerManual', '手动切换')
    }
    return t('accounts.switchHistory.triggerUnknown', '未知')
  }

  const formatSwitchHistoryOrigin = (triggerSource?: string | null) => {
    const normalizedSource = (triggerSource || '').trim().toLowerCase()
    if (normalizedSource.startsWith('tools.ws.')) {
      return t('accounts.switchHistory.originPlugin', '插件端')
    }
    if (normalizedSource.startsWith('tools.account.')) {
      return t('accounts.switchHistory.originDesktop', '桌面端')
    }
    return t('accounts.switchHistory.originUnknown', '未知')
  }

  const formatSwitchHistoryAutoRule = (rule?: string | null) => {
    if (rule === 'current_disabled') {
      return t('accounts.switchHistory.autoReasonRuleCurrentDisabled', '当前账号已禁用')
    }
    if (rule === 'current_quota_forbidden') {
      return t('accounts.switchHistory.autoReasonRuleQuotaForbidden', '当前账号配额受限')
    }
    if (rule === 'group_and_credits_below_threshold') {
      return t('accounts.switchHistory.autoReasonRuleGroupAndCreditsBelowThreshold', '模型分组和 Credits 同时低于阈值')
    }
    if (rule === 'group_below_threshold') {
      return t('accounts.switchHistory.autoReasonRuleGroupBelowThreshold', '模型分组低于阈值')
    }
    if (rule === 'credits_below_threshold') {
      return t('accounts.switchHistory.autoReasonRuleCreditsBelowThreshold', '剩余 Credits 低于阈值')
    }
    return t('accounts.switchHistory.triggerUnknown', '未知')
  }

  const formatSwitchHistoryAutoScope = (scopeMode?: string | null) => {
    if (scopeMode === 'selected_groups') {
      return t('accounts.switchHistory.autoReasonScopeSelectedGroups', '指定模型分组')
    }
    return t('accounts.switchHistory.autoReasonScopeAnyGroup', '任一模型分组')
  }

  const formatSwitchHistoryAutoReason = (
    reason?: accountService.AntigravityAutoSwitchReason | null
  ) => {
    if (!reason) {
      return t('accounts.switchHistory.autoReasonUnknown', '自动切号触发，未记录详细原因')
    }
    const formatCreditsValue = (value?: number | null) => {
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return '-'
      }
      return value.toFixed(2).replace(/\.?0+$/, '')
    }
    const hitGroupText = (reason.hitGroups || [])
      .map((group) => `${group.groupName}=${group.percentage}%`)
      .join('、')
    const selectedGroupText = (reason.selectedGroupNames || []).join('、')
    const creditsThresholdText =
      reason.creditsEnabled && typeof reason.creditsThreshold === 'number'
        ? String(reason.creditsThreshold)
        : '-'
    const currentCreditsText = reason.creditsEnabled
      ? formatCreditsValue(reason.currentCreditsRemaining)
      : '-'
    return t('accounts.switchHistory.autoReason', {
      rule: formatSwitchHistoryAutoRule(reason.rule),
      quotaThreshold: reason.threshold,
      creditsThreshold: creditsThresholdText,
      scope: formatSwitchHistoryAutoScope(reason.scopeMode),
      selectedGroups: selectedGroupText || '-',
      hitGroups: hitGroupText || '-',
      currentCredits: currentCreditsText,
      candidates: reason.candidateCount ?? 0,
      defaultValue:
        '规则：{{rule}}；额度阈值：{{quotaThreshold}}%；Credits 阈值：{{creditsThreshold}}；范围：{{scope}}；监控分组：{{selectedGroups}}；命中分组：{{hitGroups}}；当前 Credits：{{currentCredits}}；候选账号：{{candidates}}',
    })
  }

  const handleImportFromTools = async () => {
    setImporting(true)
    setAddStatus('loading')
    setAddMessage(t('modals.import.importingTools'))
    try {
      const imported = await accountService.importFromOldTools()
      await fetchAccounts()
      await Promise.allSettled(imported.map((acc) => refreshQuota(acc.id, antigravityRuntimeTarget)))
      await fetchAccounts()
      await assignAccountsToAddTargetGroup(imported)
      if (imported.length === 0) {
        setAddStatus('error')
        setAddMessage(t('modals.import.noAccountsFound'))
      } else {
        setAddStatus('success')
        setAddMessage(t('messages.importSuccess', { count: imported.length }))
        setTimeout(() => {
          setShowAddModal(false)
          resetAddModalState()
        }, 1200)
      }
    } catch (e) {
      setAddStatus('error')
      setAddMessage(t('messages.importFailed', { error: String(e) }))
    }
    setImporting(false)
  }

  const handleImportFromLocal = async () => {
    setImporting(true)
    setAddStatus('loading')
    setAddMessage(t('modals.import.importingLocal'))
    try {
      const imported = await accountService.importFromLocal()
      await fetchAccounts()
      await new Promise((resolve) => setTimeout(resolve, 180))
      await fetchAccounts()
      await refreshQuota(imported.id, antigravityRuntimeTarget)
      await fetchAccounts()
      await assignAccountsToAddTargetGroup([imported])
      setAddStatus('success')
      setAddMessage(
        t('messages.importLocalSuccess', { email: maskAccountText(imported.email) })
      )
      setTimeout(() => {
        setShowAddModal(false)
        resetAddModalState()
      }, 1200)
    } catch (e) {
      setAddStatus('error')
      setAddMessage(t('messages.importFailed', { error: String(e) }))
    }
    setImporting(false)
  }

  const handleImportFromFiles = async () => {
    let unlistenProgress: UnlistenFn | undefined
    try {
      const selected = await openFileDialog({
        multiple: true,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (!selected || (Array.isArray(selected) && selected.length === 0)) return
      const paths = Array.isArray(selected) ? selected : [selected]
      setImporting(true)
      setAddStatus('loading')
      setAddMessage(t('modals.import.importingFiles', { count: paths.length }))

      unlistenProgress = await listen<{ current: number; total: number; email: string }>(
        'accounts:file-import-progress',
        (event) => {
          const { current, total, email } = event.payload ?? {}
          if (current > 0 && total > 0) {
            const label = email ? ` ${email}` : ''
            setAddMessage(`${t('modals.import.importingFiles', { count: total })} ${current}/${total}${label}`)
          }
        }
      )

      const result = await accountService.importFromFiles(paths)
      const { imported, failed } = result
      await fetchAccounts()
      await Promise.allSettled(imported.map((acc) => refreshQuota(acc.id, antigravityRuntimeTarget)))
      await fetchAccounts()
      await assignAccountsToAddTargetGroup(imported)
      if (imported.length === 0 && failed.length === 0) {
        setAddStatus('error')
        setAddMessage(t('modals.import.noAccountsFound'))
      } else if (failed.length > 0) {
        // 有失败的，显示失败列表，不自动关闭弹窗
        const failedList = failed.map((f) => f.email).join(', ')
        setAddStatus(imported.length > 0 ? 'success' : 'error')
        setAddMessage(
          `${t('messages.importSuccess', { count: imported.length })}，${t('messages.importPartialFailed', { failCount: failed.length, failList: failedList })}`
        )
      } else {
        setAddStatus('success')
        setAddMessage(t('messages.importSuccess', { count: imported.length }))
        setTimeout(() => {
          setShowAddModal(false)
          resetAddModalState()
        }, 1200)
      }
    } catch (e) {
      setAddStatus('error')
      setAddMessage(t('messages.importFailed', { error: String(e) }))
    } finally {
      if (unlistenProgress) {
        unlistenProgress()
      }
      setImporting(false)
    }
  }

  const handleImportFromExtension = async () => {
    setImporting(true)
    setAddStatus('loading')
    setAddMessage(t('modals.import.importingExtension'))
    let unlistenProgress: UnlistenFn | undefined
    try {
      const knownAccountIds = new Set(accounts.map((account) => account.id))
      unlistenProgress = await listen<ExtensionImportProgressPayload>(
        'accounts:extension-import-progress',
        (event) => {
          const payload = event.payload ?? {}
          const current = Number(payload.current ?? 0)
          const total = Number(payload.total ?? 0)
          if (current > 0 && total > 0) {
            setAddMessage(
              t('accounts.token.importProgress', {
                current,
                total
              })
            )
          }
        }
      )
      const count = await accountService.syncFromExtension()
      await fetchAccounts()
      await fetchCurrentAccount(antigravityRuntimeTarget)
      if (count > 0) {
        const imported = (await accountService.listAccounts()).filter(
          (account) => !knownAccountIds.has(account.id),
        )
        await assignAccountsToAddTargetGroup(imported)
      }
      if (count === 0) {
        setAddStatus('error')
        setAddMessage(t('modals.import.noAccountsFound'))
      } else {
        setAddStatus('success')
        setAddMessage(t('messages.importSuccess', { count }))
        setTimeout(() => {
          setShowAddModal(false)
          resetAddModalState()
        }, 1200)
      }
    } catch (e) {
      setAddStatus('error')
      setAddMessage(t('messages.importFailed', { error: String(e) }))
    } finally {
      if (unlistenProgress) {
        unlistenProgress()
      }
      setImporting(false)
    }
  }

  const extractRefreshTokens = (input: string) => {
    const tokens: string[] = []
    const trimmed = input.trim()
    if (!trimmed) return tokens

    try {
      const parsed = JSON.parse(trimmed)
      const pushToken = (value: unknown) => {
        if (typeof value === 'string' && value.startsWith('1//')) {
          tokens.push(value)
        }
      }

      if (Array.isArray(parsed)) {
        parsed.forEach((item) => {
          if (typeof item === 'string') {
            pushToken(item)
            return
          }
          if (item && typeof item === 'object') {
            const token =
              (item as { refresh_token?: string; refreshToken?: string })
                .refresh_token ||
              (item as { refresh_token?: string; refreshToken?: string })
                .refreshToken
            pushToken(token)
          }
        })
      } else if (parsed && typeof parsed === 'object') {
        const token =
          (parsed as { refresh_token?: string; refreshToken?: string })
            .refresh_token ||
          (parsed as { refresh_token?: string; refreshToken?: string })
            .refreshToken
        pushToken(token)
      }
    } catch {
      // ignore JSON parse errors, fallback to regex
    }

    if (tokens.length === 0) {
      const matches = trimmed.match(/1\/\/[a-zA-Z0-9_\-]+/g)
      if (matches) tokens.push(...matches)
    }

    return Array.from(new Set(tokens))
  }

  const handleTokenImport = async () => {
    const trimmedInput = tokenInput.trim()
    const quickNoteLines = trimmedInput.split(/\r?\n/).map((line) => line.trim()).filter((line) => line.includes('----'))
    if (quickNoteLines.length > 0 && !trimmedInput.startsWith('{') && !trimmedInput.startsWith('[')) {
        setImporting(true)
        setAddStatus('loading')
        try {
          const imported = await accountService.importFromJson(trimmedInput)
          await fetchAccounts()
          await assignAccountsToAddTargetGroup(imported)
          setAddStatus('success')
          setAddMessage(t('messages.importSuccess', { count: imported.length }))
        } catch (error) {
          setAddStatus('error')
          setAddMessage(t('messages.importFailed', { error: String(error) }))
        } finally {
          setImporting(false)
        }
        return
    }
    if (trimmedInput.startsWith('{') || trimmedInput.startsWith('[')) {
      setImporting(true)
      setAddStatus('loading')
      try {
        const importedAccounts = await accountService.importFromJson(trimmedInput)
        await Promise.allSettled(
        importedAccounts.filter((account) => !account.pending_oauth).map((account) => refreshQuota(account.id, antigravityRuntimeTarget)),
        )
        await fetchAccounts()
        await assignAccountsToAddTargetGroup(importedAccounts)
        setAddStatus('success')
        setAddMessage(t('accounts.token.importSuccess', { count: importedAccounts.length }))
        if (importedAccounts.length > 0) {
          window.setTimeout(() => {
            setShowAddModal(false)
            resetAddModalState()
          }, 1200)
        }
      } catch (error) {
        setAddStatus('error')
        setAddMessage(t('messages.importFailed', { error: String(error) }))
      } finally {
        setImporting(false)
      }
      return
    }

    const tokens = extractRefreshTokens(tokenInput)
    if (tokens.length === 0) {
      setAddStatus('error')
      setAddMessage(t('accounts.token.invalid'))
      return
    }

    setImporting(true)
    setAddStatus('loading')
    let success = 0
    let fail = 0
    const importedAccounts: Account[] = []

    for (let i = 0; i < tokens.length; i += 1) {
      setAddMessage(
        t('accounts.token.importProgress', {
          current: i + 1,
          total: tokens.length
        })
      )
      try {
        const account = await accountService.addAccountWithToken(tokens[i])
        importedAccounts.push(account)
        success += 1
      } catch (e) {
        console.error('Token 导入失败:', e)
        fail += 1
      }
      await new Promise((resolve) => setTimeout(resolve, 120))
    }

    if (importedAccounts.length > 0) {
      await Promise.allSettled(
        importedAccounts.filter((acc) => !acc.pending_oauth).map((acc) => refreshQuota(acc.id, antigravityRuntimeTarget))
      )
      await fetchAccounts()
      await assignAccountsToAddTargetGroup(importedAccounts)
    }

    if (success === tokens.length) {
      setAddStatus('success')
      setAddMessage(t('accounts.token.importSuccess', { count: success }))
      setTimeout(() => {
        setShowAddModal(false)
        resetAddModalState()
      }, 1200)
    } else if (success > 0) {
      setAddStatus('success')
      setAddMessage(t('accounts.token.importPartial', { success, fail }))
    } else {
      setAddStatus('error')
      setAddMessage(t('accounts.token.importFailed'))
    }

    setImporting(false)
  }

  const handleCopyOauthUrl = async () => {
    if (!oauthUrl) return
    try {
      await navigator.clipboard.writeText(oauthUrl)
      setOauthUrlCopied(true)
      window.setTimeout(() => setOauthUrlCopied(false), 1200)
    } catch (e) {
      console.error('复制失败:', e)
    }
  }

  const handleSubmitOauthCallbackUrl = async () => {
    const callbackUrl = oauthCallbackInput.trim()
    if (!callbackUrl) return

    setOauthCallbackSubmitting(true)
    setOauthCallbackError(null)
    try {
      await accountService.submitOAuthCallbackUrl(callbackUrl)
    } catch (e) {
      setOauthCallbackError(String(e).replace(/^Error:\s*/, ''))
      setOauthCallbackSubmitting(false)
    }
  }

  const handleExport = async () => {
    const visibleIdSet = new Set(filteredAccounts.map((account) => account.id))
    const selectedVisibleIds = Array.from(selected).filter((id) => visibleIdSet.has(id))
    const ids = selectedVisibleIds.length > 0 ? selectedVisibleIds : filteredAccounts.map((account) => account.id)
    if (ids.length === 0) return
    exportAccountIdsRef.current = ids
    includeExportSensitiveNotesRef.current = false
    setIncludeExportSensitiveNotes(false)
    await exportModal.startExport(ids)
  }

  const exportSelectionCount = filteredAccounts.reduce(
    (count, account) => count + (selected.has(account.id) ? 1 : 0),
    0,
  )

  const toggleSelect = (id: string) => {
    const next = new Set(selected)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    setSelected(next)
  }

  const toggleSelectAll = () => {
    if (paginatedIds.length === 0) return
    setSelected((prev) => {
      const next = new Set(prev)
      const pageFullySelected = paginatedIds.every((id) => next.has(id))
      if (pageFullySelected) {
        paginatedIds.forEach((id) => next.delete(id))
      } else {
        paginatedIds.forEach((id) => next.add(id))
      }
      return next
    })
  }

  // 从当前分组中移除选中账号
  const handleRemoveFromGroup = async () => {
    if (!activeGroupId || selected.size === 0) return
    await removeAccountsFromGroup(activeGroupId, Array.from(selected))
    setSelected(new Set())
    await reloadAccountGroups()
  }

  const handleRemoveSingleFromGroup = useCallback(
    async (groupId: string, accountId: string) => {
      setRemovingGroupAccountIds((prev) => {
        const next = new Set(prev)
        next.add(accountId)
        return next
      })

      try {
        await removeAccountsFromGroup(groupId, [accountId])
        setSelected((prev) => {
          if (!prev.has(accountId)) return prev
          const next = new Set(prev)
          next.delete(accountId)
          return next
        })
        await reloadAccountGroups()
      } catch (error) {
        console.error('Failed to remove account from group:', error)
        setMessage({
          text: t('messages.actionFailed', {
            action: t('accounts.groups.removeFromGroup'),
            error: String(error),
          }),
          tone: 'error',
        })
      } finally {
        setRemovingGroupAccountIds((prev) => {
          const next = new Set(prev)
          next.delete(accountId)
          return next
        })
      }
    },
    [reloadAccountGroups, t]
  )

  const requestDeleteGroup = useCallback((groupId: string, groupName: string) => {
    setGroupDeleteError(null)
    setGroupDeleteConfirm({
      id: groupId,
      name: groupName,
    })
  }, [])

  const confirmDeleteGroup = useCallback(async () => {
    if (!groupDeleteConfirm || deletingGroup) return

    setDeletingGroup(true)
    setGroupDeleteError(null)
    try {
      await deleteGroup(groupDeleteConfirm.id)
      await reloadAccountGroups()
      setGroupDeleteConfirm(null)
      setGroupDeleteError(null)
    } catch (error) {
      console.error('Failed to delete account group:', error)
      setGroupDeleteError(
        t('accounts.groups.error.deleteFailed', { error: String(error) })
      )
    } finally {
      setDeletingGroup(false)
    }
  }, [deletingGroup, groupDeleteConfirm, reloadAccountGroups, t])

  // 渲染分组文件夹卡片

  const toggleTagFilterValue = (tag: string) => {
    setTagFilter((prev) => {
      if (prev.includes(tag)) return prev.filter((item) => item !== tag);
      return [...prev, tag];
    });
  };

  const clearTagFilter = () => {
    setTagFilter([]);
  };

  const requestDeleteTag = (tag: string) => {
    const normalized = normalizeAccountTag(tag)
    if (!normalized) return
    const count = accounts.filter((account) =>
      (account.tags || []).some((item) => normalizeAccountTag(item) === normalized)
    ).length
    setTagDeleteConfirmError(null)
    setTagDeleteConfirm({ tag: normalized, count })
  }

  const confirmDeleteTag = async () => {
    if (!tagDeleteConfirm || deletingTag) return
    setDeletingTag(true)
    setTagDeleteConfirmError(null)
    const target = tagDeleteConfirm.tag
    const affected = accounts.filter((account) =>
      (account.tags || []).some((item) => normalizeAccountTag(item) === target)
    )

    try {
      const results = await Promise.allSettled(
        affected.map((account) => {
          const nextTags = (account.tags || []).filter(
            (item) => normalizeAccountTag(item) !== target
          )
          return accountService.updateAccountTags(account.id, nextTags)
        })
      )

      const firstRejected = results.find(
        (result): result is PromiseRejectedResult => result.status === 'rejected'
      )
      if (firstRejected) {
        setTagDeleteConfirmError(
          t('messages.actionFailed', { action: t('common.delete'), error: String(firstRejected.reason) })
        )
        return
      }

      setTagFilter((prev) => prev.filter((item) => normalizeAccountTag(item) !== target))
      await fetchAccounts()
      setTagDeleteConfirm(null)
      setTagDeleteConfirmError(null)
    } finally {
      setDeletingTag(false)
    }
  }

  const openTagModal = (accountId: string) => {
    setShowTagModal(accountId);
  };

  const openAccountNoteModal = useCallback((account: Account) => {
    setOauthAccountNoteMode(false)
    setEditingAccountNoteId(account.id)
    setEditingAccountNoteForm(buildAntigravityAccountNoteForm(account))
    setAccountNoteSecretVisible(true)
    setAccountNotePasswordVisible(true)
    setAccountNoteCopiedKey(null)
    setAccountNoteFieldError(null)
    setAccountNoteMfaPickerOpen(false)
    setSavedMfaRecords(loadSavedMfaRecords())
    setAccountNoteError(null)
    resetAccountNoteMailPreview()
  }, [resetAccountNoteMailPreview, setAccountNoteError])

  const openOAuthAccountNoteModal = useCallback(() => {
    setOauthAccountNoteMode(true)
    setEditingAccountNoteId(null)
    setEditingAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
    setAccountNoteSecretVisible(true)
    setAccountNotePasswordVisible(true)
    setAccountNoteCopiedKey(null)
    setAccountNoteFieldError(null)
    setAccountNoteMfaPickerOpen(false)
    setSavedMfaRecords(loadSavedMfaRecords())
    setAccountNoteError(null)
    resetAccountNoteMailPreview()
  }, [resetAccountNoteMailPreview, setAccountNoteError])

  useEffect(() => {
    if (editingAccountNoteId && editingAccountNoteAccount?.mail_url?.trim()) {
      void fetchAccountNoteMailPreviewForUrl(editingAccountNoteAccount.mail_url)
    }
  }, [editingAccountNoteAccount, editingAccountNoteId, fetchAccountNoteMailPreviewForUrl])

  useEffect(() => {
    if (oauthAccountNoteMode && oauthAccountNoteForm.mailUrl.trim()) {
      void fetchAccountNoteMailPreviewForUrl(oauthAccountNoteForm.mailUrl)
    }
  }, [fetchAccountNoteMailPreviewForUrl, oauthAccountNoteForm.mailUrl, oauthAccountNoteMode])

  const closeAccountNoteModal = useCallback(() => {
    if (savingAccountNote) return
    setEditingAccountNoteId(null)
    setOauthAccountNoteMode(false)
    setEditingAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
    setAccountNoteCopiedKey(null)
    setAccountNoteFieldError(null)
    setAccountNoteMfaPickerOpen(false)
    setAccountNoteError(null)
    resetAccountNoteMailPreview()
  }, [resetAccountNoteMailPreview, savingAccountNote, setAccountNoteError])

  const updateEditingAccountNoteForm = useCallback(
    (update: Partial<AntigravityAccountNoteFormState>) => {
      if (oauthAccountNoteMode) {
        setOauthAccountNoteForm((previous) => ({ ...previous, ...update }))
      } else {
        setEditingAccountNoteForm((previous) => ({ ...previous, ...update }))
      }
      if (Object.prototype.hasOwnProperty.call(update, 'twoFactorSecret')) {
        setAccountNoteFieldError(null)
      }
      if (Object.prototype.hasOwnProperty.call(update, 'mailUrl')) {
        setAccountNoteMailPreview(null)
        setAccountNoteMailPreviewError(null)
      }
      setAccountNoteError(null)
    },
    [oauthAccountNoteMode, setAccountNoteError],
  )

  const copyAccountNoteValue = useCallback(async (key: string, value: string) => {
    const text = value.trim()
    if (!text) return
    await navigator.clipboard.writeText(text)
    setAccountNoteCopiedKey(key)
    window.setTimeout(() => setAccountNoteCopiedKey((current) => (current === key ? null : current)), 1200)
  }, [])

  const handleSaveAccountNote = useCallback(async () => {
    if ((!editingAccountNoteId && !oauthAccountNoteMode) || savingAccountNote) return
    setSavingAccountNote(true)
    setAccountNoteError(null)
    setAccountNoteFieldError(null)
    try {
      const rawTwoFactorSecret = activeAccountNoteForm.twoFactorSecret.trim()
      const parsedTwoFactorSecret = rawTwoFactorSecret
        ? parseMfaCredentialInput(rawTwoFactorSecret)
        : null
      if (rawTwoFactorSecret && !parsedTwoFactorSecret) {
        setAccountNoteFieldError(
          t('accounts.accountNote.twoFactorSecretInvalid', '2FA 秘钥格式无效，请输入 Base32 secret 或 otpauth:// 链接'),
        )
        return
      }
      const normalizedTwoFactorSecret = parsedTwoFactorSecret?.secret ?? rawTwoFactorSecret
      const noteUpdate = {
        note: activeAccountNoteForm.note,
        twoFactorSecret: normalizedTwoFactorSecret,
        accountPassword: activeAccountNoteForm.accountPassword,
        phoneNumber: activeAccountNoteForm.phoneNumber,
        mailUrl: activeAccountNoteForm.mailUrl,
      }
      if (oauthAccountNoteMode) {
        setOauthAccountNoteForm({ ...activeAccountNoteForm, twoFactorSecret: normalizedTwoFactorSecret })
      } else if (editingAccountNoteId) {
        await updateAccountNotes(editingAccountNoteId, noteUpdate)
      }
      if (normalizedTwoFactorSecret) {
        setSavedMfaRecords(upsertSavedMfaRecord({
          secret: normalizedTwoFactorSecret,
          accountName: editingAccountNoteAccount?.email ?? parsedTwoFactorSecret?.accountName ?? null,
          remark: activeAccountNoteForm.note,
        }))
      }
      setMessage({ text: t('accounts.accountNote.saved', '账号备注已保存') })
      setEditingAccountNoteId(null)
      setOauthAccountNoteMode(false)
      setEditingAccountNoteForm(EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM)
    } catch (error) {
      setAccountNoteError(
        t('accounts.accountNote.saveFailed', {
          error: String(error).replace(/^Error:\s*/, ''),
          defaultValue: '保存账号备注失败：{{error}}',
        })
      )
    } finally {
      setSavingAccountNote(false)
    }
  }, [
    activeAccountNoteForm,
    editingAccountNoteId,
    editingAccountNoteAccount,
    oauthAccountNoteMode,
    savingAccountNote,
    setAccountNoteError,
    t,
    updateAccountNotes,
  ])

  const accountNoteOtpToken = useMemo(() => {
    const secret = activeAccountNoteForm.twoFactorSecret.trim()
    return secret ? getMfaOtpToken(secret) : ''
  }, [activeAccountNoteForm.twoFactorSecret, mfaTimeRemaining])

  const renderAccountNoteChip = useCallback((account: Account) => {
    const hasNote = hasAntigravityAccountNoteDetails(account)
    return (
      <button
        type="button"
        className={`codex-account-note-chip ${hasNote ? 'has-note' : 'empty-note'}`}
        onClick={() => openAccountNoteModal(account)}
        title={hasNote
          ? getAntigravityAccountNoteTitle(account, t('accounts.accountNote.short', '账号备注'))
          : t('accounts.accountNote.emptyTitle', '填写账号备注')}
      >
        <FileText size={12} />
        <span>{hasNote ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.addShort', '加备注')}</span>
      </button>
    )
  }, [openAccountNoteModal, t])

  useEscClose((Boolean(editingAccountNoteId) || oauthAccountNoteMode) && !savingAccountNote, closeAccountNoteModal)

  const handleSaveTags = async (tags: string[], notes?: string) => {
    if (!showTagModal) return;
    const scrollY = window.scrollY
    const accountId = showTagModal
    await accountService.updateAccountNotes(accountId, notes ?? '')
    await updateAccountTags(accountId, tags);
    setShowTagModal(null);
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        window.scrollTo({ top: scrollY, behavior: 'auto' })
      })
    })
  };

  const handleAssignAccountsToGroup = async (
    groupId: string,
    groupName: string,
    accountIds: string[]
  ) => {
    const currentGroup = accountGroups.find((group) => group.id === groupId)
    if (!currentGroup) return

    const nextName = groupName.trim()
    if (!nextName) {
      throw new Error(t('platformLayout.groupNameRequired'))
    }

    if (accountGroups.some((group) => group.id !== groupId && group.name === nextName)) {
      throw new Error(t('accounts.groups.error.duplicate'))
    }

    const currentIds = new Set(currentGroup.accountIds)
    const nextIds = new Set(accountIds)
    const addedIds = accountIds.filter((accountId) => !currentIds.has(accountId))
    const removedIds = currentGroup.accountIds.filter((accountId) => !nextIds.has(accountId))
    const shouldRename = nextName !== currentGroup.name

    if (!shouldRename && addedIds.length === 0 && removedIds.length === 0) return

    if (shouldRename) {
      await renameGroup(groupId, nextName)
    }

    if (accountIds.length > 0) {
      await assignAccountsToGroup(groupId, accountIds)
    }

    if (removedIds.length > 0) {
      await removeAccountsFromGroup(groupId, removedIds)
    }

    await reloadAccountGroups()
  }

  const formatDate = (timestamp: number) => {
    const d = new Date(timestamp * 1000)
    return (
      d.toLocaleDateString(locale, {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit'
      }) +
      ' ' +
      d.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })
    )
  }

  const normalizeWarningMessage = (raw: string) =>
    raw.replace(/^Error:\s*/i, '').trim()

  const extractQuotaErrorMessage = (raw: string) => {
    const trimmed = raw.trim()
    if (!trimmed) return raw
    try {
      const parsed = JSON.parse(trimmed)
      if (parsed?.error?.message) {
        return String(parsed.error.message)
      }
    } catch (_) {
      // Keep raw message if it is not JSON.
    }
    return raw
  }

  const renderErrorMessage = (raw: string) => {
    const message = extractQuotaErrorMessage(raw)
    const parts = message.split(/(https?:\/\/[^\s]+)/g)
    const linkRegex = /(https?:\/\/[^\s]+)/
    return parts.map((part, index) => {
      if (linkRegex.test(part)) {
        return (
          <a key={`link-${index}`} href={part} target="_blank" rel="noreferrer">
            {part}
          </a>
        )
      }
      return <span key={`text-${index}`}>{part}</span>
    })
  }

  const isAuthFailure = (message: string) => {
    const lower = message.toLowerCase()
    return (
      lower.includes('invalid_grant') ||
      lower.includes('unauthorized') ||
      lower.includes('unauthenticated') ||
      lower.includes('invalid authentication') ||
      lower.includes('401')
    )
  }

  const parseRefreshDetail = (
    detail: string
  ): { email: string; reason: string } | null => {
    const match = detail.match(/^Account\s+(.+?):\s+(.+)$/)
    if (!match) return null
    const email = match[1].trim()
    let reason = match[2].trim()
    reason = reason.replace(/^Fetch quota failed\s*-\s*/i, '')
    reason = reason.replace(/^Save quota failed\s*-\s*/i, '')
    return { email, reason }
  }

  const buildWarningMapFromDetails = (details: string[]) => {
    const next: Record<string, { kind: 'auth' | 'error'; message: string }> = {}
    details.forEach((detail) => {
      const parsed = parseRefreshDetail(detail)
      if (!parsed) return
      const reason = normalizeWarningMessage(parsed.reason)
      next[parsed.email] = {
        kind: isAuthFailure(reason) ? 'auth' : 'error',
        message: reason
      }
    })
    return next
  }

  useEffect(() => {
    if (Object.keys(refreshWarnings).length === 0) return
    const existing = new Set(accounts.map((acc) => acc.email))
    setRefreshWarnings((prev) => {
      let changed = false
      const next: Record<string, { kind: 'auth' | 'error'; message: string }> =
        {}
      Object.entries(prev).forEach(([email, warning]) => {
        if (existing.has(email)) {
          next[email] = warning
        } else {
          changed = true
        }
      })
      return changed ? next : prev
    })
  }, [accounts, refreshWarnings])

  const resolveGroupLabel = (groupKey: string) =>
    groupKey === untaggedKey ? t('accounts.untagged', '未分组') : groupKey

  const renderCustomQuotaSection = (account: Account, isList: boolean = false) => {
    const quotaDisplayItems = getQuotaDisplayItems(account);
    const hasModels = account.quota?.models && account.quota.models.length > 0;
    
    if (!hasModels) {
      return (
        <div className="quota-empty" style={{ gridColumn: '1 / -1', textAlign: 'center' }}>
          {t('overview.noQuotaData')}
        </div>
      );
    }

    const claude5h = quotaDisplayItems.find(item => item.key === 'claude:5h');
    const claudeWeekly = quotaDisplayItems.find(item => item.key === 'claude:weekly');
    const gemini5h = quotaDisplayItems.find(item => item.key === 'gemini:5h');
    const geminiWeekly = quotaDisplayItems.find(item => item.key === 'gemini:weekly');

    const renderBar = (label: string, item: any) => {
      const percentage = item ? item.percentage : 100;
      const resetTime = item ? item.resetTime : '';
      const resetLabel = resetTime ? formatResetTimeDisplay(resetTime, t) : '';
      
      return (
        <div className={isList ? "quota-item" : "quota-compact-item"}>
          <div className={isList ? "quota-header" : "quota-compact-header"}>
            <span className={isList ? "quota-name" : "model-label"}>{label}</span>
            <span className={`${isList ? "quota-value" : "model-pct"} ${getQuotaClass(percentage)}`}>
              {percentage}%
            </span>
          </div>
          <div className={isList ? "quota-progress-track" : "quota-compact-bar-track"}>
            <div
              className={`${isList ? "quota-progress-bar" : "quota-compact-bar"} ${getQuotaClass(percentage)}`}
              style={{ width: `${percentage}%` }}
            />
          </div>
          {(isList || resetLabel) && (
            <div className={isList ? "quota-footer" : undefined}>
              <span
                className={isList ? "quota-reset" : "quota-compact-reset"}
                title={resetLabel || undefined}
              >
                {resetLabel || '\u00A0'}
              </span>
            </div>
          )}
        </div>
      );
    };

    return (
      <>
        <div className="quota-column">
          <div className="quota-column-title">Claude</div>
          {renderBar("5h", claude5h)}
          {renderBar(t('common.weekly', 'Weekly'), claudeWeekly)}
        </div>
        <div className="quota-column">
          <div className="quota-column-title">Gemini</div>
          {renderBar("5h", gemini5h)}
          {renderBar(t('common.weekly', 'Weekly'), geminiWeekly)}
        </div>
      </>
    );
  };

  const renderGridCards = (items: Account[], groupKey?: string) =>
    items.map((account) => {
      const isCurrent = currentAccount?.id === account.id
      const tierBadge = getAntigravityTierBadge(account.quota)
      const availableCreditsDisplay = getAvailableAICreditsDisplay(account)
      const isDisabled = account.disabled
      const isForbidden = Boolean(account.quota?.is_forbidden)
      const isSelected = selected.has(account.id)
      const quotaError = account.quota_error
      const hasQuotaError = Boolean(quotaError?.message)
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean)
      const visibleTags = accountTags.slice(0, 2)
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length)
      const warning = refreshWarnings[account.email]
      const warningLabel =
        warning?.kind === 'auth'
          ? t('accounts.status.authInvalid')
          : t('accounts.status.refreshFailed')
      const warningTitle = warning?.message || ''
      const forbiddenTitle = t('accounts.status.forbidden_tooltip')
      const disabledTitle = isDisabled
        ? `${t('accounts.status.disabled')}${account.disabled_reason ? `: ${account.disabled_reason}` : ''}`
        : ''
      const verificationReason = account.disabled_reason || verificationStatusMap[account.id]
      const hasVerificationIssue = verificationReason === 'verification_required' || verificationReason === 'tos_violation'

      const hasModels = account.quota?.models && account.quota.models.length > 0
      if (!hasModels) {
        console.log('[AccountsPage] 账号无配额数据:', {
          email: account.email,
          isCurrent,
          hasQuota: !!account.quota,
          quotaModelCount: account.quota?.models?.length ?? 0
        })
      }

      return (
        <div
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`account-card ${isCurrent ? 'current' : ''} ${isDisabled ? 'disabled' : ''} ${isSelected ? 'selected' : ''}`}
        >
          <div className="card-top">
            <div className="card-select">
              <input
                type="checkbox"
                checked={isSelected}
                onChange={() => toggleSelect(account.id)}
              />
            </div>
            <span className="account-email" title={maskAccountText(account.email)}>
              {maskAccountText(account.email)}
            </span>
            {isCurrent && (
              <span className="current-tag">
                {t('accounts.status.current')}
              </span>
            )}
            {warning && (
              <span className="status-pill warning" title={warningTitle}>
                <CircleAlert size={12} />
                {warningLabel}
              </span>
            )}
            {isDisabled && (
              <span className="status-pill disabled" title={disabledTitle}>
                <CircleAlert size={12} />
                {t('accounts.status.disabled')}
              </span>
            )}
            {isForbidden && (
              <span className="status-pill forbidden" title={forbiddenTitle}>
                <Lock size={12} />
                {t('accounts.status.forbidden')}
              </span>
            )}
            <span className={`tier-badge ${tierBadge.className}`}>
              {tierBadge.label}
            </span>
            {isPendingAntigravityAccount(account) && (
              <span className="status-pill warning">{t('codex.pendingAuth.badge', '待授权')}</span>
            )}
            {(() => {
              const vBadge = getVerificationBadge(account)
              return vBadge ? (
                <span className={`verification-status-pill ${vBadge.className}`} title={vBadge.label}>
                  {vBadge.label}
                </span>
              ) : null
            })()}
          </div>

          <div className="account-sub-line antigravity-account-meta-inline">
            {renderAccountNoteChip(account)}
            {isPendingAntigravityAccount(account) && (
              <button type="button" className="btn btn-sm btn-outline" onClick={() => openPendingOAuthAccount(account)}>
                {t('codex.pendingAuth.authorizeAction', '授权添加')}
              </button>
            )}
          </div>

          {account.notes && (
            <div className="card-notes">
              <span className="notes-text" title={account.notes}>{account.notes}</span>
            </div>
          )}

          <div className="card-quota-grid">
            {isForbidden ? (
              <div className="quota-forbidden" title={forbiddenTitle}>
                <Lock size={14} />
                <span>{t('accounts.status.forbidden_msg')}</span>
              </div>
            ) : (
              <>
                {hasQuotaError && (
                  <div className="quota-empty" title={quotaError?.message}>
                    {t('common.shared.quota.queryFailed', '配额查询失败')}
                  </div>
                )}
                {renderCustomQuotaSection(account, false)}
              </>
            )}
            <div className="quota-credits-field">
              <span className="quota-credits-label">
                {t('common.shared.credits.availableAiCredits', 'Available AI Credits')}: {availableCreditsDisplay}
              </span>
            </div>
          </div>

          {accountTags.length > 0 && (
            <div className="card-tags">
              {visibleTags.map((tag, idx) => (
                <span key={`${account.id}-${tag}-${idx}`} className="tag-pill">
                  {tag}
                </span>
              ))}
              {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
            </div>
          )}
          <div className="card-footer">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              {isPendingAntigravityAccount(account) && (
                <button
                  type="button"
                  className="card-action-btn pending-auth-action"
                  onClick={() => openPendingOAuthAccount(account)}
                  title={t('common.shared.addModal.oauth', 'OAuth 授权')}
                >
                  <Globe size={14} />
                </button>
              )}
              {(hasQuotaError || hasVerificationIssue) && (
                <button
                  className="card-action-btn is-danger"
                  onClick={() =>
                    hasVerificationIssue
                      ? setShowVerificationErrorModal(account.id)
                      : setShowErrorModal(account.id)
                  }
                  title={t('accounts.actions.viewError')}
                >
                  <AlertTriangle size={14} />
                </button>
              )}
              <button
                className="card-action-btn"
                onClick={() => setShowQuotaModal(account.id)}
                title={t('accounts.actions.viewDetails')}
              >
                <CircleAlert size={14} />
              </button>
              <button
                className="card-action-btn"
                onClick={() => openTagModal(account.id)}
                title={t('accounts.editTags', '编辑标签')}
              >
                <Tag size={14} />
              </button>
              <button
                type="button"
                className={`card-action-btn ${hasAntigravityAccountNoteDetails(account) ? 'active' : ''}`}
                onClick={() => isPendingAntigravityAccount(account) ? openPendingOAuthAccount(account) : openAccountNoteModal(account)}
                title={hasAntigravityAccountNoteDetails(account) ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.emptyTitle', '填写账号备注')}
                aria-label={t('accounts.accountNote.title', '账号备注')}
              >
                <FileText size={14} />
              </button>
              <button
                className={`card-action-btn ${!isCurrent ? 'success' : ''}`}
                onClick={() => handleSwitch(account.id)}
                disabled={!!switching}
                title={
                  isCurrent
                    ? t('accounts.actions.switch')
                    : t('accounts.actions.switchTo')
                }
              >
                {switching === account.id ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Play size={14} />
                )}
              </button>
              <button
                className={`card-action-btn${refreshResult[account.id] === 'success' ? ' is-success' : refreshResult[account.id] === 'error' ? ' is-danger' : ''}`}
                onClick={() => handleRefresh(account.id)}
                disabled={refreshing.has(account.id)}
                title={t('accounts.refreshQuota')}
              >
                {refreshing.has(account.id) ? (
                  <RotateCw size={14} className="loading-spinner" />
                ) : refreshResult[account.id] === 'success' ? (
                  <Check size={16} className="text-success" />
                ) : refreshResult[account.id] === 'error' ? (
                  <X size={16} className="text-danger" />
                ) : (
                  <RotateCw size={14} />
                )}
              </button>
              <button
                className="card-action-btn export-btn"
                onClick={() => handleExportSingle(account)}
                title={t('accounts.export')}
              >
                <Upload size={14} />
              </button>
              <button
                className="card-action-btn danger"
                onClick={() => handleDelete(account.id)}
                title={t('common.delete')}
              >
                <Trash2 size={14} />
              </button>
            </div>
          </div>
        </div>
      )
    })

  // 渲染文件夹卡片（嵌入accounts-grid内）
  const renderInlineFolderCards = () => {
    if (activeGroupId || accountGroups.length === 0) return null
    return accountGroups.map((group) => {
      const groupAccounts = accounts.filter((acc) => group.accountIds.includes(acc.id))
      return (
        <div
          key={`folder-${group.id}`}
          className="account-card folder-inline-card"
          onClick={() => {
            setActiveGroupId(group.id)
            setSelected(new Set())
          }}
        >
          <div className="folder-inline-header">
            <div className="folder-inline-icon">
              <FolderOpen size={24} />
            </div>
            <div className="folder-inline-info">
              <span className="folder-inline-name">{group.name}</span>
              <span className="folder-inline-count">
                {t('accounts.groups.accountCount', { count: groupAccounts.length })}
              </span>
            </div>
            <button
              className="folder-icon-btn"
              title={t('accounts.groups.addAccounts')}
              onClick={(e) => {
                e.stopPropagation()
                setGroupQuickAddGroupId(group.id)
              }}
            >
              <FolderPlus size={14} />
            </button>
            <button
              className="folder-icon-btn"
              title={t('accounts.groups.editTitle')}
              onClick={(e) => {
                e.stopPropagation()
                setGroupAccountPickerGroupId(group.id)
              }}
            >
              <Pencil size={14} />
            </button>
            <button
              className="folder-icon-btn folder-delete-btn"
              title={t('accounts.groups.deleteTitle')}
              onClick={(e) => {
                e.stopPropagation()
                requestDeleteGroup(group.id, group.name)
              }}
            >
              <Trash2 size={14} />
            </button>
          </div>
          <div className="folder-inline-preview">
            {groupAccounts.map((acc) => (
              <div key={acc.id} className={`folder-preview-item${acc.disabled ? ' disabled' : ''}`}>
                <span className="folder-preview-email" title={maskAccountText(acc.email) || ''}>
                  {maskAccountText(acc.email)}
                </span>
                {acc.quota?.subscription_tier && (
                  <span className={`tier-badge ${(acc.quota.subscription_tier || '').replace(/-tier$/, '').replace('g1-', '').toLowerCase()}`}>
                    {(acc.quota.subscription_tier || '').replace(/-tier$/, '').replace('g1-', '').toUpperCase()}
                  </span>
                )}
                <button
                  type="button"
                  className="folder-preview-remove-btn"
                  onClick={(e) => {
                    e.stopPropagation()
                    void handleRemoveSingleFromGroup(group.id, acc.id)
                  }}
                  title={t('accounts.groups.removeFromGroup')}
                  aria-label={`${t('accounts.groups.removeFromGroup')}: ${maskAccountText(acc.email)}`}
                  disabled={removingGroupAccountIds.has(acc.id)}
                >
                  <LogOut size={12} />
                </button>
              </div>
            ))}
          </div>
        </div>
      )
    })
  }

  // 渲染卡片视图
  const renderGridView = () => {
    return (
      <div className="grid-view-container">
        {!groupByTag ? (
          <div className="accounts-grid">
            {renderInlineFolderCards()}
            {renderGridCards(paginatedAccounts)}
          </div>
        ) : (
          <div className="tag-group-list">
            {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
              <div key={groupKey} className="tag-group-section">
                <div className="tag-group-header">
                  <span className="tag-group-title">
                    {resolveGroupLabel(groupKey)}
                  </span>
                  <span className="tag-group-count">{totalCount}</span>
                </div>
                <div className="tag-group-grid accounts-grid">
                  {renderGridCards(items, groupKey)}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    )
  }

  const handleExportSingle = async (account: Account) => {
    const baseName = account.email.includes('@')
      ? account.email.slice(0, account.email.indexOf('@'))
      : account.email
    exportAccountIdsRef.current = [account.id]
    includeExportSensitiveNotesRef.current = false
    setIncludeExportSensitiveNotes(false)
    await exportModal.startExport([account.id], baseName)
  }

  // 渲染紧凑视图 - 只显示邮箱和配额百分比
	  const renderCompactView = () => {
    // 获取排序后的分组
    const orderedGroups = getOrderedDisplayGroups()
    // 过滤隐藏的分组用于显示配额
    const visibleGroups = orderedGroups.filter((g) => !hiddenGroups.has(g.id))

    // 构建分组配置用于计算综合配额
    const groupSettings: GroupSettings = {
      groupMappings: {},
      groupNames: {},
      groupOrder: orderedGroups.map((g) => g.id),
      updatedAt: 0,
      updatedBy: 'desktop'
    }
    for (const group of orderedGroups) {
      groupSettings.groupNames[group.id] = group.name
      for (const modelId of group.models) {
        groupSettings.groupMappings[modelId] = group.id
      }
    }

    const renderCompactCards = (items: Account[]) =>
      items.map((account) => {
        const isCurrent = currentAccount?.id === account.id
        const tierBadge = getAntigravityTierBadge(account.quota)
        const quotas = getAccountQuotas(account)
        const overallQuota = calculateOverallQuota(quotas)
        const isSelected = selected.has(account.id)
        const isDisabled = account.disabled
        const isForbidden = Boolean(account.quota?.is_forbidden)
        const warning = refreshWarnings[account.email]
        const warningLabel =
          warning?.kind === 'auth'
            ? t('accounts.status.authInvalid')
            : t('accounts.status.refreshFailed')
        const warningTitle = warning?.message || ''
        const forbiddenTitle = t('accounts.status.forbidden_tooltip')
        const disabledTitle = isDisabled
          ? `${t('accounts.status.disabled')}${account.disabled_reason ? `: ${account.disabled_reason}` : ''}`
          : ''
        const statusHints = []
        if (warning) statusHints.push(warningTitle || warningLabel)
        if (isDisabled) statusHints.push(disabledTitle || t('accounts.status.disabled'))
        if (isForbidden) statusHints.push(forbiddenTitle)
        const statusTitle = statusHints.join(' / ')

        // 获取可见分组的配额（按排序后的顺序，排除隐藏的和无配额数据的）
        const groupQuotas = visibleGroups
          .map((group) => {
            const colorIdx = getGroupColorIndex(
              group.id,
              orderedGroups.findIndex((g) => g.id === group.id) % 8
            )
            const percentage = calculateGroupQuota(
              group.id,
              quotas,
              groupSettings
            )
            return {
              id: group.id,
              name: group.name,
              percentage,
              color: colorOptions[colorIdx]?.color || colorOptions[0].color
            }
          })
          .filter((gq) => gq.percentage !== null) as Array<{
            id: string
            name: string
            percentage: number
            color: string
          }>

        const isSwitching = switching === account.id

        return (
          <div
            key={account.id}
            className={`${styles.card} ${isCurrent ? styles.cardCurrent : ''} ${isSelected ? styles.cardSelected : ''} ${isSwitching ? styles.cardSwitching : ''}`}
            onClick={() => {
              if (!switching) toggleSelect(account.id)
            }}
            title={maskAccountText(account.email)}
            style={{ pointerEvents: switching ? 'none' : undefined }}
          >
            <input
              type="checkbox"
              checked={isSelected}
              onChange={(e) => {
                e.stopPropagation()
                toggleSelect(account.id)
              }}
              onClick={(e) => e.stopPropagation()}
            />
            <span
              className={`${styles.email} ${tierBadge.tier === 'PRO' || tierBadge.tier === 'ULTRA' ? styles.emailGradient : ''}`}
            >
              {(warning || isDisabled || isForbidden) && (
                <span className={styles.statusIcon} title={statusTitle}>
                  !
                </span>
              )}
              <span className={styles.emailText}>
                {maskAccountText(account.email)}
              </span>
            </span>
            <div className={styles.quotas}>
              {groupQuotas.length > 0 ? (
                groupQuotas.map((gq) => (
                  <span
                    key={gq.id}
                    className={`${styles.quota} ${gq.percentage >= 50 ? styles.quotaHigh : gq.percentage >= 20 ? styles.quotaMedium : styles.quotaLow}`}
                    title={gq.name}
                  >
                    <span
                      className={styles.dot}
                      style={{ background: gq.color }}
                    />
                    {gq.percentage}%
                  </span>
                ))
              ) : (
                <span
                  className={`${styles.quota} ${overallQuota >= 50 ? styles.quotaHigh : overallQuota >= 20 ? styles.quotaMedium : styles.quotaLow}`}
                >
                  {overallQuota}%
                </span>
              )}
            </div>
            <button
              type="button"
              className={`${styles.noteBtn} ${hasAntigravityAccountNoteDetails(account) ? styles.noteBtnActive : ''}`}
              onClick={(event) => {
                event.stopPropagation()
                openAccountNoteModal(account)
              }}
              title={hasAntigravityAccountNoteDetails(account) ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.emptyTitle', '填写账号备注')}
              aria-label={t('accounts.accountNote.title', '账号备注')}
            >
              <FileText size={12} />
              <span>{hasAntigravityAccountNoteDetails(account) ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.addShort', '加备注')}</span>
            </button>
            <button
              type="button"
              className={styles.switchBtn}
              onClick={(e) => {
                e.stopPropagation()
                handleSwitch(account.id)
              }}
              disabled={isSwitching}
              title={
                isCurrent
                  ? t('accounts.actions.switch')
                  : t('accounts.actions.switchTo')
              }
              aria-label={
                isCurrent
                  ? t('accounts.actions.switch')
                  : t('accounts.actions.switchTo')
              }
            >
              <Play size={12} />
            </button>
          </div>
        )
      })

	    return (
	      <>
	        <div className={styles.container}>
          {/* 图例 - 支持拖拽排序、颜色选择、显示/隐藏 */}
          {orderedGroups.length > 0 && (
            <div
              className={styles.legend}
              onMouseUp={handleDragEnd}
              onMouseLeave={handleDragEnd}
            >
              {orderedGroups.map((group, index) => {
                const colorIdx = getGroupColorIndex(group.id, index % 8)
                const isHidden = hiddenGroups.has(group.id)
                const isPickerOpen = showColorPicker === group.id

                return (
                  <span
                    key={group.id}
                    className={`${styles.legendItem} ${draggedGroupId === group.id ? styles.legendItemDragging : ''} ${draggedGroupId && draggedGroupId !== group.id ? styles.legendItemDropTarget : ''} ${isHidden ? styles.legendItemHidden : ''}`}
                    onMouseEnter={() => handleDragMove(group.id)}
                  >
                    {/* 拖拽手柄 - 只有这里触发拖拽 */}
                    <GripVertical
                      size={12}
                      className={styles.gripIcon}
                      onMouseDown={(e) => handleDragStart(e, group.id)}
                    />

                    {/* 颜色点 - 点击打开颜色选择器 */}
                    <span
                      className={styles.legendDotWrapper}
                      onClick={(e) => openColorPicker(e, group.id, isPickerOpen)}
                    >
                      <span
                        className={styles.legendDot}
                        style={{
                          background:
                            colorOptions[colorIdx]?.color || colorOptions[0].color
                        }}
                      />
                    </span>

                    <span className={styles.legendName}>{group.name}</span>

                    {/* 显示/隐藏切换 */}
                    <button
                      className={styles.visibilityBtn}
                      onClick={(e) => {
                        e.stopPropagation()
                        toggleGroupVisibility(group.id)
                      }}
                      title={
                        isHidden
                          ? t('accounts.compact.show', '显示')
                          : t('accounts.compact.hide', '隐藏')
                      }
                    >
                      {isHidden ? <EyeOff size={12} /> : <Eye size={12} />}
                    </button>
                  </span>
                )
              })}
            </div>
          )}

	          {/* 账号列表 */}
	          {groupByTag ? (
	            <div className="tag-group-list">
	              {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                <div key={groupKey} className="tag-group-section">
                  <div className="tag-group-header">
                    <span className="tag-group-title">
                      {resolveGroupLabel(groupKey)}
                    </span>
                    <span className="tag-group-count">
                      {totalCount}
                    </span>
                  </div>
                  <div className={`tag-group-grid ${styles.grid}`}>
                    {renderCompactCards(items)}
                  </div>
                </div>
              ))}
            </div>
	          ) : (
	            <>
	              {hasVisibleAccountGroups && (
	                <div className="accounts-grid">{renderInlineFolderCards()}</div>
	              )}
	              <div className={styles.grid}>{renderCompactCards(paginatedAccounts)}</div>
	            </>
	          )}
	        </div>

        {/* Color Picker Portal - rendered to body */}
        {showColorPicker &&
          colorPickerPos &&
          createPortal(
            <div
              ref={colorPickerRef}
              className={styles.colorPickerPortal}
              style={{
                position: 'fixed',
                top: colorPickerPos.top,
                left: colorPickerPos.left,
                transform: 'translateX(-50%)',
                zIndex: 9999
              }}
              onClick={(e) => e.stopPropagation()}
            >
              {colorOptions.map((opt) => {
                const groupId = showColorPicker
                const currentColorIdx = getGroupColorIndex(
                  groupId,
                  orderedGroups.findIndex((g) => g.id === groupId) % 8
                )
                return (
                  <span
                    key={opt.index}
                    className={`${styles.colorOption} ${currentColorIdx === opt.index ? styles.colorOptionActive : ''}`}
                    style={{ background: opt.color }}
                    onClick={() => setGroupColor(groupId, opt.index)}
                    title={opt.name}
                  />
                )
              })}
            </div>,
            document.body
          )}
      </>
    )
  }

  const renderListRows = (items: Account[], groupKey?: string) =>
    items.map((account) => {
      const isCurrent = currentAccount?.id === account.id
      const tierBadge = getAntigravityTierBadge(account.quota)
      const availableCreditsDisplay = getAvailableAICreditsDisplay(account)
      const isForbidden = Boolean(account.quota?.is_forbidden)
      const quotaError = account.quota_error
      const hasQuotaError = Boolean(quotaError?.message)
      const warning = refreshWarnings[account.email]
      const warningLabel =
        warning?.kind === 'auth'
          ? t('accounts.status.authInvalid')
          : t('accounts.status.refreshFailed')
      const warningTitle = warning?.message || ''
      const forbiddenTitle = t('accounts.status.forbidden_tooltip')
      const disabledTitle = account.disabled
        ? `${t('accounts.status.disabled')}${account.disabled_reason ? `: ${account.disabled_reason}` : ''}`
        : ''
      const verificationReason = account.disabled_reason || verificationStatusMap[account.id]
      const hasVerificationIssue = verificationReason === 'verification_required' || verificationReason === 'tos_violation'

      return (
        <tr
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={isCurrent ? 'current' : ''}
        >
          <td>
            <input
              type="checkbox"
              checked={selected.has(account.id)}
              onChange={() => toggleSelect(account.id)}
            />
          </td>
          <td>
            <div className="account-cell">
              <div className="account-main-line">
                <span className="account-email-text" title={maskAccountText(account.email)}>
                  {maskAccountText(account.email)}
                </span>
                {isCurrent && (
                  <span className="mini-tag current">
                    {t('accounts.status.current')}
                  </span>
                )}
                {isPendingAntigravityAccount(account) && (
                  <span className="status-pill warning">{t('codex.pendingAuth.badge', '待授权')}</span>
                )}
              </div>
              <div className="account-sub-line antigravity-account-meta-inline">
                {renderAccountNoteChip(account)}
                {isPendingAntigravityAccount(account) && (
                  <button type="button" className="btn btn-sm btn-outline" onClick={() => openPendingOAuthAccount(account)}>
                    {t('codex.pendingAuth.authorizeAction', '授权添加')}
                  </button>
                )}
              </div>
              <div className="account-sub-line">
                <span className={`tier-badge ${tierBadge.className}`}>
                  {tierBadge.label}
                </span>
                {(() => {
                  const vBadge = getVerificationBadge(account)
                  return vBadge ? (
                    <span className={`verification-status-pill ${vBadge.className}`} title={vBadge.label}>
                      {vBadge.label}
                    </span>
                  ) : null
                })()}
                {warning && (
                  <span className="status-pill warning" title={warningTitle}>
                    <CircleAlert size={12} />
                    {warningLabel}
                  </span>
                )}
                {account.disabled && (
                  <span className="status-pill disabled" title={disabledTitle}>
                    <CircleAlert size={12} />
                    {t('accounts.status.disabled')}
                  </span>
                )}
                {isForbidden && (
                  <span className="status-pill forbidden" title={forbiddenTitle}>
                    <Lock size={12} />
                    {t('accounts.status.forbidden')}
                  </span>
                )}
              </div>
            </div>
          </td>
          <td>
            <div className="quota-grid">
              {isForbidden ? (
                <div className="quota-forbidden" title={forbiddenTitle}>
                  <Lock size={14} />
                  <span>{t('accounts.status.forbidden_msg')}</span>
                </div>
              ) : (
                <>
                  {hasQuotaError && (
                    <div className="quota-empty" title={quotaError?.message}>
                      {t('common.shared.quota.queryFailed', '配额查询失败')}
                    </div>
                  )}
                  {renderCustomQuotaSection(account, true)}
                </>
              )}
              <div className="quota-credits-field">
                <span className="quota-credits-label">
                  {t('common.shared.credits.availableAiCredits', 'Available AI Credits')}: {availableCreditsDisplay}
                </span>
              </div>
            </div>
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              {isPendingAntigravityAccount(account) && (
                <button type="button" className="action-btn" onClick={() => openPendingOAuthAccount(account)} title={t('common.shared.addModal.oauth', 'OAuth 授权')}>
                  <Globe size={16} />
                </button>
              )}
              {(hasQuotaError || hasVerificationIssue) && (
                <button
                  className="action-btn is-danger"
                  onClick={() =>
                    hasVerificationIssue
                      ? setShowVerificationErrorModal(account.id)
                      : setShowErrorModal(account.id)
                  }
                  title={t('accounts.actions.viewError')}
                >
                  <AlertTriangle size={16} />
                </button>
              )}
              <button
                className="action-btn"
                onClick={() => setShowQuotaModal(account.id)}
                title={t('accounts.actions.viewDetails')}
              >
                <CircleAlert size={16} />
              </button>
              <button
                className="action-btn"
                onClick={() => openTagModal(account.id)}
                title={t('accounts.editTags', '编辑标签')}
              >
                <Tag size={16} />
              </button>
              <button
                type="button"
                className={`action-btn ${hasAntigravityAccountNoteDetails(account) ? 'active' : ''}`}
                onClick={() => isPendingAntigravityAccount(account) ? openPendingOAuthAccount(account) : openAccountNoteModal(account)}
                title={hasAntigravityAccountNoteDetails(account) ? t('accounts.accountNote.short', '账号备注') : t('accounts.accountNote.emptyTitle', '填写账号备注')}
                aria-label={t('accounts.accountNote.title', '账号备注')}
              >
                <FileText size={16} />
              </button>
              <button
                className={`action-btn ${!isCurrent ? 'success' : ''}`}
                onClick={() => handleSwitch(account.id)}
                disabled={!!switching}
                title={
                  isCurrent
                    ? t('accounts.actions.switch')
                    : t('accounts.actions.switchTo')
                }
              >
                {switching === account.id ? (
                  <div className="loading-spinner" style={{ width: 14, height: 14 }} />
                ) : (
                  <Play size={16} />
                )}
              </button>
              <button
                className={`action-btn${refreshResult[account.id] === 'success' ? ' is-success' : refreshResult[account.id] === 'error' ? ' is-danger' : ''}`}
                onClick={() => handleRefresh(account.id)}
                disabled={refreshing.has(account.id)}
                title={t('accounts.refreshQuota')}
              >
                {refreshing.has(account.id) ? (
                  <RotateCw size={16} className="loading-spinner" />
                ) : refreshResult[account.id] === 'success' ? (
                  <Check size={18} className="text-success" />
                ) : refreshResult[account.id] === 'error' ? (
                  <X size={18} className="text-danger" />
                ) : (
                  <RotateCw size={16} />
                )}
              </button>
              <button
                className="action-btn"
                onClick={() => handleExportSingle(account)}
                title={t('accounts.export')}
              >
                <Upload size={16} />
              </button>
              <button
                className="action-btn danger"
                onClick={() => handleDelete(account.id)}
                title={t('common.delete')}
              >
                <Trash2 size={16} />
              </button>
            </div>
          </td>
        </tr>
      )
    })

  // 渲染列表视图
  const renderListView = () => (
    <div className={`account-table-container${groupByTag ? ' grouped' : ''}`}>
      <table className="account-table">
        <thead>
          <tr>
            <th style={{ width: 40 }}>
              <input
                type="checkbox"
                checked={allPaginatedSelected}
                onChange={toggleSelectAll}
              />
            </th>
            <th style={{ width: 220 }}>{t('accounts.columns.email')}</th>
            <th>{t('accounts.columns.quota')}</th>
            <th className="sticky-action-header table-action-header">
              {t('accounts.columns.actions')}
            </th>
          </tr>
        </thead>
        <tbody>
          {!activeGroupId && accountGroups.length > 0 && accountGroups.map((group) => {
            const groupAccounts = accounts.filter((acc) => group.accountIds.includes(acc.id))
            return (
              <tr
                key={`folder-row-${group.id}`}
                className="folder-table-row"
                style={{ cursor: 'pointer' }}
                onClick={() => {
                  setActiveGroupId(group.id)
                  setSelected(new Set())
                }}
              >
                <td></td>
                <td colSpan={3}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <FolderOpen size={16} style={{ color: 'var(--primary)' }} />
                    <strong>{group.name}</strong>
                    <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>
                      {t('accounts.groups.accountCount', { count: groupAccounts.length })}
                    </span>
                  </div>
                </td>
                <td>
                  <div className="folder-table-actions">
                    <button
                      className="folder-icon-btn"
                      title={t('accounts.groups.addAccounts')}
                      onClick={(e) => {
                        e.stopPropagation()
                        setGroupQuickAddGroupId(group.id)
                      }}
                    >
                      <FolderPlus size={14} />
                    </button>
                    <button
                      className="folder-icon-btn"
                      title={t('accounts.groups.editTitle')}
                      onClick={(e) => {
                        e.stopPropagation()
                        setGroupAccountPickerGroupId(group.id)
                      }}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="folder-icon-btn folder-delete-btn"
                      title={t('accounts.groups.deleteTitle')}
                      onClick={(e) => {
                        e.stopPropagation()
                        requestDeleteGroup(group.id, group.name)
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </td>
              </tr>
            )
          })}
          {groupByTag
            ? paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
              <Fragment key={groupKey}>
                <tr className="tag-group-row">
                  <td colSpan={5}>
                    <div className="tag-group-header">
                      <span className="tag-group-title">
                        {resolveGroupLabel(groupKey)}
                      </span>
                      <span className="tag-group-count">{totalCount}</span>
                    </div>
                  </td>
                </tr>
                {renderListRows(items, groupKey)}
              </Fragment>
            ))
            : renderListRows(paginatedAccounts)}
        </tbody>
      </table>
    </div>
  )

  return {
    accountGroups,
    accountNoteCopiedKey,
    accountNoteError,
    accountNoteErrorScrollKey,
    accountNoteFieldError,
    accountNoteMailPreview,
    accountNoteMailPreviewError,
    accountNoteMailPreviewLoading,
    accountNoteMfaPickerOpen,
    accountNoteOtpToken,
    accountNotePasswordVisible,
    accountNoteSecretVisible,
    accounts,
    activeAccountNoteEmail,
    activeAccountNoteForm,
    activeGroup,
    activeGroupId,
    addMessage,
    addStatus,
    addTab,
    addTargetGroup,
    allPaginatedSelected,
    ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH,
    ANTIGRAVITY_TOKEN_BATCH_EXAMPLE,
    ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE,
    antigravitySeamlessSwitchUnlocked,
    availableTags,
    clearFilterTypes,
    clearTagFilter,
    closeAccountNoteModal,
    closeAddModal,
    confirmClearSwitchHistory,
    confirmDelete,
    confirmDeleteGroup,
    confirmDeleteTag,
    copyAccountNoteValue,
    currentAccount,
    customSortAccounts,
    customSortDropTargetId,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    deleting,
    deletingGroup,
    deletingTag,
    displayGroups,
    draggedCustomSortAccountId,
    editingAccountNoteAccount,
    exportAccountIdsRef,
    exporting,
    exportModal,
    exportSelectionCount,
    exportSensitiveRefreshSeqRef,
    fetchAccountNoteMailPreviewForUrl,
    fileCorruptedError,
    filteredAccounts,
    filterTypes,
    formatAntigravityMailPreviewTime,
    formatDate,
    formatMfaRecordOption,
    formatSwitchHistoryAutoReason,
    formatSwitchHistoryOrigin,
    formatSwitchHistoryStage,
    formatSwitchHistoryTrigger,
    getQuotaDisplayItems,
    getVerificationBadge,
    groupAccountPickerGroup,
    groupAccountPickerGroupId,
    groupByTag,
    groupDeleteConfirm,
    groupDeleteError,
    groupDeleteErrorScrollKey,
    groupQuickAddGroup,
    groupQuickAddGroupId,
    handleAssignAccountsToGroup,
    handleBatchDelete,
    handleClearSwitchHistory,
    handleCopyOauthUrl,
    handleCustomSortDragMove,
    handleCustomSortDragStart,
    handleExport,
    handleImportFromExtension,
    handleImportFromFiles,
    handleImportFromLocal,
    handleImportFromTools,
    handleOAuthComplete,
    handleOAuthStart,
    handlePendingOAuthComplete,
    handlePendingOAuthStart,
    handleRefresh,
    handleRefreshAll,
    handleRemoveFromGroup,
    handleSaveAccountNote,
    handleSavePendingOAuthAccount,
    handleSaveTags,
    handleSortByChange,
    handleSubmitOauthCallbackUrl,
    handleTokenImport,
    handleViewModeChange,
    handleWakeupSelected,
    hasAntigravityAccountNoteFormDetails,
    hasVisibleAccountGroups,
    importing,
    includeExportSensitiveNotes,
    includeExportSensitiveNotesRef,
    isCustomSortActive,
    loading,
    locale,
    maskAccountText,
    message,
    mfaTimeRemaining,
    moveCustomSortAccount,
    oauthAccountNoteForm,
    oauthAccountNoteMode,
    oauthCallbackError,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthUrl,
    oauthUrlCopied,
    onNavigate,
    openAddModal,
    openOAuthAccountNoteModal,
    openSwitchHistoryModal,
    paginatedIds,
    pagination,
    pendingOAuthAccount,
    pendingOAuthEmailError,
    pendingOAuthEmailInput,
    privacyModeEnabled,
    refreshing,
    refreshingAll,
    reloadAccountGroups,
    renderCompactView,
    renderErrorMessage,
    renderGridView,
    renderListView,
    requestDeleteTag,
    resetAddModalState,
    resetCustomSortOrder,
    savedMfaRecords,
    savingAccountNote,
    savingPendingOAuthAccount,
    searchQuery,
    selected,
    setAccountNoteMfaPickerOpen,
    setAccountNotePasswordVisible,
    setAccountNoteSecretVisible,
    setActiveGroupId,
    setAddTab,
    setDeleteConfirm,
    setDeleteConfirmError,
    setFileCorruptedError,
    setGroupAccountPickerGroupId,
    setGroupByTag,
    setGroupDeleteConfirm,
    setGroupDeleteError,
    setGroupQuickAddGroupId,
    setIncludeExportSensitiveNotes,
    setMessage,
    setOauthCallbackInput,
    setPendingOAuthEmailError,
    setPendingOAuthEmailInput,
    setSavedMfaRecords,
    setSearchQuery,
    setSelected,
    setShowAccountGroupModal,
    setShowAddToGroupModal,
    setShowCustomSortModal,
    setShowErrorModal,
    setShowQuotaModal,
    setShowSwitchHistoryModal,
    setShowTagModal,
    setShowVerificationErrorModal,
    setSortDirection,
    setSwitchHistoryClearConfirmOpen,
    setTagDeleteConfirm,
    setTagDeleteConfirmError,
    setTokenInput,
    showAccountGroupModal,
    showAddModal,
    showAddToGroupModal,
    showCustomSortModal,
    showErrorModal,
    showQuotaModal,
    showSwitchHistoryModal,
    showTagModal,
    showVerificationErrorModal,
    sortBy,
    sortDirection,
    stopCustomSortDragging,
    switchHistory,
    switchHistoryClearConfirmOpen,
    switchHistoryClearing,
    switchHistoryLoading,
    t,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    tagFilter,
    tierCounts,
    tierFilterOptions,
    toggleFilterTypeValue,
    togglePrivacyMode,
    toggleSelectAll,
    toggleTagFilterValue,
    tokenInput,
    updateEditingAccountNoteForm,
    verificationDetailMap,
    verificationStatusMap,
    viewMode,
    wakeupRunning,
  };
}

/** 组合业务 Controller 与独立 View，保持原组件公开调用入口不变。 */
export function AccountsPage(props: AccountsPageProps) {
  const controller = useAccountsPageController(props);
  return <AccountsOverviewView {...controller} />;
}
