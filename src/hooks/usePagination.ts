import { useCallback, useEffect, useMemo, useState } from 'react';

const DEFAULT_PAGE_SIZE_OPTIONS = [20, 50, 100] as const;
const DEFAULT_PAGE_SIZE = DEFAULT_PAGE_SIZE_OPTIONS[0];

const normalizeStorageScope = (value: string) =>
  value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '_');

const normalizePageSizeOptions = (options?: readonly number[]) => {
  const source = options && options.length > 0 ? options : DEFAULT_PAGE_SIZE_OPTIONS;
  const unique = Array.from(
    new Set(
      source
        .map((value) => Math.floor(value))
        .filter((value) => Number.isFinite(value) && value > 0),
    ),
  );
  return unique.length > 0 ? unique : [...DEFAULT_PAGE_SIZE_OPTIONS];
};

const normalizePageSize = (value: number, options: readonly number[]) =>
  options.includes(value) ? value : options[0] ?? DEFAULT_PAGE_SIZE;

const readStoredPageSize = (storageKey: string, fallback: number) => {
  try {
    const raw = localStorage.getItem(storageKey);
    if (!raw) return fallback;
    const parsed = Number.parseInt(raw, 10);
    return Number.isFinite(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
};

const writeStoredPageSize = (storageKey: string, value: number) => {
  try {
    localStorage.setItem(storageKey, String(value));
  } catch {
    // ignore persistence failures
  }
};

export interface UsePaginationOptions<TItem> {
  items: readonly TItem[];
  storageKey: string;
  pageSizeOptions?: readonly number[];
  defaultPageSize?: number;
}

export interface UsePaginationReturn<TItem> {
  pageItems: TItem[];
  totalItems: number;
  currentPage: number;
  totalPages: number;
  pageSize: number;
  pageSizeOptions: readonly number[];
  rangeStart: number;
  rangeEnd: number;
  canGoPrevious: boolean;
  canGoNext: boolean;
  setCurrentPage: (page: number) => void;
  setPageSize: (pageSize: number) => void;
  goToPreviousPage: () => void;
  goToNextPage: () => void;
}

export interface PaginatedGroup<TItem> {
  groupKey: string;
  items: TItem[];
  totalCount: number;
}

export function buildPaginationPageSizeStorageKey(scope: string) {
  return `agtools.${normalizeStorageScope(scope)}.accounts_page_size`;
}

export function isEveryIdSelected(selected: ReadonlySet<string>, ids: readonly string[]) {
  return ids.length > 0 && ids.every((id) => selected.has(id));
}

export function buildPaginatedGroups<TItem extends { id: string }>(
  groupedEntries: ReadonlyArray<readonly [string, readonly TItem[]]>,
  pageItems: readonly TItem[],
): PaginatedGroup<TItem>[] {
  if (groupedEntries.length === 0 || pageItems.length === 0) {
    return [];
  }

  const pageIdSet = new Set(pageItems.map((item) => item.id));
  const result: PaginatedGroup<TItem>[] = [];

  groupedEntries.forEach(([groupKey, items]) => {
    const visibleItems = items.filter((item) => pageIdSet.has(item.id));
    if (visibleItems.length === 0) {
      return;
    }
    result.push({
      groupKey,
      items: visibleItems,
      totalCount: items.length,
    });
  });

  return result;
}

export function usePagination<TItem>({
  items,
  storageKey,
  pageSizeOptions,
  defaultPageSize,
}: UsePaginationOptions<TItem>): UsePaginationReturn<TItem> {
  const resolvedPageSizeOptions = useMemo(
    () => normalizePageSizeOptions(pageSizeOptions),
    [pageSizeOptions],
  );
  const resolvedDefaultPageSize = useMemo(
    () => normalizePageSize(defaultPageSize ?? DEFAULT_PAGE_SIZE, resolvedPageSizeOptions),
    [defaultPageSize, resolvedPageSizeOptions],
  );

  const [pageSize, setPageSizeState] = useState(() =>
    normalizePageSize(
      readStoredPageSize(storageKey, resolvedDefaultPageSize),
      resolvedPageSizeOptions,
    ),
  );
  const [currentPage, setCurrentPageState] = useState(1);

  useEffect(() => {
    setPageSizeState((prev) => normalizePageSize(prev, resolvedPageSizeOptions));
  }, [resolvedPageSizeOptions]);

  useEffect(() => {
    writeStoredPageSize(storageKey, pageSize);
  }, [pageSize, storageKey]);

  const totalItems = items.length;
  const totalPages = Math.max(1, Math.ceil(totalItems / pageSize));

  useEffect(() => {
    setCurrentPageState((prev) => Math.min(prev, totalPages));
  }, [totalPages]);

  const pageItems = useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    return items.slice(startIndex, startIndex + pageSize);
  }, [currentPage, items, pageSize]);

  const setCurrentPage = useCallback(
    (page: number) => {
      if (!Number.isFinite(page)) {
        return;
      }
      const normalized = Math.max(1, Math.min(totalPages, Math.floor(page)));
      setCurrentPageState(normalized);
    },
    [totalPages],
  );

  const setPageSize = useCallback(
    (nextPageSize: number) => {
      const normalizedPageSize = normalizePageSize(
        Math.floor(nextPageSize),
        resolvedPageSizeOptions,
      );
      setCurrentPageState((prevPage) =>
        Math.max(1, Math.floor(((prevPage - 1) * pageSize) / normalizedPageSize) + 1),
      );
      setPageSizeState(normalizedPageSize);
    },
    [pageSize, resolvedPageSizeOptions],
  );

  const goToPreviousPage = useCallback(() => {
    setCurrentPageState((prev) => Math.max(1, prev - 1));
  }, []);

  const goToNextPage = useCallback(() => {
    setCurrentPageState((prev) => Math.min(totalPages, prev + 1));
  }, [totalPages]);

  const rangeStart = totalItems === 0 ? 0 : (currentPage - 1) * pageSize + 1;
  const rangeEnd = totalItems === 0 ? 0 : Math.min(totalItems, currentPage * pageSize);

  return {
    pageItems,
    totalItems,
    currentPage,
    totalPages,
    pageSize,
    pageSizeOptions: resolvedPageSizeOptions,
    rangeStart,
    rangeEnd,
    canGoPrevious: currentPage > 1,
    canGoNext: currentPage < totalPages,
    setCurrentPage,
    setPageSize,
    goToPreviousPage,
    goToNextPage,
  };
}
