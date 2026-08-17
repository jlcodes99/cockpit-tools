import assert from "node:assert/strict";
import test from "node:test";
import {
  dismissBootSplash,
  resetBootSplashStateForTests,
  setBootSplashStage,
} from "./bootSplash.ts";

function installSplashDom() {
  const splash = {
    className: "",
    classList: {
      add(name: string) {
        const owner = this.owner;
        if (!owner) return;
        owner.className = `${owner.className} ${name}`.trim();
      },
      owner: null as { className: string } | null,
    },
    attrs: {} as Record<string, string>,
    setAttribute(name: string, value: string) {
      this.attrs[name] = value;
    },
    removed: false,
    remove() {
      this.removed = true;
    },
  };
  splash.classList.owner = splash;

  const track = {
    attrs: {} as Record<string, string>,
    setAttribute(name: string, value: string) {
      this.attrs[name] = value;
    },
  };

  const bar = {
    style: { width: "" },
    parentElement: track,
  };

  const previousDocument = (globalThis as { document?: unknown }).document;
  const previousSetTimeout = globalThis.setTimeout;
  (globalThis as { document?: unknown }).document = {
    getElementById(id: string) {
      if (id === "app-boot-splash") return splash;
      if (id === "app-boot-bar") return bar;
      return null;
    },
  };
  const timeouts: Array<() => void> = [];
  globalThis.setTimeout = ((handler: () => void) => {
    timeouts.push(handler);
    return 1;
  }) as typeof setTimeout;

  return {
    splash,
    bar,
    track,
    timeouts,
    restore() {
      (globalThis as { document?: unknown }).document = previousDocument;
      globalThis.setTimeout = previousSetTimeout;
      resetBootSplashStateForTests();
    },
  };
}

test("dismissBootSplash hides and removes the static splash once", () => {
  resetBootSplashStateForTests();
  const dom = installSplashDom();
  try {
    dismissBootSplash();
    dismissBootSplash();
    assert.match(dom.splash.className, /is-hidden/);
    assert.equal(dom.splash.attrs["aria-hidden"], "true");
    assert.equal(dom.splash.removed, false);
    assert.equal(dom.timeouts.length, 1);
    dom.timeouts[0]();
    assert.equal(dom.splash.removed, true);
  } finally {
    dom.restore();
  }
});

test("boot splash stages only move forward and page ready dismisses", () => {
  resetBootSplashStateForTests();
  const dom = installSplashDom();
  try {
    setBootSplashStage("script_loaded");
    assert.equal(dom.track.attrs["data-mode"], "determinate");
    assert.equal(dom.bar.style.width, "80%");

    setBootSplashStage("react_mounted");
    assert.equal(dom.bar.style.width, "90%");

    setBootSplashStage("script_loaded");
    assert.equal(dom.bar.style.width, "90%");

    setBootSplashStage("page_ready");
    assert.equal(dom.bar.style.width, "100%");
    assert.match(dom.splash.className, /is-hidden/);
  } finally {
    dom.restore();
  }
});
