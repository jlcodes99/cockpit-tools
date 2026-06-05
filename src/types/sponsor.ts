export interface Sponsor {
  id: string;
  name: string;
  priority: number;
  logoUrl?: string | null;
  url?: string | null;
  badge?: string | null;
  description: string;
  integration?: SponsorIntegration | null;
  targetVersions: string;
  targetLanguages?: string[];
  createdAt: string;
  expiresAt?: string | null;
}

export interface SponsorIntegration {
  enabled: boolean;
  type?: 'sub2api' | 'new_api' | null;
  baseUrl: string;
  wireApi?: 'responses' | 'chat_completions' | null;
  quickConfigure?: boolean;
  dashboardCard?: boolean;
  models?: string[];
  supportsVision?: boolean;
  website?: string | null;
  apiKeyUrl?: string | null;
}

export interface SponsorModule {
  enabled: boolean;
  entryVisible: boolean;
  title: string;
  subtitle: string;
  targetVersions: string;
  targetLanguages?: string[];
  createdAt: string;
  expiresAt?: string | null;
  sponsors: Sponsor[];
}

export interface SponsorModuleState {
  sponsorModule: SponsorModule | null;
}
