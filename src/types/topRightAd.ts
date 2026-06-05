export interface TopRightAd {
  id: string;
  priority: number;
  text: string;
  badge?: string | null;
  ctaLabel?: string | null;
  ctaUrl?: string | null;
  targetVersions: string;
  targetLanguages?: string[];
  createdAt: string;
  expiresAt?: string | null;
}

export interface TopRightAdState {
  ad: TopRightAd | null;
  ads: TopRightAd[];
}
