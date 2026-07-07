import { ReactNode } from 'react';
import { Github } from 'lucide-react';
import { TFunction } from 'i18next';
import { PlatformId } from '../types/platform';
import { AntigravityIcon } from '../components/icons/AntigravityIcon';
import { AntigravityIdeIcon } from '../components/icons/AntigravityIdeIcon';
import { CodexIcon } from '../components/icons/CodexIcon';
import { ClaudeIcon } from '../components/icons/ClaudeIcon';
import { WindsurfIcon } from '../components/icons/WindsurfIcon';
import { KiroIcon } from '../components/icons/KiroIcon';
import { CursorIcon } from '../components/icons/CursorIcon';
import { GeminiIcon } from '../components/icons/GeminiIcon';
import { CodebuddyIcon } from '../components/icons/CodebuddyIcon';
import { QoderIcon } from '../components/icons/QoderIcon';
import { TraeCnIcon, TraeIcon, TraeSoloCnIcon, TraeSoloIcon } from '../components/icons/TraeIcon';
import { WorkbuddyIcon } from '../components/icons/WorkbuddyIcon';
import { ZedIcon } from '../components/icons/ZedIcon';

export function getPlatformLabel(platformId: PlatformId, _t: TFunction): string {
  switch (platformId) {
    case 'antigravity':
      return 'Antigravity';
    case 'antigravity_ide':
      return 'Antigravity IDE';
    case 'codex':
      return 'Codex';
    case 'claude_manager':
      return 'Claude';
    case 'zed':
      return 'Zed';
    case 'github-copilot':
      return 'GitHub Copilot';
    case 'windsurf':
      return 'Windsurf';
    case 'kiro':
      return 'Kiro';
    case 'cursor':
      return 'Cursor';
    case 'gemini':
      return 'Gemini Cli';
    case 'codebuddy':
      return 'CodeBuddy';
    case 'codebuddy_cn':
      return _t('nav.codebuddyCn', 'CodeBuddy CN');
    case 'qoder':
      return _t('nav.qoder', 'Qoder');
    case 'trae':
      return _t('nav.trae', 'Trae');
    case 'trae_solo':
      return _t('nav.traeSolo', 'TRAE SOLO');
    case 'trae_cn':
      return _t('nav.traeCn', 'Trae CN');
    case 'trae_solo_cn':
      return _t('nav.traeSoloCn', 'TRAE SOLO CN');
    case 'workbuddy':
      return 'WorkBuddy';
    default:
      return platformId;
  }
}

export function renderPlatformIcon(platformId: PlatformId, size = 20): ReactNode {
  switch (platformId) {
    case 'antigravity':
      return <AntigravityIcon style={{ width: size, height: size }} />;
    case 'antigravity_ide':
      return <AntigravityIdeIcon style={{ width: size, height: size }} />;
    case 'codex':
      return <CodexIcon size={size} />;
    case 'claude_manager':
      return <ClaudeIcon size={size} />;
    case 'zed':
      return <ZedIcon size={size} />;
    case 'github-copilot':
      return <Github size={size} />;
    case 'windsurf':
      return <WindsurfIcon style={{ width: size, height: size }} />;
    case 'kiro':
      return <KiroIcon style={{ width: size, height: size }} />;
    case 'cursor':
      return <CursorIcon style={{ width: size, height: size }} />;
    case 'gemini':
      return <GeminiIcon style={{ width: size, height: size }} />;
    case 'codebuddy':
      return <CodebuddyIcon style={{ width: size, height: size }} />;
    case 'codebuddy_cn':
      return <CodebuddyIcon style={{ width: size, height: size }} />;
    case 'qoder':
      return <QoderIcon style={{ width: size, height: size }} />;
    case 'trae':
      return <TraeIcon style={{ width: size, height: size }} />;
    case 'trae_solo':
      return <TraeSoloIcon style={{ width: size, height: size }} />;
    case 'trae_cn':
      return <TraeCnIcon style={{ width: size, height: size }} />;
    case 'trae_solo_cn':
      return <TraeSoloCnIcon style={{ width: size, height: size }} />;
    case 'workbuddy':
      return <WorkbuddyIcon style={{ width: size, height: size }} />;
    default:
      return null;
  }
}
