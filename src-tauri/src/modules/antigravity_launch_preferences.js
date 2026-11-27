(preferences) => {
  if (window.top !== window || location.protocol !== 'https:' || location.hostname !== '127.0.0.1') {
    return 'waiting';
  }
  if (window.__cockpitAntigravityLaunchPreferences) return window.__cockpitAntigravityLaunchPreferences;
  const composer = document.querySelector('[data-agent-composer-root].agy-composer-footer');
  const key = composer && Object.keys(composer).find(name => name.startsWith('__reactFiber$'));
  if (!key) return 'waiting';

  let settingsStore;
  let policy;
  const anchor = composer[key];
  let root = anchor;
  while (root.return) root = root.return;
  // React can leave the DOM reference on the previous side of the same tree.
  const committed = root.stateNode?.current === root.alternate ? anchor.alternate ?? anchor : anchor;
  for (let fiber = committed; fiber; fiber = fiber.return) {
    const value = fiber.memoizedProps?.value;
    if (typeof value?.getState === 'function') {
      const state = value.getState();
      if (typeof state?.getPrefValue === 'function' && typeof state?.setPrefValues === 'function' &&
          typeof state?.getConfiguredMagicWorkspaceVcs === 'function') settingsStore = value;
    }
    if (typeof value?.isSettingDisabled === 'function' && typeof value?.isSettingOptionAllowed === 'function' &&
        typeof value?.getEffectiveSettingValue === 'function') policy = value;
  }
  if (!settingsStore || policy?.fetchStatus !== 'loaded') return 'waiting';

  const settings = settingsStore.getState();
  const layouts = { software_engineering: 1, knowledge_work: 2 };
  // Mirrors the desktop client's Request Review, Default, Auto and Turbo presets.
  const modes = {
    request_review: [2, 1, false, 2],
    default: [1, 4, true, 2],
    auto: [6, 2, true, 2],
    turbo: [3, 3, false, 1],
  };
  const layout = layouts[preferences.layout];
  const mode = modes[preferences.permission];
  if (!Number.isInteger(layout) || !Array.isArray(mode)) return 'unsupported';
  const [permissionPreset, cascadeAutoExecutionPolicy, enableTerminalSandbox, filePolicy] = mode;
  const fileAccess = policy.outsideWorkspaceFileAccessPolicy;
  const nonWorkspaceFileAccessPolicy = fileAccess === 1 ? 3 : fileAccess === 2 && filePolicy === 1 ? 2 : filePolicy;
  const layoutAllowed = !policy.isSettingDisabled('project4kMode');
  const permissionAllowed = !policy.isSettingDisabled('permissionPreset') &&
    policy.isSettingOptionAllowed('permissionPreset', preferences.permission) &&
    (!enableTerminalSandbox || policy.sandboxSupported === true);
  const desired = {
    ...(layoutAllowed ? { project4kMode: layout } : {}),
    ...(permissionAllowed ? {
      enableTerminalSandbox, cascadeAutoExecutionPolicy, nonWorkspaceFileAccessPolicy, permissionPreset,
    } : {}),
  };

  window.__cockpitAntigravityLaunchPreferences = 'applying';
  void (async () => {
    try {
      const updates = Object.fromEntries(Object.entries(desired).filter(([name, value]) =>
        settings.getPrefValue(name) !== value));
      if (Object.keys(updates).length) await settings.setPrefValues(updates);
      const saved = settingsStore.getState();
      if (!Object.entries(desired).every(([name, value]) => saved.getPrefValue(name) === value)) {
        window.__cockpitAntigravityLaunchPreferences = 'failed';
        return;
      }
      window.__cockpitAntigravityLaunchPreferences =
        !layoutAllowed && !permissionAllowed ? 'restricted' :
          !layoutAllowed ? 'layout_restricted' :
            !permissionAllowed ? 'permission_restricted' : 'applied';
    } catch {
      window.__cockpitAntigravityLaunchPreferences = 'failed';
    }
  })();
  return window.__cockpitAntigravityLaunchPreferences;
}
