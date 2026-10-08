import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, Gauge, Search, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { proxyPickerPosition } from '../../utils/codexProxyPickerPosition';

export interface ProxySelectOption {
  value: string;
  label: string;
  detail?: string;
  badge?: ReactNode;
  disabled?: boolean;
  delay?: number;
  /** Keep policy choices before sortable node choices. */
  pinned?: boolean;
  measure?: { label: string; disabled: boolean; run: () => void };
}

export type ProxySelectSort = 'default' | 'name' | 'latency';

export interface CodexProxySelectProps {
  value: string;
  options: ProxySelectOption[];
  label: string;
  placeholder: string;
  searchPlaceholder?: string;
  disabled?: boolean;
  onChange: (id: string) => void;
  footer?: ReactNode;
  sortable?: boolean;
  measuring?: boolean;
}

function knownDelay(option: ProxySelectOption): number {
  return typeof option.delay === 'number' && Number.isFinite(option.delay) && option.delay >= 0
    ? option.delay : Number.POSITIVE_INFINITY;
}

/** Freeze displayed IDs during a batch; badges can still update without moving a row. */
export function orderProxySelectOptions(options: ProxySelectOption[], sort: ProxySelectSort, frozenIds?: string[]): ProxySelectOption[] {
  const positions = frozenIds && new Map(frozenIds.map((id, index) => [id, index]));
  return options.map((option, index) => ({ option, index })).sort((a, b) => {
    if (positions) {
      const before = positions.get(a.option.value) ?? positions.size + a.index;
      const after = positions.get(b.option.value) ?? positions.size + b.index;
      return before - after;
    }
    const pinned = Number(!!b.option.pinned) - Number(!!a.option.pinned);
    if (pinned) return pinned;
    if (sort === 'name') return a.option.label.localeCompare(b.option.label, undefined, { numeric: true }) || a.index - b.index;
    if (sort === 'latency') {
      const left = knownDelay(a.option); const right = knownDelay(b.option);
      if (left !== right) return left < right ? -1 : 1;
    }
    return a.index - b.index;
  }).map(({ option }) => option);
}

/** A local picker only: opening, filtering and sorting never invoke a network check. */
export function CodexProxySelect({ value, options, label, placeholder, searchPlaceholder, disabled = false, onChange, footer, sortable = false, measuring = false }: CodexProxySelectProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState<ProxySelectSort>('default');
  const [position, setPosition] = useState<CSSProperties>({});
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const ordering = useRef<{ sort: ProxySelectSort; ids: string[] } | undefined>(undefined);
  const focusSelection = useRef(false);
  const selected = options.find((option) => option.value === value);
  const ordered = useMemo(() => orderProxySelectOptions(options, sort,
    measuring && ordering.current?.sort === sort ? ordering.current.ids : undefined), [options, sort, measuring]);
  const visible = ordered.filter((option) => option.label.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const close = () => { setOpen(false); trigger.current?.focus({ preventScroll: true }); };

  // The shared stack runs at window capture before the underlying modal's Escape handler.
  useEscCloseTopmost(open, close);
  useLayoutEffect(() => {
    if (!measuring || ordering.current?.sort !== sort) ordering.current = { sort, ids: ordered.map((option) => option.value) };
  }, [ordered, sort, measuring]);
  useLayoutEffect(() => {
    if (!open) return;
    const reposition = () => {
      const rect = trigger.current?.getBoundingClientRect();
      if (!rect) return;
      const panel = menu.current;
      const list = panel?.querySelector<HTMLElement>('.codex-proxy-select-options');
      const height = panel && list ? panel.getBoundingClientRect().height - list.getBoundingClientRect().height + list.scrollHeight : 400;
      const width = Math.min(rect.width, Math.max(0, window.innerWidth - 12));
      setPosition({ ...proxyPickerPosition(rect.top, rect.bottom, window.innerHeight, height),
        left: Math.max(6, Math.min(rect.left, window.innerWidth - width - 6)), width });
    };
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !menu.current?.contains(event.target) && !trigger.current?.contains(event.target)) setOpen(false);
    };
    reposition();
    document.addEventListener('pointerdown', outside);
    window.addEventListener('resize', reposition);
    window.addEventListener('scroll', reposition, true);
    return () => {
      document.removeEventListener('pointerdown', outside);
      window.removeEventListener('resize', reposition);
      window.removeEventListener('scroll', reposition, true);
    };
  }, [open, query, options, footer, sortable]);
  useEffect(() => {
    if (!open) return;
    const initial = focusSelection.current
      ? menu.current?.querySelector<HTMLButtonElement>('.codex-proxy-select-option[aria-selected="true"]:not(:disabled)')
        ?? menu.current?.querySelector<HTMLButtonElement>('.codex-proxy-select-option:not(:disabled)')
      : menu.current?.querySelector<HTMLInputElement>('input');
    initial?.focus({ preventScroll: true });
    focusSelection.current = false;
  }, [open]);
  useEffect(() => { if (disabled) setOpen(false); }, [disabled]);

  const select = (option: ProxySelectOption) => {
    if (disabled || option.disabled) return;
    onChange(option.value); close();
  };
  const menuContent = open && <div ref={menu} role="dialog" aria-label={label} className="codex-proxy-select-menu" style={{ ...position, position: 'fixed' }}
    onKeyDown={(event) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); return; }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || ((event.key === 'Home' || event.key === 'End') && event.target !== menu.current?.querySelector('input'))) {
        event.preventDefault();
        const items = [...(menu.current?.querySelectorAll<HTMLButtonElement>('.codex-proxy-select-option:not(:disabled)') ?? [])];
        const index = items.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
          : index < 0 ? (event.key === 'ArrowDown' ? 0 : items.length - 1)
            : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
        items[next]?.focus();
      }
      if (event.key === 'Enter' && event.target === menu.current?.querySelector('input')) {
        const first = visible.find((option) => !option.disabled);
        if (first) { event.preventDefault(); select(first); }
      }
    }}
    onBlur={(event) => {
      if (event.relatedTarget instanceof Node && !menu.current?.contains(event.relatedTarget) && !trigger.current?.contains(event.relatedTarget)) setOpen(false);
    }}>
    <div className="codex-proxy-select-header">
      <label className="codex-proxy-select-search"><Search size={16} aria-hidden="true" />
        <input value={query} aria-label={searchPlaceholder ?? t('codex.proxy.catalog.search')} placeholder={searchPlaceholder ?? t('codex.proxy.catalog.search')}
          onChange={(event) => setQuery(event.target.value)} />
      </label>
      <button type="button" className="codex-proxy-select-tool" aria-label={t('common.close')} title={t('common.close')} onClick={close}><X size={17} aria-hidden="true" /></button>
    </div>
    {sortable && <div role="group" aria-label={t('codex.proxy.catalog.sortLabel')} className="codex-proxy-select-sorts">
      {(['default', 'name', 'latency'] as const).map((kind) => <button key={kind} type="button" className="codex-proxy-select-sort" aria-pressed={sort === kind}
        onClick={() => setSort(kind)}>{t(`codex.proxy.catalog.sortChoice_${kind}`)}</button>)}
    </div>}
    <div role="listbox" aria-label={label} className="codex-proxy-select-options">{visible.map((option) => <div className="codex-proxy-select-row" role="presentation" key={option.value}>
      <button type="button" role="option" aria-selected={value === option.value} disabled={option.disabled} className="codex-proxy-select-option"
        title={option.detail ? `${option.label}\n${option.detail}` : option.label} onClick={() => select(option)}>
        <span className="codex-proxy-select-name"><strong>{option.label}</strong>{option.detail && <small>{option.detail}</small>}</span>
        {option.badge}
        <span className="codex-proxy-select-check" aria-hidden="true">{value === option.value && <Check size={16} />}</span>
      </button>
      {option.measure && <button type="button" className="codex-proxy-select-tool" disabled={disabled || option.measure.disabled}
        aria-label={`${option.label} · ${option.measure.label}`} title={option.measure.label} onClick={option.measure.run}><Gauge size={17} aria-hidden="true" /></button>}
    </div>)}
      {!visible.length && <p className="codex-proxy-select-empty">{t('codex.proxy.catalog.noResults')}</p>}
    </div>
    {footer && <div className="codex-proxy-select-footer">{footer}</div>}
  </div>;
  return <div className="codex-proxy-select" data-open={open || undefined}>
    <span className="codex-proxy-select-label">{label}</span>
    <button ref={trigger} type="button" className="codex-proxy-select-trigger" disabled={disabled} aria-label={label} aria-haspopup="dialog" aria-expanded={open}
      title={selected?.label ?? placeholder} onClick={() => { setQuery(''); setOpen(!open); }}
      onKeyDown={(event) => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); focusSelection.current = true; setQuery(''); setOpen(true); }
      }}><span>{selected?.label ?? placeholder}</span>{selected?.badge}<ChevronDown size={18} aria-hidden="true" /></button>
    {menuContent && createPortal(menuContent, document.body)}
  </div>;
}
