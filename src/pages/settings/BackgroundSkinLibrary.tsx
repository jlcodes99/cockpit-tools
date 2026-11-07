import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Check, MoreHorizontal, Plus } from 'lucide-react';
import { BackgroundSurface } from '../../components/AppBackground';
import type { BackgroundSkin } from '../../services/appBackgroundService';

interface SkinCardProps {
  skin: BackgroundSkin;
  selected: boolean;
  disabled: boolean;
  onSelect: (id: string) => void;
  onReplace: (id: string) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}

export function BackgroundSkinCard({ skin, selected, disabled, onSelect, onReplace, onRename, onDelete }: SkinCardProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const label = skin.name || t('settings.background.custom');
  useEffect(() => {
    const next = URL.createObjectURL(skin.image);
    setUrl(next);
    return () => URL.revokeObjectURL(next);
  }, [skin.image]);
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setMenu(false);
    };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [menu]);
  return <div className="background-skin-card" ref={root} onKeyDown={event => {
    if (event.key === 'Escape') { setMenu(false); trigger.current?.focus(); }
  }}>
    <button type="button" className={`background-preset ${selected ? 'selected' : ''}`} disabled={disabled}
      aria-pressed={selected} onClick={() => { setMenu(false); onSelect(skin.id); }}>
      <span className="background-preset-swatch">
        <BackgroundSurface settings={{ ...skin.settings, strength: 85, blur: 0 }} imageUrl={url} />
        {selected && <Check className="background-skin-check" size={16} aria-hidden="true" />}
      </span>
      <span className="background-skin-name" title={label}>{label}</span>
    </button>
    <button type="button" className="btn btn-secondary background-skin-menu-trigger" ref={trigger} disabled={disabled} data-skin-manage={skin.id}
      aria-label={t('settings.background.manageSkin', { name: label })} aria-expanded={menu}
      onClick={() => setMenu(!menu)}><MoreHorizontal size={16} /></button>
    {menu && <div className="background-skin-menu">
      <button type="button" className="btn btn-secondary" onClick={() => { setMenu(false); onReplace(skin.id); }}>{t('settings.background.replaceImage')}</button>
      <button type="button" className="btn btn-secondary" onClick={() => { setMenu(false); onRename(skin.id, label); }}>{t('settings.background.rename')}</button>
      <button type="button" className="btn btn-secondary" onClick={() => { setMenu(false); onDelete(skin.id); }}>{t('common.delete')}</button>
    </div>}
  </div>;
}

interface LibraryProps extends Omit<SkinCardProps, 'skin' | 'selected'> {
  skins: BackgroundSkin[];
  activeSkinId: string | null;
  onAdd: () => void;
}

export function BackgroundSkinLibrary({ skins, activeSkinId, onAdd, ...actions }: LibraryProps) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<{ kind: 'rename' | 'delete'; id: string } | null>(null);
  const [name, setName] = useState('');
  const nameId = useId();
  const root = useRef<HTMLDivElement>(null);
  const closeEditor = () => {
    const trigger = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('[data-skin-manage]') ?? [])
      .find(button => button.dataset.skinManage === editing?.id);
    setEditing(null);
    trigger?.focus();
  };
  return <div className="background-skin-library" ref={root} onKeyDown={event => {
    if (event.key === 'Escape' && editing) { event.stopPropagation(); closeEditor(); }
  }}>
    <h4>{t('settings.background.mySkins')}</h4>
    <div className="background-presets" role="group" aria-label={t('settings.background.mySkins')}>
      {skins.map(skin => <BackgroundSkinCard key={skin.id} skin={skin} selected={activeSkinId === skin.id} {...actions}
        onRename={(id, name) => { setName(name); setEditing({ kind: 'rename', id }); }}
        onDelete={id => setEditing({ kind: 'delete', id })} />)}
      <button type="button" className="background-preset background-preset-upload" disabled={actions.disabled} onClick={() => { setEditing(null); onAdd(); }}>
        <span className="background-preset-swatch"><Plus className="background-preset-add" size={24} aria-hidden="true" /></span>
        <span>{t('settings.background.addSkin')}</span>
      </button>
    </div>
    {editing && createPortal(<div className="modal-overlay background-skin-dialog-overlay">
      <form className="modal background-skin-dialog" role="dialog" aria-modal="true"
        aria-labelledby={`${nameId}-title`} onSubmit={event => {
          event.preventDefault();
          if (actions.disabled) return;
          if (editing.kind === 'rename') {
            if (!name.trim()) return;
            actions.onRename(editing.id, name);
          } else {
            actions.onDelete(editing.id);
          }
          closeEditor();
        }}>
        <div className="modal-header"><h2 id={`${nameId}-title`}>{editing.kind === 'rename' ? t('settings.background.rename') : t('common.delete')}</h2></div>
        <div className="modal-body background-skin-dialog-body">
          {editing.kind === 'rename' ? <>
            <label htmlFor={nameId}>{t('settings.background.skinName')}</label>
            <input id={nameId} className="background-skin-name-input" value={name} maxLength={80} autoFocus disabled={actions.disabled}
              onChange={event => setName(event.target.value)} />
          </> : <p>{t('settings.background.deleteSkinConfirm', { name: skins.find(skin => skin.id === editing.id)?.name || t('settings.background.custom') })}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" autoFocus={editing.kind === 'delete'} onClick={closeEditor}>{t('common.cancel')}</button>
          <button type="submit" className={`btn ${editing.kind === 'rename' ? 'btn-primary' : 'btn-danger'}`}
            disabled={actions.disabled || (editing.kind === 'rename' && !name.trim())}>{editing.kind === 'rename' ? t('common.save') : t('common.delete')}</button>
        </div>
      </form>
    </div>, document.body)}
  </div>;
}
