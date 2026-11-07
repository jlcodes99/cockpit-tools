export interface VersionJumpInfo {
  previous_version: string;
  current_version: string;
}

export type UpdateCheckSource = 'auto' | 'manual';
export type UpdateActionState = 'hidden' | 'available' | 'downloading' | 'installing' | 'ready';

export type UpdateRuntimeInfo = {
  platform: string;
  linux_install_kind: string;
  linux_managed_install_supported: boolean;
  updater_target?: string | null;
};

export type LinuxUpdateProgressPhase =
  | 'download_started'
  | 'downloading'
  | 'downloaded'
  | 'auth_required'
  | 'installing'
  | 'completed';

export type LinuxUpdateProgressPayload = {
  version: string;
  phase: LinuxUpdateProgressPhase;
  progress?: number | null;
};

export type UpdateAction = {
  state: UpdateActionState;
  version: string | null;
  progress: number;
  requiresInstall: boolean;
};
