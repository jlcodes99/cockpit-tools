import { useCallback, useMemo, useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';

interface UseExportJsonModalOptions {
  exportFilePrefix: string;
  exportJsonByIds: (ids: string[]) => Promise<string>;
  onError?: (error: unknown) => void;
}

interface UseExportJsonModalReturn {
  preparing: boolean;
  showModal: boolean;
  jsonContent: string;
  hidden: boolean;
  copied: boolean;
  saving: boolean;
  savedPath: string | null;
  savedDirectory: string | null;
  canOpenSavedDirectory: boolean;
  pathCopied: boolean;
  startExport: (ids: string[], fileNameBase?: string) => Promise<void>;
  closeModal: () => void;
  toggleHidden: () => void;
  copyJson: () => Promise<void>;
  saveJson: () => Promise<void>;
  openSavedDirectory: () => Promise<void>;
  copySavedPath: () => Promise<void>;
  resolveDefaultExportPath: (fileName: string) => Promise<string>;
  saveJsonFile: (json: string, defaultFileName: string) => Promise<string | null>;
  replaceJsonContent: (json: string) => void;
}

const JSON_EXTENSION_REGEX = /\.json$/i;
const INVALID_FILE_CHARS_REGEX = /[<>:"/\\|?*\x00-\x1F]/g;

function sanitizeFileBaseName(input: string | undefined, fallback: string): string {
  const raw = (input || '').trim().replace(JSON_EXTENSION_REGEX, '');
  const normalized = raw
    .replace(INVALID_FILE_CHARS_REGEX, '_')
    .replace(/\s+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_+|_+$/g, '');
  return normalized || fallback;
}

function buildExportFileName(baseName: string): string {
  const date = new Date().toISOString().slice(0, 10);
  return `${baseName}_${date}.json`;
}

function getDirectoryPath(filePath: string): string {
  const slashIndex = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\'));
  if (slashIndex <= 0) {
    return filePath;
  }
  return filePath.slice(0, slashIndex);
}

export function useExportJsonModal(options: UseExportJsonModalOptions): UseExportJsonModalReturn {
  const { exportFilePrefix, exportJsonByIds, onError } = options;

  const [preparing, setPreparing] = useState(false);
  const [showModal, setShowModal] = useState(false);
  const [jsonContent, setJsonContent] = useState('');
  const [hidden, setHidden] = useState(true);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [downloadsDir, setDownloadsDir] = useState<string | null>(null);
  const [pathCopied, setPathCopied] = useState(false);
  const [defaultFileName, setDefaultFileName] = useState(() => buildExportFileName(exportFilePrefix));

  const resolveDefaultExportPath = useCallback(async (fileName: string) => {
    const userAgent = navigator.userAgent.toLowerCase();
    if (!userAgent.includes('mac')) return fileName;
    try {
      const dir = await invoke<string>('get_downloads_dir');
      if (!dir) return fileName;
      const normalized = dir.endsWith('/') ? dir.slice(0, -1) : dir;
      return `${normalized}/${fileName}`;
    } catch (error) {
      console.error('获取下载目录失败:', error);
      return fileName;
    }
  }, []);

  const loadDownloadsDir = useCallback(async () => {
    try {
      const dir = await invoke<string>('get_downloads_dir');
      if (!dir) return null;
      const normalized = dir.replace(/\\/g, '/').replace(/\/+$/, '');
      setDownloadsDir(normalized);
      return normalized;
    } catch (error) {
      console.error('获取下载目录失败:', error);
      return null;
    }
  }, []);

  const saveJsonFile = useCallback(
    async (json: string, fileName: string) => {
      const defaultPath = await resolveDefaultExportPath(fileName);
      const filePath = await save({
        defaultPath,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!filePath) return null;
      await invoke('save_text_file', { path: filePath, content: json });
      return filePath;
    },
    [resolveDefaultExportPath],
  );

  const startExport = useCallback(
    async (ids: string[], fileNameBase?: string) => {
      if (!ids.length) return;
      setPreparing(true);
      try {
        if (!downloadsDir) {
          await loadDownloadsDir();
        }
        const json = await exportJsonByIds(ids);
        const safeBaseName = sanitizeFileBaseName(fileNameBase, exportFilePrefix);
        setDefaultFileName(buildExportFileName(safeBaseName));
        setJsonContent(json);
        setHidden(true);
        setCopied(false);
        setSavedPath(null);
        setPathCopied(false);
        setShowModal(true);
      } catch (error) {
        onError?.(error);
      } finally {
        setPreparing(false);
      }
    },
    [downloadsDir, exportFilePrefix, exportJsonByIds, loadDownloadsDir, onError],
  );

  const closeModal = useCallback(() => {
    setShowModal(false);
    setHidden(true);
    setCopied(false);
  }, []);

  const replaceJsonContent = useCallback((json: string) => {
    setJsonContent(json);
    setCopied(false);
    setSavedPath(null);
    setPathCopied(false);
  }, []);

  const toggleHidden = useCallback(() => {
    setHidden((prev) => !prev);
  }, []);

  const copyJson = useCallback(async () => {
    if (!jsonContent) return;
    try {
      await navigator.clipboard.writeText(jsonContent);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch (error) {
      onError?.(error);
    }
  }, [jsonContent, onError]);

  const saveJson = useCallback(async () => {
    if (!jsonContent || saving) return;
    setSaving(true);
    try {
      const filePath = await saveJsonFile(jsonContent, defaultFileName);
      if (filePath) {
        setSavedPath(filePath);
        setPathCopied(false);
      }
    } catch (error) {
      onError?.(error);
    } finally {
      setSaving(false);
    }
  }, [defaultFileName, jsonContent, onError, saveJsonFile, saving]);

  const savedDirectory = useMemo(() => {
    if (!savedPath) return null;
    return getDirectoryPath(savedPath);
  }, [savedPath]);

  const canOpenSavedDirectory = useMemo(() => {
    if (!savedDirectory || !downloadsDir) return false;
    const normalizedSaved = savedDirectory.replace(/\\/g, '/').replace(/\/+$/, '');
    return normalizedSaved === downloadsDir || normalizedSaved.startsWith(`${downloadsDir}/`);
  }, [downloadsDir, savedDirectory]);

  const openSavedDirectory = useCallback(async () => {
    if (!savedDirectory || !canOpenSavedDirectory) return;
    try {
      await openPath(savedDirectory);
    } catch (error) {
      onError?.(error);
    }
  }, [canOpenSavedDirectory, onError, savedDirectory]);

  const copySavedPath = useCallback(async () => {
    if (!savedPath) return;
    try {
      await navigator.clipboard.writeText(savedPath);
      setPathCopied(true);
      window.setTimeout(() => setPathCopied(false), 1200);
    } catch (error) {
      onError?.(error);
    }
  }, [onError, savedPath]);

  return {
    preparing,
    showModal,
    jsonContent,
    hidden,
    copied,
    saving,
    savedPath,
    savedDirectory,
    canOpenSavedDirectory,
    pathCopied,
    startExport,
    closeModal,
    toggleHidden,
    copyJson,
    saveJson,
    openSavedDirectory,
    copySavedPath,
    resolveDefaultExportPath,
    saveJsonFile,
    replaceJsonContent,
  };
}
