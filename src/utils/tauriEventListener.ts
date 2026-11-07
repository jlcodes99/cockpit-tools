import {
  listen,
  type EventCallback,
  type EventName,
  type Options,
  type UnlistenFn,
} from "@tauri-apps/api/event";

type MaybeAsyncUnlisten = () => void | Promise<void>;
type ListenerErrorHandler = (error: unknown, eventName: string) => void;

const STALE_LISTENER_MESSAGE = "listeners[eventId].handlerId";
let guardedEventInternals: Window["__TAURI_EVENT_PLUGIN_INTERNALS__"] | null = null;

export function isStaleTauriListenerError(error: unknown): boolean {
  return String(error).includes(STALE_LISTENER_MESSAGE);
}

/**
 * Tauri 2.11's injected unregister function checks the event bucket but not
 * the listener id before reading `handlerId`. A recreated webview can therefore
 * throw while React is tearing down listeners registered by the previous view.
 * Guard the injected function once, before any application listeners mount.
 */
export function installTauriEventCleanupGuard(): boolean {
  if (typeof window === "undefined") return false;
  const internals = window.__TAURI_EVENT_PLUGIN_INTERNALS__;
  if (!internals || guardedEventInternals === internals) return false;
  const unregisterListener = internals.unregisterListener;
  if (typeof unregisterListener !== "function") return false;

  internals.unregisterListener = (event, eventId) => {
    try {
      unregisterListener(event, eventId);
    } catch (error) {
      if (isStaleTauriListenerError(error)) return;
      throw error;
    }
  };
  guardedEventInternals = internals;
  return true;
}

function defaultListenerErrorHandler(error: unknown, eventName: string): void {
  if (isStaleTauriListenerError(error)) {
    console.debug(`[TauriEvent] ignored stale listener cleanup: ${eventName}`);
    return;
  }
  console.warn(`[TauriEvent] failed to clean up listener: ${eventName}`, error);
}

/**
 * Tauri 2.11 can reject an unlisten call after a webview has already cleared
 * its JS listener registry. Make cleanup single-shot and always consume the
 * returned promise so React effect teardown cannot create an unhandled
 * rejection during window recreation.
 */
export function createSafeTauriUnlisten(
  unlisten: MaybeAsyncUnlisten,
  eventName: string,
  onError: ListenerErrorHandler = defaultListenerErrorHandler,
): UnlistenFn {
  let called = false;
  return () => {
    if (called) return;
    called = true;
    try {
      const pending = unlisten();
      if (pending && typeof pending.then === "function") {
        void pending.catch((error) => onError(error, eventName));
      }
    } catch (error) {
      onError(error, eventName);
    }
  };
}

/**
 * React-friendly Tauri event subscription. It covers both teardown races:
 * cleanup before listen() resolves, and stale/double cleanup after a webview
 * registry reset.
 */
export function subscribeTauriEvent<T>(
  eventName: EventName,
  handler: EventCallback<T>,
  options?: Options,
): UnlistenFn {
  let disposed = false;
  let cleanup: UnlistenFn | null = null;

  void listen<T>(eventName, handler, options).then(
    (unlisten) => {
      const safeCleanup = createSafeTauriUnlisten(
        unlisten as unknown as MaybeAsyncUnlisten,
        eventName,
      );
      if (disposed) {
        safeCleanup();
        return;
      }
      cleanup = safeCleanup;
    },
    (error) => {
      if (!disposed) {
        console.warn(`[TauriEvent] failed to register listener: ${eventName}`, error);
      }
    },
  );

  return () => {
    disposed = true;
    cleanup?.();
    cleanup = null;
  };
}

/** Preserve the async listen contract while making teardown single-shot and rejection-safe. */
export async function listenSafely<T>(eventName: EventName, handler: EventCallback<T>, options?: Options): Promise<UnlistenFn> {
  return createSafeTauriUnlisten(await listen<T>(eventName, handler, options), String(eventName));
}
