const BOOT_SPLASH_ID = "app-boot-splash";
const BOOT_BAR_ID = "app-boot-bar";
const BOOT_SPLASH_HIDDEN_CLASS = "is-hidden";
const BOOT_SPLASH_REMOVE_DELAY_MS = 280;

export type BootSplashStage = "script_loaded" | "react_mounted" | "page_ready";

const STAGE_PROGRESS: Record<BootSplashStage, number> = {
  script_loaded: 80,
  react_mounted: 90,
  page_ready: 100,
};

let bootSplashDismissed = false;
let lastProgress = 0;

function bootSplashElement(): HTMLElement | null {
  if (typeof document === "undefined") {
    return null;
  }
  return document.getElementById(BOOT_SPLASH_ID);
}

function bootBarElement(): HTMLElement | null {
  if (typeof document === "undefined") {
    return null;
  }
  return document.getElementById(BOOT_BAR_ID);
}

export function setBootSplashStage(stage: BootSplashStage): void {
  if (bootSplashDismissed) {
    return;
  }
  const progress = STAGE_PROGRESS[stage];
  if (progress < lastProgress) {
    return;
  }
  lastProgress = progress;

  const bar = bootBarElement();
  const track = bar?.parentElement;
  if (track) {
    track.setAttribute("data-mode", "determinate");
  }
  if (bar) {
    bar.style.width = `${progress}%`;
  }

  if (stage === "page_ready") {
    dismissBootSplash();
  }
}

export function dismissBootSplash(): void {
  if (bootSplashDismissed || typeof document === "undefined") {
    return;
  }
  const splash = bootSplashElement();
  if (!splash) {
    bootSplashDismissed = true;
    return;
  }
  bootSplashDismissed = true;
  splash.classList.add(BOOT_SPLASH_HIDDEN_CLASS);
  splash.setAttribute("aria-hidden", "true");
  globalThis.setTimeout(() => {
    splash.remove();
  }, BOOT_SPLASH_REMOVE_DELAY_MS);
}

export function resetBootSplashStateForTests(): void {
  bootSplashDismissed = false;
  lastProgress = 0;
}
