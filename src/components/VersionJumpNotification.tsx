import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { VersionJumpInfo } from '../types/updater';
import { X, Sparkles, PartyPopper } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useEscClose } from '../hooks/useEscClose';
import { prependUpdaterReleaseHighlights } from '../utils/updaterReleaseNotes';
import './UpdateNotification.css';

interface VersionJumpNotificationProps {
  info: VersionJumpInfo;
  onClose: () => void;
}

export const VersionJumpNotification: React.FC<VersionJumpNotificationProps> = ({
  info,
  onClose,
}) => {
  const { t, i18n } = useTranslation();
  const [localNotes, setLocalNotes] = useState<{ release_notes: string; release_notes_zh: string } | null>(null);

  useEffect(() => {
    let active = true;
    let pendingLoaded = false;
    setLocalNotes(null);
    type Notes = { release_notes: string; release_notes_zh: string };
    void invoke<Notes>('get_bundled_release_notes', { version: info.current_version })
      .then((notes) => { if (active && !pendingLoaded) setLocalNotes(notes); })
      .catch((error) => { console.error('Failed to read bundled release notes:', error); });
    void invoke<Notes | null>('get_pending_release_notes', { version: info.current_version })
      .then((notes) => { if (active && notes) { pendingLoaded = true; setLocalNotes(notes); } })
      .catch((error) => { console.error('Failed to read pending release notes:', error); });
    return () => { active = false; };
  }, [info.current_version]);

  useEscClose(true, onClose);

  useEffect(() => {
    const perfWindow = window as Window & {
      __agtoolsVersionJumpModalRequestedAt?: number;
    };
    const requestedAt = perfWindow.__agtoolsVersionJumpModalRequestedAt;
    if (typeof requestedAt === 'number') {
      console.log(
        `[StartupPerf][VersionJumpModal] mounted ${(performance.now() - requestedAt).toFixed(2)}ms after setVersionJumpInfo`,
      );
      delete perfWindow.__agtoolsVersionJumpModalRequestedAt;
      return;
    }

    console.log('[StartupPerf][VersionJumpModal] mounted without request timestamp');
  }, []);

  const releaseNotes = useMemo(() => {
    const isZh = i18n.language.startsWith('zh');
    const notes = isZh && localNotes?.release_notes_zh
      ? localNotes.release_notes_zh
      : localNotes?.release_notes || '';
    return prependUpdaterReleaseHighlights(
      info.current_version,
      notes,
      i18n.language,
    );
  }, [info.current_version, localNotes, i18n.language]);

  const formattedNotes = useMemo(() => {
    const formatStartedAt = performance.now();
    if (!releaseNotes) return null;

    const lines = releaseNotes.split('\n');
    const elements: React.ReactNode[] = [];
    let key = 0;

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;

      if (trimmed.startsWith('### ')) {
        elements.push(
          <h4 key={key++} className="release-notes-heading">
            {trimmed.slice(4)}
          </h4>
        );
      } else if (trimmed.startsWith('## ')) {
        continue;
      } else if (trimmed.startsWith('- ')) {
        const content = trimmed.slice(2);
        const parts = content.split(/\*\*(.*?)\*\*/g);
        elements.push(
          <li key={key++} className="release-notes-item">
            {parts.map((part, i) =>
              i % 2 === 1 ? <strong key={i}>{part}</strong> : part
            )}
          </li>
        );
      }
    }

    console.log(
      `[StartupPerf][VersionJumpModal] formatted ${elements.length} release note nodes in ${(performance.now() - formatStartedAt).toFixed(2)}ms`,
    );
    return elements.length > 0 ? (
      <ul className="release-notes-list">{elements}</ul>
    ) : null;
  }, [releaseNotes]);

  return (
    <div className="modal-overlay update-overlay">
      <div className="modal update-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2 className="update-modal-title">
            <span className="update-icon version-jump-icon">
              <PartyPopper size={18} />
            </span>
            {t('update_notification.versionJumpTitle', '🎉 Successfully Updated!')}
          </h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.cancel')}>
            <X size={18} />
          </button>
        </div>
        <div className="modal-body update-modal-body">
          <div className="update-version">v{info.current_version}</div>
          <p className="update-message">
            {t('update_notification.versionJumpMessage', 'Updated from v{{previous}} to v{{current}}', {
              previous: info.previous_version,
              current: info.current_version,
            })}
          </p>

          {formattedNotes && (
            <div className="release-notes">
              <h3 className="release-notes-title">
                {t('update_notification.whatsNew', "What's New")}
              </h3>
              <div className="release-notes-content">
                {formattedNotes}
              </div>
            </div>
          )}
        </div>
        <div className="modal-footer">
          <button className="btn btn-primary" onClick={onClose}>
            <Sparkles size={16} />
            {t('update_notification.gotIt', 'Got it!')}
          </button>
        </div>
      </div>
    </div>
  );
};
