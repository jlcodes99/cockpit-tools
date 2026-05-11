import { invoke } from '@tauri-apps/api/core';

export type CodexExportFormat = 'cockpit_tools' | 'sub2api' | 'cpa';

export interface CodexExportDocument {
  id: string;
  label: string;
  fileNameBase: string;
  jsonContent: string;
}

export type CodexExportContent =
  | {
      type: 'single';
      fileNameBase: string;
      jsonContent: string;
    }
  | {
      type: 'multiple';
      fileNameBase: string;
      documents: CodexExportDocument[];
    };

export function buildCodexExportFileNameBase(
  baseName: string,
  format: CodexExportFormat,
): string {
  if (format === 'cockpit_tools') {
    return baseName;
  }
  return `${baseName}_${format}`;
}

export async function buildCodexExportContent(
  rawJson: string,
  format: CodexExportFormat,
  baseName: string,
): Promise<CodexExportContent> {
  return await invoke<CodexExportContent>('codex_build_export_content', {
    rawJson,
    format,
    baseName,
  });
}
