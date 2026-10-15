/** Local reminder preferences always take precedence over remote prompt policy. */
export class UpdatePromptPolicy {
  enabled = false;
  revision = 0;
  private source: 'auto' | 'manual' | null = null;
  private readonly promptedVersions = new Set<string>();

  applyPreference(enabled: boolean, revision?: number): boolean {
    if (revision !== undefined && revision !== this.revision) return false;
    this.enabled = enabled;
    this.revision += 1;
    return true;
  }

  shouldCloseAutomaticPrompt(): boolean {
    return !this.enabled && this.source === 'auto';
  }

  openAutomatic(version: string, mode: string): boolean {
    if (!this.enabled || mode !== 'popup' || this.source === 'manual'
      || this.promptedVersions.has(version)) return false;
    this.promptedVersions.add(version);
    this.source = 'auto';
    return true;
  }

  openManual(): void {
    this.source = 'manual';
  }

  close(): void {
    this.source = null;
  }
}
