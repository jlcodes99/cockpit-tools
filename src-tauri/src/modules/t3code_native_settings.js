async ({ field, value }) => {
  const bridge = window.desktopBridge;
  if (!bridge?.getClientSettings || !bridge?.setClientSettings) return null;
  const settingsKey = 't3code:client-settings:v1';
  try {
    if (field === 'language' || field === 'notificationMode') {
      const settings = await bridge.getClientSettings();
      await bridge.setClientSettings({ ...(settings ?? {}), [field]: value });
      // T3 uses this event to refresh its client settings snapshot from persistence.
      window.dispatchEvent(new StorageEvent('storage', { key: settingsKey }));
    }
    const settings = await bridge.getClientSettings();
    return {
      language: settings?.language ?? 'en',
      notificationMode: settings?.notificationMode ?? 'off',
    };
  } catch {
    return null;
  }
}
