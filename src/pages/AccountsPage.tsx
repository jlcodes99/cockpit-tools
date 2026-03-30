import { useState, useEffect, useMemo, useRef, useCallback, Fragment } from 'react'
import { createPortal } from 'react-dom'
import {
  Plus,
  RefreshCw,
  Upload,
  Trash2,
  Rocket,
  X,
  Globe,
  KeyRound,
  Database,
  Plug,
  Copy,
  Check,
  LayoutGrid,
  List,
  Search,
  Fingerprint,
  Link,
  Lock,
  AlertTriangle,
  CircleAlert,
  Play,
  RotateCw,
  ArrowDownWideNarrow,
  Rows3,
  GripVertical,
  Eye,
  EyeOff,
  Tag,
  BookOpen,
  FileUp,
  ExternalLink,
  FolderOpen,
  FolderPlus,
  ChevronRight,
  LogOut,
  Pencil
} from 'lucide-react'
import { useTranslation, Trans } from 'react-i18next'
import { useAccountStore } from '../stores/useAccountStore'
import * as accountService from '../services/accountService'
import { FingerprintWithStats, Account } from '../types/account'
import { Page } from '../types/navigation'
import {
  getAntigravityTierBadge,
  getQuotaClass,
  formatResetTimeDisplay,
} from '../utils/account'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import { openUrl } from '@tauri-apps/plugin-opener'
import { TagEditModal } from '../components/TagEditModal'
import { ExportJsonModal } from '../components/ExportJsonModal'
import { AccountGroupModal, AddToGroupModal } from '../components/AccountGroupModal'
import { GroupAccountPickerModal } from '../components/GroupAccountPickerModal'
import { ModalErrorMessage, useModalErrorState } from '../components/ModalErrorMessage'
import {
  AccountGroup,
  getAccountGroups,
  assignAccountsToGroup,
  removeAccountsFromGroup,
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
  ANTIGRAVITY_ACCOUNTS_SORT_BY_STORAGE_KEY,
  ANTIGRAVITY_ACCOUNTS_SORT_DIRECTION_STORAGE_KEY,
  ANTIGRAVITY_RESET_SORT_PREFIX,
  DEFAULT_ANTIGRAVITY_SORT_BY,
  createAntigravityAccountComparator,
  normalizeAntigravitySortBy,
  normalizeAntigravitySortDirection,
} from '../utils/antigravityAccountSort'
import { OverviewTabsHeader } from '../components/OverviewTabsHeader'
import styles from '../styles/CompactView.module.css'
import { FileCorruptedModal, parseFileCorruptedError, type FileCorruptedError } from '../components/FileCorruptedModal'
import { QuickSettingsPopover } from '../components/QuickSettingsPopover'
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled
} from '../utils/privacy'
import { useExportJsonModal } from '../hooks/useExportJsonModal'
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown'
import { AccountTagFilterDropdown } from '../components/AccountTagFilterDropdown'
import {
  accountMatchesTagFilters,
  accountMatchesTypeFilters,
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
  collectAvailableAccountTags,
  normalizeAccountTag,
  type AccountFilterType,
} from '../utils/accountFilters'

interface AccountsPageProps {
  onNavigate?: (page: Page) => void
}

type ViewMode = 'grid' | 'list' | 'compact'

interface VerificationDetailRecord {
  status: string
  lastMessage?: string | null
  lastErrorCode?: number | null
  validationUrl?: string | null
  appealUrl?: string | null
}

interface VerificationHistoryRecord {
  accountId: string
  status: string
  lastMessage?: string | null
  lastErrorCode?: number | null
  validationUrl?: string | null
  appealUrl?: string | null
}

interface VerificationHistoryBatch {
  batchId: string
  verifiedAt: number
  records?: VerificationHistoryRecord[]
}

const buildVerificationHistoryMaps = (batches: VerificationHistoryBatch[] = []) => {
  const sorted = [...batches].sort((a, b) => b.verifiedAt - a.verifiedAt)
  const statusMap: Record<string, string> = {}
  const detailMap: Record<string, VerificationDetailRecord> = {}

  for (const batch of sorted) {
    for (const record of batch.records || []) {
      if (!(record.accountId in statusMap)) {
        statusMap[record.accountId] = record.status
        detailMap[record.accountId] = {
          status: record.status,
          lastMessage: record.lastMessage,
          lastErrorCode: record.lastErrorCode,
          validationUrl: record.validationUrl,
          appealUrl: record.appealUrl,
        }
      }
    }
  }

  return { statusMap, detailMap }
}

interface ExtensionImportProgressPayload {
  phase?: string
  current?: number
  total?: number
  email?: string
}

const ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE = `{"refresh_token":"1//0gAbCdEf..."}`
const ANTIGRAVITY_TOKEN_BATCH_EXAMPLE = `[
  {"refresh_token":"1//0gTokenA..."},
  {"refreshToken":"1//0gTokenB..."}
]`

export function AccountsPage({ onNavigate }: AccountsPageProps) {
  const { t, i18n } = useTranslation()
  const locale = i18n.language || 'zh-CN'
  const untaggedKey = '__untagged__'
  const {
    accounts,
    currentAccount,
    loading,
    error: storeError,
    fetchAccounts,
    fetchCurrentAccount,
    deleteAccounts,
    refreshQuota,
    refreshAllQuotas,
    startOAuthLogin,
    switchAccount,
    updateAccountTags
  } = useAccountStore()

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

  // View mode - persisted to localStorage
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    const saved = localStorage.getItem('accountsViewMode')
    return saved === 'grid' || saved === 'list' || saved === 'compact'
      ? saved
      : 'grid'
  })
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault()
  )

  // Persist view mode changes
  const handleViewModeChange = (mode: ViewMode) => {
    setViewMode(mode)
    localStorage.setItem('accountsViewMode', mode)
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
  const [filterTypes, setFilterTypes] = useState<AccountFilterType[]>([])
  const [tagFilter, setTagFilter] = useState<string[]>([])
  const [groupByTag, setGroupByTag] = useState(false)

  const toggleFilterTypeValue = useCallback((value: AccountFilterType) => {
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
  const exportModal = useExportJsonModal({
    exportFilePrefix: 'accounts_export',
    exportJsonByIds: accountService.exportAccounts,
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
  // 指纹选择弹框
  const [fingerprints, setFingerprints] = useState<FingerprintWithStats[]>([])
  const [showFpSelectModal, setShowFpSelectModal] = useState<string | null>(
    null
  )
  const [selectedFpId, setSelectedFpId] = useState<string | null>(null)
  const {
    message: fpSelectError,
    scrollKey: fpSelectErrorScrollKey,
    set: setFpSelectError,
  } = useModalErrorState()
  const originalFingerprint = fingerprints.find((fp) => fp.is_original)
  const selectableFingerprints = fingerprints.filter((fp) => !fp.is_original)

  // Quota Detail Modal
  const [showQuotaModal, setShowQuotaModal] = useState<string | null>(null)
  const [showErrorModal, setShowErrorModal] = useState<string | null>(null)
  const [showVerificationErrorModal, setShowVerificationErrorModal] = useState<string | null>(null)

  // 标签编辑弹窗
  const [showTagModal, setShowTagModal] = useState<string | null>(null)

  const [displayGroups, setDisplayGroups] = useState<DisplayGroup[]>([])
  const [displayGroupsLoaded, setDisplayGroupsLoaded] = useState(false)

  // ─── 账号分组（文件夹）────────────────────────────────────
  const [accountGroups, setAccountGroups] = useState<AccountGroup[]>([])
  const [activeGroupId, setActiveGroupId] = useState<string | null>(null)
  const [showAccountGroupModal, setShowAccountGroupModal] = useState(false)
  const [showAddToGroupModal, setShowAddToGroupModal] = useState(false)
  const [groupAccountPickerGroupId, setGroupAccountPickerGroupId] = useState<string | null>(null)

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

  const groupAccountPickerGroup = useMemo(() => {
    if (!groupAccountPickerGroupId) return null
    return accountGroups.find((group) => group.id === groupAccountPickerGroupId) || null
  }, [accountGroups, groupAccountPickerGroupId])

  // 离开已删除的分组
  useEffect(() => {
    if (activeGroupId && !accountGroups.find((g) => g.id === activeGroupId)) {
      setActiveGroupId(null)
    }
  }, [accountGroups, activeGroupId])
  const [sortBy, setSortBy] = useState<string>(() =>
    normalizeAntigravitySortBy(
      localStorage.getItem(ANTIGRAVITY_ACCOUNTS_SORT_BY_STORAGE_KEY)
    )
  )
  const [sortDirection, setSortDirection] = useState<'asc' | 'desc'>(() =>
    normalizeAntigravitySortDirection(
      localStorage.getItem(ANTIGRAVITY_ACCOUNTS_SORT_DIRECTION_STORAGE_KEY)
    )
  )

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
  const activeGroupIdRef = useRef(activeGroupId)
  const verificationHistoryRequestIdRef = useRef(0)
  const colorPickerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    showAddModalRef.current = showAddModal
    addTabRef.current = addTab
    oauthUrlRef.current = oauthUrl
    addStatusRef.current = addStatus
    activeGroupIdRef.current = activeGroupId
  }, [showAddModal, addTab, oauthUrl, addStatus, activeGroupId])

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

  useEffect(() => {
    localStorage.setItem(ANTIGRAVITY_ACCOUNTS_SORT_BY_STORAGE_KEY, sortBy)
  }, [sortBy])

  useEffect(() => {
    localStorage.setItem(
      ANTIGRAVITY_ACCOUNTS_SORT_DIRECTION_STORAGE_KEY,
      sortDirection
    )
  }, [sortDirection])

  useEffect(() => {
    if (!displayGroupsLoaded) {
      return
    }
    const normalizedSortBy = normalizeAntigravitySortBy(sortBy)
    if (
      normalizedSortBy === 'overall' ||
      normalizedSortBy === 'created_at' ||
      normalizedSortBy === 'default'
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

  const accountSortComparator = useMemo(
    () =>
      createAntigravityAccountComparator({
        sortBy,
        sortDirection,
        displayGroups,
      }),
    [displayGroups, sortBy, sortDirection]
  )

  const availableTags = useMemo(() => collectAvailableAccountTags(accounts), [accounts])

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
      const selectedTypes = new Set(filterTypes)
      result = result.filter((acc) =>
        accountMatchesTypeFilters(acc, selectedTypes, verificationStatusMap)
      )
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
      if (aKey === untaggedKey) return 1
      if (bKey === untaggedKey) return -1
      return aKey.localeCompare(bKey)
    })
  }, [filteredAccounts, groupByTag, tagFilter, untaggedKey])

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
    () => buildAccountTierFilterOptions(t, tierCounts),
    [t, tierCounts.FREE, tierCounts.PRO, tierCounts.TOS_VIOLATION, tierCounts.ULTRA, tierCounts.UNKNOWN, tierCounts.VERIFICATION_REQUIRED]
  )

  const loadFingerprints = async () => {
    try {
      const list = await accountService.listFingerprints()
      setFingerprints(list)
    } catch (e) {
      console.error(e)
    }
  }

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
    fetchCurrentAccount()
    loadFingerprints()
    loadDisplayGroups()
    loadVerificationHistory()

    let unlisten: UnlistenFn | undefined

    listen<string>('accounts:refresh', async () => {
      await fetchAccounts()
      await fetchCurrentAccount()
      const latestAccounts = useAccountStore.getState().accounts
      const accountsWithoutQuota = latestAccounts.filter(
        (acc) => !acc.quota?.models?.length
      )
      if (accountsWithoutQuota.length > 0) {
        await Promise.allSettled(
          accountsWithoutQuota.map((acc) => refreshQuota(acc.id))
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
        const newAccount = await accountService.completeOAuthLogin()
        await fetchAccounts()
        await fetchCurrentAccount()
        // 如果在文件夹内添加，自动归入当前文件夹
        if (activeGroupIdRef.current && newAccount?.id) {
          await assignAccountsToGroup(activeGroupIdRef.current, [newAccount.id])
          await reloadAccountGroups()
        }
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
  }, [fetchAccounts, fetchCurrentAccount])

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
      await refreshQuota(accountId)
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
          groupAccounts.map((acc) => refreshQuota(acc.id))
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
      setSelected((prev) => {
        if (prev.size === 0) return prev
        const next = new Set(prev)
        deleteConfirm.ids.forEach((id) => next.delete(id))
        return next
      })
      setDeleteConfirm(null)
      setDeleteConfirmError(null)
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

  const resetAddModalState = () => {
    setAddStatus('idle')
    setAddMessage('')
    setTokenInput('')
    setOauthUrlCopied(false)
    setOauthCallbackInput('')
    setOauthCallbackSubmitting(false)
    setOauthCallbackError(null)
  }

  const openAddModal = (tab: 'oauth' | 'token' | 'import') => {
    setAddTab(tab)
    setShowAddModal(true)
    resetAddModalState()
  }

  const closeAddModal = () => {
    // 允许用户随时关闭弹窗，取消正在进行的 OAuth 流程
    if (addStatus === 'loading') {
      accountService.cancelOAuthLogin().catch(() => { })
    }
    setShowAddModal(false)
    resetAddModalState()
    setOauthUrl('')
  }

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
      await startOAuthLogin()
      await fetchAccounts()
      await fetchCurrentAccount()
    })
  }

  const handleOAuthComplete = async () => {
    await runModalAction(t('modals.import.oauthAction'), async () => {
      await accountService.completeOAuthLogin()
      await fetchAccounts()
      await fetchCurrentAccount()
    })
  }

  const handleSwitch = async (accountId: string) => {
    setMessage(null)
    setSwitching(accountId)
    try {
      const account = await switchAccount(accountId)
      await fetchCurrentAccount()
      setMessage({ text: t('messages.switched', { email: maskAccountText(account.email) }) })
    } catch (e) {
      const raw = String(e)
      if (!raw.startsWith('APP_PATH_NOT_FOUND:')) {
        setMessage({
          text: t('messages.switchFailed', { error: raw }),
          tone: 'error'
        })
      }
    }
    setSwitching(null)
  }

  const handleImportFromTools = async () => {
    setImporting(true)
    setAddStatus('loading')
    setAddMessage(t('modals.import.importingTools'))
    try {
      const imported = await accountService.importFromOldTools()
      await fetchAccounts()
      await loadFingerprints()
      await Promise.allSettled(imported.map((acc) => refreshQuota(acc.id)))
      await fetchAccounts()
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
      await refreshQuota(imported.id)
      await fetchAccounts()
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
      await Promise.allSettled(imported.map((acc) => refreshQuota(acc.id)))
      await fetchAccounts()
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
      await fetchCurrentAccount()
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
        importedAccounts.map((acc) => refreshQuota(acc.id))
      )
      await fetchAccounts()
      // 如果在文件夹内添加，自动归入当前文件夹
      if (activeGroupId) {
        await assignAccountsToGroup(activeGroupId, importedAccounts.map((acc) => acc.id))
        await reloadAccountGroups()
      }
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
    if (selected.size === filteredAccounts.length) setSelected(new Set())
    else setSelected(new Set(filteredAccounts.map((a) => a.id)))
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

  const handleSaveTags = async (tags: string[], notes?: string) => {
    if (!showTagModal) return;
    const accountId = showTagModal
    await accountService.updateAccountNotes(accountId, notes ?? '')
    await updateAccountTags(accountId, tags);
    setShowTagModal(null);
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

  const openFpSelectModal = (accountId: string) => {
    const account = accounts.find((a) => a.id === accountId)
    setSelectedFpId(account?.fingerprint_id || 'original')
    setFpSelectError(null)
    setShowFpSelectModal(accountId)
  }

  const handleBindFingerprint = async () => {
    if (!showFpSelectModal || !selectedFpId) return
    try {
      setFpSelectError(null)
      await accountService.bindAccountFingerprint(
        showFpSelectModal,
        selectedFpId
      )
      await fetchAccounts()
      setShowFpSelectModal(null)
    } catch (e) {
      setFpSelectError(t('messages.bindFailed', { error: String(e) }))
    }
  }

  const getFingerprintName = (fpId?: string) => {
    if (!fpId || fpId === 'original') return t('modals.fingerprint.original')
    const fp = fingerprints.find((f) => f.id === fpId)
    return fp?.name || fpId
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

  const renderGridCards = (items: Account[], groupKey?: string) =>
    items.map((account) => {
      const isCurrent = currentAccount?.id === account.id
      const tierBadge = getAntigravityTierBadge(account.quota)
      const quotaDisplayItems = getQuotaDisplayItems(account)
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

      if (quotaDisplayItems.length === 0) {
        console.log('[AccountsPage] 账号无配额数据:', {
          email: account.email,
          isCurrent,
          hasQuota: !!account.quota,
          quotaModels: account.quota?.models,
          quotaModelsLength: account.quota?.models?.length,
          rawQuota: account.quota
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
            {(() => {
              const vBadge = getVerificationBadge(account)
              return vBadge ? (
                <span className={`verification-status-pill ${vBadge.className}`} title={vBadge.label}>
                  {vBadge.label}
                </span>
              ) : null
            })()}
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
                {quotaDisplayItems.map((item) => {
                  const resetLabel = formatResetTimeDisplay(item.resetTime, t)
                  return (
                    <div key={item.key} className="quota-compact-item">
                      <div className="quota-compact-header">
                        <span className="model-label">{item.label}</span>
                        <span
                          className={`model-pct ${getQuotaClass(item.percentage)}`}
                        >
                          {item.percentage}%
                        </span>
                      </div>
                      <div className="quota-compact-bar-track">
                        <div
                          className={`quota-compact-bar ${getQuotaClass(item.percentage)}`}
                          style={{ width: `${item.percentage}%` }}
                        />
                      </div>
                      {resetLabel && (
                        <span className="quota-compact-reset">{resetLabel}</span>
                      )}
                    </div>
                  )
                })}
                {quotaDisplayItems.length === 0 && (
                  <div className="quota-empty">{t('overview.noQuotaData')}</div>
                )}
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
                onClick={() => openFpSelectModal(account.id)}
                title={t('accounts.actions.fingerprint')}
              >
                <Fingerprint size={14} />
              </button>
              <button
                className="card-action-btn"
                onClick={() => openTagModal(account.id)}
                title={t('accounts.editTags', '编辑标签')}
              >
                <Tag size={14} />
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
        {filteredAccounts.length > 0 && (
          <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
              <input
                type="checkbox"
                checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0}
                onChange={toggleSelectAll}
              />
              {t('common.selectAll', '全选')}
            </label>
          </div>
        )}
        {!groupByTag ? (
          <div className="accounts-grid">
            {renderInlineFolderCards()}
            {renderGridCards(filteredAccounts)}
          </div>
        ) : (
          <div className="tag-group-list">
            {groupedAccounts.map(([groupKey, groupAccounts]) => (
              <div key={groupKey} className="tag-group-section">
                <div className="tag-group-header">
                  <span className="tag-group-title">
                    {resolveGroupLabel(groupKey)}
                  </span>
                  <span className="tag-group-count">{groupAccounts.length}</span>
                </div>
                <div className="tag-group-grid accounts-grid">
                  {renderGridCards(groupAccounts, groupKey)}
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
	              {groupedAccounts.map(([groupKey, groupAccounts]) => (
                <div key={groupKey} className="tag-group-section">
                  <div className="tag-group-header">
                    <span className="tag-group-title">
                      {resolveGroupLabel(groupKey)}
                    </span>
                    <span className="tag-group-count">
                      {groupAccounts.length}
                    </span>
                  </div>
                  <div className={`tag-group-grid ${styles.grid}`}>
                    {renderCompactCards(groupAccounts)}
                  </div>
                </div>
              ))}
            </div>
	          ) : (
	            <>
	              {hasVisibleAccountGroups && (
	                <div className="accounts-grid">{renderInlineFolderCards()}</div>
	              )}
	              <div className={styles.grid}>{renderCompactCards(filteredAccounts)}</div>
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
      const quotaDisplayItems = getQuotaDisplayItems(account)
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
            <button
              className="fp-select-btn"
              onClick={() => openFpSelectModal(account.id)}
              title={t('accounts.actions.selectFingerprint')}
            >
              <Fingerprint size={14} />
              <span className="fp-select-name">
                {getFingerprintName(account.fingerprint_id)}
              </span>
              <Link size={12} />
            </button>
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
                  {quotaDisplayItems.map((item) => (
                    <div className="quota-item" key={item.key}>
                      <div className="quota-header">
                        <span className="quota-name">{item.label}</span>
                        <span
                          className={`quota-value ${getQuotaClass(item.percentage)}`}
                        >
                          {item.percentage}%
                        </span>
                      </div>
                      <div className="quota-progress-track">
                        <div
                          className={`quota-progress-bar ${getQuotaClass(item.percentage)}`}
                          style={{ width: `${item.percentage}%` }}
                        />
                      </div>
                      <div className="quota-footer">
                        <span className="quota-reset">
                          {formatResetTimeDisplay(item.resetTime, t)}
                        </span>
                      </div>
                    </div>
                  ))}
                  {quotaDisplayItems.length === 0 && (
                    <span style={{ color: 'var(--text-muted)', fontSize: 13 }}>
                      {t('overview.noQuotaData')}
                    </span>
                  )}
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
                checked={
                  selected.size === filteredAccounts.length &&
                  filteredAccounts.length > 0
                }
                onChange={toggleSelectAll}
              />
            </th>
            <th style={{ width: 220 }}>{t('accounts.columns.email')}</th>
            <th style={{ width: 130 }}>{t('accounts.columns.fingerprint')}</th>
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
            ? groupedAccounts.map(([groupKey, groupAccounts]) => (
              <Fragment key={groupKey}>
                <tr className="tag-group-row">
                  <td colSpan={5}>
                    <div className="tag-group-header">
                      <span className="tag-group-title">
                        {resolveGroupLabel(groupKey)}
                      </span>
                      <span className="tag-group-count">{groupAccounts.length}</span>
                    </div>
                  </td>
                </tr>
                {renderListRows(groupAccounts, groupKey)}
              </Fragment>
            ))
            : renderListRows(filteredAccounts)}
        </tbody>
      </table>
    </div>
  )

  return (
    <>
      <main className="main-content accounts-page">
        <OverviewTabsHeader
          active="overview"
          onNavigate={onNavigate}
          onOpenManual={() => onNavigate?.('manual')}
          subtitle={t('overview.subtitle')}
        />

        {/* 面包屑：进入分组后显示 */}
        {activeGroup && (
          <div className="folder-breadcrumb">
            <button
              className="breadcrumb-back"
              onClick={() => {
                setActiveGroupId(null)
                setSelected(new Set())
              }}
            >
              <FolderOpen size={14} />
              {t('accounts.groups.allGroups')}
            </button>
            <ChevronRight size={14} className="breadcrumb-sep" />
            <span className="breadcrumb-current">
              {activeGroup.name}
              <span className="breadcrumb-count">({filteredAccounts.length})</span>
            </span>
            {selected.size > 0 && (
              <>
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={() => setShowAddToGroupModal(true)}
                  title={t('accounts.groups.moveToGroup')}
                >
                  <FolderPlus size={14} />
                  {t('accounts.groups.moveToGroup')} ({selected.size})
                </button>
                <button
                  className="btn btn-secondary breadcrumb-remove-btn"
                  onClick={handleRemoveFromGroup}
                  title={t('accounts.groups.removeFromGroup')}
                >
                  <LogOut size={14} />
                  {t('accounts.groups.removeFromGroup')} ({selected.size})
                </button>
              </>
            )}
          </div>
        )}

        {/* 分组文件夹已嵌入到 accounts-grid 内，此处不再单独显示 */}

        {/* 工具栏 */}
        <div className="toolbar">
          <div className="toolbar-left">
            <div className="search-box">
              <Search size={16} className="search-icon" />
              <input
                type="text"
                placeholder={t('accounts.search')}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>

            <div className="view-switcher">
              <button
                className={`view-btn ${viewMode === 'compact' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('compact')}
                title={t('accounts.view.compact')}
              >
                <Rows3 size={16} />
              </button>
              <button
                className={`view-btn ${viewMode === 'list' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('list')}
                title={t('accounts.view.list')}
              >
                <List size={16} />
              </button>
              <button
                className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`}
                onClick={() => handleViewModeChange('grid')}
                title={t('accounts.view.grid')}
              >
                <LayoutGrid size={16} />
              </button>
            </div>

            <MultiSelectFilterDropdown
              options={tierFilterOptions}
              selectedValues={filterTypes}
              allLabel={t('accounts.filter.all', { count: tierCounts.all })}
              filterLabel={t('accounts.filterLabel', '筛选')}
              clearLabel={t('accounts.clearFilter', '清空筛选')}
              emptyLabel={t('common.none', '暂无')}
              ariaLabel={t('accounts.filterLabel', '筛选')}
              onToggleValue={(value) => toggleFilterTypeValue(value as AccountFilterType)}
              onClear={clearFilterTypes}
            />

            <AccountTagFilterDropdown
              availableTags={availableTags}
              selectedTags={tagFilter}
              onToggleTag={toggleTagFilterValue}
              onClear={clearTagFilter}
              onDeleteTag={requestDeleteTag}
              groupByTag={groupByTag}
              onToggleGroupByTag={setGroupByTag}
            />
            {/* 排序下拉菜单 */}
            <div className="sort-select">
              <ArrowDownWideNarrow size={14} className="sort-icon" />
              <select
                value={sortBy}
                onChange={(e) => setSortBy(e.target.value)}
                aria-label={t('accounts.sortLabel', '排序')}
              >
                <option value="overall">
                  {t('accounts.sort.overall', '按综合配额')}
                </option>
                <option value="created_at">
                  {t('accounts.sort.createdAt', '按创建时间')}
                </option>
                {displayGroups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {t('accounts.sort.byGroup', {
                      group: group.name,
                      defaultValue: `按 ${group.name} 配额`
                    })}
                  </option>
                ))}
                {displayGroups.map(group => (
                  <option key={`${group.id}-reset`} value={`${ANTIGRAVITY_RESET_SORT_PREFIX}${group.id}`}>
                    {t('accounts.sort.byGroupReset', { group: group.name, defaultValue: `按 ${group.name} 重置时间` })}
                  </option>
                ))}
              </select>
            </div>

            {/* 排序方向切换按钮 */}
            <button
              className="sort-direction-btn"
              onClick={() =>
                setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))
              }
              title={
                sortDirection === 'desc'
                  ? t('accounts.sort.descTooltip', '当前：降序，点击切换为升序')
                  : t('accounts.sort.ascTooltip', '当前：升序，点击切换为降序')
              }
              aria-label={t('accounts.sort.toggleDirection', '切换排序方向')}
            >
              {sortDirection === 'desc' ? '⬇' : '⬆'}
            </button>
          </div>

          <div className="toolbar-right">
            <button
              className="btn btn-primary icon-only"
              onClick={() => openAddModal('oauth')}
              title={t('accounts.addAccount')}
              aria-label={t('accounts.addAccount')}
            >
              <Plus size={14} />
            </button>
            <button
              className="btn btn-secondary icon-only"
              onClick={handleRefreshAll}
              disabled={refreshingAll}
              title={t('accounts.refreshAll')}
              aria-label={t('accounts.refreshAll')}
            >
              <RefreshCw
                size={14}
                className={refreshingAll ? 'loading-spinner' : ''}
              />
            </button>
            <button
              className="btn btn-secondary icon-only"
              onClick={togglePrivacyMode}
              title={
                privacyModeEnabled
                  ? t('privacy.showSensitive', '显示邮箱')
                  : t('privacy.hideSensitive', '隐藏邮箱')
              }
              aria-label={
                privacyModeEnabled
                  ? t('privacy.showSensitive', '显示邮箱')
                  : t('privacy.hideSensitive', '隐藏邮箱')
              }
            >
              {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
            <button
              className="btn btn-secondary export-btn icon-only"
              onClick={handleExport}
              disabled={exporting || filteredAccounts.length === 0}
              title={
                exportSelectionCount > 0
                  ? `${t('accounts.export')} (${exportSelectionCount})`
                  : t('accounts.export')
              }
              aria-label={
                exportSelectionCount > 0
                  ? `${t('accounts.export')} (${exportSelectionCount})`
                  : t('accounts.export')
              }
            >
              <Upload size={14} />
            </button>
            {selected.size > 0 && (
              <>
                <button
                  className="btn btn-secondary icon-only"
                  onClick={() => setShowAddToGroupModal(true)}
                  title={t('accounts.groups.addToGroup')}
                  aria-label={t('accounts.groups.addToGroup')}
                >
                  <FolderPlus size={14} />
                </button>
                <button
                  className="btn btn-danger icon-only"
                  onClick={handleBatchDelete}
                  title={`${t('common.delete')} (${selected.size})`}
                  aria-label={`${t('common.delete')} (${selected.size})`}
                >
                  <Trash2 size={14} />
                </button>
              </>
            )}
            {!activeGroupId && (
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setShowAccountGroupModal(true)}
                title={t('accounts.groups.manageTitle')}
                aria-label={t('accounts.groups.manageTitle')}
              >
                <FolderOpen size={14} />
              </button>
            )}
            <QuickSettingsPopover type="antigravity" />
          </div>
        </div>

        {message && (
          <div
            className={`action-message${message.tone ? ` ${message.tone}` : ''}`}
          >
            <span className="action-message-text">{message.text}</span>
            <button
              className="action-message-close"
              onClick={() => setMessage(null)}
              aria-label={t('common.close')}
            >
              <X size={14} />
            </button>
          </div>
        )}

        {/* 内容区域 */}
        {loading ? (
          <div className="empty-state">
            <div
              className="loading-spinner"
              style={{ width: 40, height: 40 }}
            />
          </div>
        ) : accounts.length === 0 ? (
          <div className="empty-state">
            <div className="icon">
              <Rocket size={40} />
            </div>
            <h3>{t('accounts.empty.title')}</h3>
            <p>{t('accounts.empty.desc')}</p>
            <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
              <button
                className="btn btn-primary"
                onClick={() => openAddModal('oauth')}
              >
                <Plus size={18} />
                {t('accounts.empty.btn')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => onNavigate?.('manual')}
              >
                <BookOpen size={18} />
                {t('manual.navTitle', '查阅接入手册')}
              </button>
            </div>
          </div>
        ) : filteredAccounts.length === 0 && !hasVisibleAccountGroups ? (
          <div className="empty-state">
            <h3>{t('accounts.noMatch.title')}</h3>
            <p>{t('accounts.noMatch.desc')}</p>
          </div>
        ) : viewMode === 'grid' ? (
          renderGridView()
        ) : viewMode === 'list' ? (
          renderListView()
        ) : (
          renderCompactView()
        )}
      </main>

      {/* Add Account Modal */}
      {showAddModal && (
        <div className="modal-overlay" onClick={closeAddModal}>
          <div
            className="modal modal-lg add-account-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t('modals.addAccount.title')}</h2>
              <button className="close-btn" onClick={closeAddModal}>
                <X size={20} />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-tabs">
                <button
                  className={`add-tab ${addTab === 'oauth' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('oauth')
                    resetAddModalState()
                  }}
                >
                  <Globe size={14} /> {t('accounts.tabs.oauth')}
                </button>
                <button
                  className={`add-tab ${addTab === 'token' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('token')
                    resetAddModalState()
                  }}
                >
                  <KeyRound size={14} /> {t('common.shared.addModal.token', 'Token / JSON')}
                </button>
                <button
                  className={`add-tab ${addTab === 'import' ? 'active' : ''}`}
                  onClick={() => {
                    setAddTab('import')
                    resetAddModalState()
                  }}
                >
                  <Database size={14} /> {t('accounts.tabs.import')}
                </button>
              </div>

              {addTab === 'oauth' && (
                <div className="add-panel">
                  <div className="oauth-hint">
                    <Globe size={18} />
                    <span>{t('accounts.oauth.hint')}</span>
                  </div>
                  <div className="oauth-actions">
                    <button
                      className="btn btn-primary"
                      onClick={handleOAuthStart}
                      disabled={addStatus === 'loading'}
                    >
                      <Globe size={16} /> {t('accounts.oauth.start')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={handleOAuthComplete}
                      disabled={!oauthUrl || addStatus === 'loading'}
                    >
                      <Check size={16} /> {t('accounts.oauth.continue')}
                    </button>
                  </div>
                  <div className="oauth-link">
                    <label>{t('accounts.oauth.linkLabel')}</label>
                    <div className="oauth-link-row">
                      <input
                        type="text"
                        value={oauthUrl || t('accounts.oauth.generatingLink')}
                        readOnly
                      />
                      <button
                        className="btn btn-secondary icon-only"
                        onClick={handleCopyOauthUrl}
                        disabled={!oauthUrl}
                        title={t('common.copy')}
                      >
                        {oauthUrlCopied ? (
                          <Check size={14} />
                        ) : (
                          <Copy size={14} />
                        )}
                      </button>
                    </div>
                  </div>
                  <div className="oauth-link">
                    <label>{t('common.shared.oauth.manualCallbackLabel', '手动输入回调地址')}</label>
                    <div className="oauth-link-row oauth-manual-input">
                      <input
                        type="text"
                        value={oauthCallbackInput}
                        onChange={(e) => setOauthCallbackInput(e.target.value)}
                        placeholder={t('common.shared.oauth.manualCallbackPlaceholder', '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...')}
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={handleSubmitOauthCallbackUrl}
                        disabled={!oauthCallbackInput.trim() || oauthCallbackSubmitting}
                      >
                        {oauthCallbackSubmitting ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Check size={16} />
                        )}{' '}
                        {t('accounts.oauth.continue')}
                      </button>
                    </div>
                  </div>
                  {oauthCallbackError && (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>{oauthCallbackError}</span>
                    </div>
                  )}
                </div>
              )}

              {addTab === 'token' && (
                <div className="add-panel">
                  <p className="add-panel-desc">{t('accounts.token.desc')}</p>
                  <details className="token-format-collapse">
                    <summary className="token-format-collapse-summary">
                      {t('messages.example', 'Example')}
                    </summary>
                    <div className="token-format">
                      <p className="token-format-required">{t('accounts.token.desc')}</p>
                      <div className="token-format-group">
                        <div className="token-format-label">{`${t('messages.example', 'Example')} 1`}</div>
                        <pre className="token-format-code">{ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE}</pre>
                      </div>
                      <div className="token-format-group">
                        <div className="token-format-label">{`${t('messages.example', 'Example')} 2`}</div>
                        <pre className="token-format-code">{ANTIGRAVITY_TOKEN_BATCH_EXAMPLE}</pre>
                      </div>
                    </div>
                  </details>
                  <textarea
                    className="token-input"
                    placeholder={t('accounts.token.placeholder')}
                    value={tokenInput}
                    onChange={(e) => setTokenInput(e.target.value)}
                    rows={6}
                  />
                  <div className="modal-actions">
                    <button
                      className="btn btn-primary"
                      onClick={handleTokenImport}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <KeyRound size={14} /> {t('accounts.token.importStart')}
                    </button>
                  </div>
                </div>
              )}

              {addTab === 'import' && (
                <div className="add-panel">
                  <div className="import-options">
                    <button
                      className="import-option"
                      onClick={handleImportFromExtension}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Plug size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromExtension')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.syncBadge')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromLocal}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Database size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromLocalDB')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.localDBDesc')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromTools}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <Rocket size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.tools')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.toolsDescMigrate')}
                        </div>
                      </div>
                    </button>

                    <button
                      className="import-option"
                      onClick={handleImportFromFiles}
                      disabled={importing || addStatus === 'loading'}
                    >
                      <div className="import-option-icon">
                        <FileUp size={20} />
                      </div>
                      <div className="import-option-content">
                        <div className="import-option-title">
                          {t('modals.import.fromFiles')}
                        </div>
                        <div className="import-option-desc">
                          {t('modals.import.fromFilesDesc')}
                        </div>
                      </div>
                    </button>
                  </div>
                </div>
              )}

              {addMessage && (
                <div className={`add-feedback ${addStatus}`}>{addMessage}</div>
              )}
            </div>
          </div>
        </div>
      )}

      <ExportJsonModal
        isOpen={exportModal.showModal}
        title={`${t('accounts.export')} JSON`}
        jsonContent={exportModal.jsonContent}
        hidden={exportModal.hidden}
        copied={exportModal.copied}
        saving={exportModal.saving}
        savedPath={exportModal.savedPath}
        canOpenSavedDirectory={exportModal.canOpenSavedDirectory}
        pathCopied={exportModal.pathCopied}
        onClose={exportModal.closeModal}
        onToggleHidden={exportModal.toggleHidden}
        onCopyJson={exportModal.copyJson}
        onSaveJson={exportModal.saveJson}
        onOpenSavedDirectory={exportModal.openSavedDirectory}
        onCopySavedPath={exportModal.copySavedPath}
      />

      {deleteConfirm && (
        <div
          className="modal-overlay"
          onClick={() => {
            if (deleting) return
            setDeleteConfirm(null)
            setDeleteConfirmError(null)
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deleting) return
                  setDeleteConfirm(null)
                  setDeleteConfirmError(null)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} />
              <p>{deleteConfirm.message}</p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setDeleteConfirm(null)
                  setDeleteConfirmError(null)
                }}
                disabled={deleting}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDelete}
                disabled={deleting}
              >
                {t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {groupDeleteConfirm && (
        <div
          className="modal-overlay"
          onClick={() => {
            if (deletingGroup) return
            setGroupDeleteConfirm(null)
            setGroupDeleteError(null)
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('accounts.groups.deleteTitle')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deletingGroup) return
                  setGroupDeleteConfirm(null)
                  setGroupDeleteError(null)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={groupDeleteError} scrollKey={groupDeleteErrorScrollKey} />
              <p>
                {t('accounts.groups.deleteConfirm', {
                  name: groupDeleteConfirm.name,
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setGroupDeleteConfirm(null)
                  setGroupDeleteError(null)
                }}
                disabled={deletingGroup}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDeleteGroup}
                disabled={deletingGroup}
              >
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}

      {tagDeleteConfirm && (
        <div
          className="modal-overlay"
          onClick={() => {
            if (deletingTag) return
            setTagDeleteConfirm(null)
            setTagDeleteConfirmError(null)
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.confirm')}</h2>
              <button
                className="modal-close"
                onClick={() => {
                  if (deletingTag) return
                  setTagDeleteConfirm(null)
                  setTagDeleteConfirmError(null)
                }}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} />
              <p>
                {t('accounts.confirmDeleteTag', {
                  tag: tagDeleteConfirm.tag,
                  count: tagDeleteConfirm.count,
                  defaultValue: '确认删除标签 "{{tag}}" 吗？该标签将从 {{count}} 个账号中移除。',
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setTagDeleteConfirm(null)
                  setTagDeleteConfirmError(null)
                }}
                disabled={deletingTag}
              >
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-danger"
                onClick={confirmDeleteTag}
                disabled={deletingTag}
              >
                {deletingTag ? '处理中...' : t('common.confirm')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Fingerprint Selection Modal */}
      {showFpSelectModal && (
        <div
          className="modal-overlay"
          onClick={() => {
            setShowFpSelectModal(null)
            setFpSelectError(null)
          }}
        >
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('modals.fingerprint.title')}</h2>
              <button
                className="close-btn"
                onClick={() => {
                  setShowFpSelectModal(null)
                  setFpSelectError(null)
                }}
              >
                <X size={20} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={fpSelectError} scrollKey={fpSelectErrorScrollKey} />
              <p>
                <Trans
                  i18nKey="modals.fingerprint.desc"
                  values={{
                    email: maskAccountText(
                      accounts.find((a) => a.id === showFpSelectModal)?.email
                    )
                  }}
                  components={{ 1: <strong></strong> }}
                />
              </p>
              <div className="form-group">
                <label>{t('modals.fingerprint.selectLabel')}</label>
                <div className="fp-select-list">
                  <label
                    className={`fp-select-item ${selectedFpId === 'original' ? 'selected' : ''}`}
                  >
                    <input
                      type="radio"
                      name="fingerprint"
                      checked={selectedFpId === 'original'}
                      onChange={() => setSelectedFpId('original')}
                    />
                    <div className="fp-select-info">
                      <span className="fp-select-item-name">
                        📌 {t('modals.fingerprint.original')}
                      </span>
                      <span className="fp-select-item-id">
                        {t('modals.fingerprint.original')} ·{' '}
                        {originalFingerprint?.bound_account_count ?? 0}{' '}
                        {t('modals.fingerprint.boundCount')}
                      </span>
                    </div>
                  </label>
                  {selectableFingerprints.map((fp) => (
                    <label
                      key={fp.id}
                      className={`fp-select-item ${selectedFpId === fp.id ? 'selected' : ''}`}
                    >
                      <input
                        type="radio"
                        name="fingerprint"
                        checked={selectedFpId === fp.id}
                        onChange={() => setSelectedFpId(fp.id)}
                      />
                      <div className="fp-select-info">
                        <span className="fp-select-item-name">{fp.name}</span>
                        <span className="fp-select-item-id">
                          {fp.id.substring(0, 8)} · {fp.bound_account_count}{' '}
                          {t('modals.fingerprint.boundCount')}
                        </span>
                      </div>
                    </label>
                  ))}
                </div>
              </div>
              <div className="modal-actions">
                <button
                  className="btn btn-secondary"
                  onClick={() => {
                    setShowFpSelectModal(null)
                    setFpSelectError(null)
                    onNavigate?.('fingerprints')
                  }}
                >
                  <Plus size={14} /> {t('modals.fingerprint.new')}
                </button>
                <div style={{ flex: 1 }}></div>
                <button
                  className="btn btn-secondary"
                  onClick={() => {
                    setShowFpSelectModal(null)
                    setFpSelectError(null)
                  }}
                >
                  {t('common.cancel')}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={handleBindFingerprint}
                >
                  {t('common.confirm')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Quota Details Modal */}
      {showQuotaModal &&
        (() => {
          const account = accounts.find((a) => a.id === showQuotaModal)
          if (!account) return null
          const tierBadge = getAntigravityTierBadge(account.quota)
          const tierClass =
            tierBadge.tier === 'PRO' || tierBadge.tier === 'ULTRA'
              ? 'pill-success'
              : 'pill-secondary'

          return (
            <div
              className="modal-overlay"
              onClick={() => setShowQuotaModal(null)}
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t('modals.quota.title')}</h2>
                  <div className="badges">
                    <span className={`pill ${tierClass}`}>{tierBadge.label}</span>
                  </div>
                  <button
                    className="close-btn"
                    onClick={() => setShowQuotaModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  {(() => {
                    const quotaDisplayItems = getQuotaDisplayItems(account)
                    if (quotaDisplayItems.length === 0) {
                      return (
                        <div className="empty-state-small">
                          {t('overview.noQuotaData')}
                        </div>
                      )
                    }
                    return (
                      <div className="quota-list">
                        {quotaDisplayItems.map((item) => (
                          <div key={item.key} className="quota-card">
                            <h4>{item.label}</h4>
                            <div className="quota-value-row">
                              <span
                                className={`quota-value ${getQuotaClass(item.percentage)}`}
                              >
                                {item.percentage}%
                              </span>
                            </div>
                            <div className="quota-bar">
                              <div
                                className={`quota-fill ${getQuotaClass(item.percentage)}`}
                                style={{
                                  width: `${Math.min(100, item.percentage)}%`
                                }}
                              ></div>
                            </div>
                            <div className="quota-reset-info">
                              <p>
                                <strong>{t('modals.quota.resetTime')}:</strong>{' '}
                                {formatResetTimeDisplay(item.resetTime, t)}
                              </p>
                            </div>
                          </div>
                        ))}
                      </div>
                    )
                  })()}

                  <div className="modal-actions" style={{ marginTop: 20 }}>
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowQuotaModal(null)}
                    >
                      {t('common.close')}
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={() => {
                        handleRefresh(account.id)
                      }}
                    >
                      {refreshing.has(account.id) ? (
                        <div className="loading-spinner small" />
                      ) : (
                        <RefreshCw size={16} />
                      )}
                      {t('common.refresh')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* Error Details Modal */}
      {showErrorModal &&
        (() => {
          const account = accounts.find((a) => a.id === showErrorModal)
          if (!account) return null
          const errorInfo = account.quota_error

          return (
            <div
              className="modal-overlay"
              onClick={() => setShowErrorModal(null)}
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{t('modals.errors.title')}</h2>
                  <button
                    className="close-btn"
                    onClick={() => setShowErrorModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  {!errorInfo?.message ? (
                    <div className="empty-state-small">
                      {t('modals.errors.empty')}
                    </div>
                  ) : (
                    <div className="error-detail">
                      <div className="error-detail-meta">
                        <span>
                          {t('modals.errors.account')}: {maskAccountText(account.email)}
                        </span>
                        {errorInfo.code && (
                          <span>
                            {t('modals.errors.code')}: {errorInfo.code}
                          </span>
                        )}
                        {errorInfo.timestamp && (
                          <span>
                            {t('modals.errors.time')}:{' '}
                            {formatDate(errorInfo.timestamp)}
                          </span>
                        )}
                      </div>
                      <div className="error-detail-message">
                        {renderErrorMessage(errorInfo.message)}
                      </div>
                    </div>
                  )}

                  <div className="modal-actions" style={{ marginTop: 20 }}>
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowErrorModal(null)}
                    >
                      {t('common.close')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* Verification Error Modal (verification_required / tos_violation) */}
      {showVerificationErrorModal &&
        (() => {
          const account = accounts.find((a) => a.id === showVerificationErrorModal)
          if (!account) return null
          const vReason = account.disabled_reason || verificationStatusMap[account.id]
          const vDetail = verificationDetailMap[account.id]
          const isTos = vReason === 'tos_violation'
          const title = isTos
            ? t('wakeup.errorUi.tosViolationTitle', 'TOS 违规')
            : t('wakeup.errorUi.verificationRequiredTitle', '需要验证')

          const openLink = async (url: string) => {
            try {
              await openUrl(url)
            } catch {
              window.open(url, '_blank', 'noopener,noreferrer')
            }
          }

          const copyLink = async (url: string) => {
            try {
              await navigator.clipboard.writeText(url)
            } catch (e) {
              console.error('复制失败', e)
            }
          }

          return (
            <div
              className="modal-overlay"
              onClick={() => setShowVerificationErrorModal(null)}
            >
              <div
                className="modal modal-lg"
                onClick={(e) => e.stopPropagation()}
              >
                <div className="modal-header">
                  <h2>{title}</h2>
                  <button
                    className="close-btn"
                    onClick={() => setShowVerificationErrorModal(null)}
                  >
                    <X size={20} />
                  </button>
                </div>
                <div className="modal-body">
                  <div className="error-detail">
                    <div className="error-detail-meta">
                      <span>{t('modals.errors.account')}: {maskAccountText(account.email)}</span>
                      {vDetail?.lastErrorCode && (
                        <span>{t('wakeup.errorUi.errorCode', { code: vDetail.lastErrorCode })}</span>
                      )}
                    </div>
                    {vDetail?.lastMessage && (
                      <div className="error-detail-message" style={{ marginTop: 12 }}>
                        {vDetail.lastMessage}
                      </div>
                    )}
                  </div>
                  {!vDetail && (
                    <div className="empty-state-small" style={{ marginTop: 12 }}>
                      {t('modals.errors.empty', '暂无验证详情')}
                    </div>
                  )}

                  {/* Action buttons based on error type */}
                  <div className="modal-actions" style={{ marginTop: 20, gap: 8, flexWrap: 'wrap' }}>
                    {!isTos && vDetail?.validationUrl && (
                      <>
                        <button
                          className="btn btn-primary"
                          onClick={() => openLink(vDetail.validationUrl!)}
                        >
                          <ExternalLink size={14} />
                          {t('wakeup.errorUi.completeVerification', '立即验证')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => copyLink(vDetail.validationUrl!)}
                        >
                          <Copy size={14} />
                          {t('wakeup.errorUi.copyValidationUrl', '复制验证地址')}
                        </button>
                      </>
                    )}
                    {isTos && vDetail?.appealUrl && (
                      <>
                        <button
                          className="btn btn-primary"
                          onClick={() => openLink(vDetail.appealUrl!)}
                        >
                          <ExternalLink size={14} />
                          {t('wakeup.errorUi.submitAppeal', '立即提交保证书')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => copyLink(vDetail.appealUrl!)}
                        >
                          <Copy size={14} />
                          {t('wakeup.errorUi.copyAppealUrl', '复制链接')}
                        </button>
                      </>
                    )}
                    <button
                      className="btn btn-secondary"
                      onClick={() => setShowVerificationErrorModal(null)}
                    >
                      {t('common.close')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )
        })()}

      {/* 标签编辑弹窗 */}
      <TagEditModal
        isOpen={!!showTagModal}
        initialTags={accounts.find((acc) => acc.id === showTagModal)?.tags || []}
        initialNotes={accounts.find((acc) => acc.id === showTagModal)?.notes ?? ''}
        availableTags={availableTags}
        onClose={() => setShowTagModal(null)}
        onSave={handleSaveTags}
      />

      {/* 账号分组管理弹窗 */}
      <AccountGroupModal
        isOpen={showAccountGroupModal}
        onClose={() => setShowAccountGroupModal(false)}
        onGroupsChanged={reloadAccountGroups}
      />

      {/* 添加到分组弹窗 */}
      <AddToGroupModal
        isOpen={showAddToGroupModal}
        onClose={() => setShowAddToGroupModal(false)}
        accountIds={Array.from(selected)}
        sourceGroupId={activeGroupId || undefined}
        onAdded={async () => {
          await reloadAccountGroups()
          setSelected(new Set())
        }}
      />

      <GroupAccountPickerModal
        isOpen={!!groupAccountPickerGroupId}
        targetGroup={groupAccountPickerGroup}
        accounts={accounts}
        accountGroups={accountGroups}
        verificationStatusMap={verificationStatusMap}
        getVerificationBadge={getVerificationBadge}
        maskAccountText={maskAccountText}
        onClose={() => setGroupAccountPickerGroupId(null)}
        onConfirm={({ name, accountIds }) =>
          handleAssignAccountsToGroup(groupAccountPickerGroupId!, name, accountIds)
        }
      />

      {/* 文件损坏弹窗 */}
      {fileCorruptedError && (
        <FileCorruptedModal
          error={fileCorruptedError}
          onClose={() => setFileCorruptedError(null)}
        />
      )}
    </>
  )
}
