import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
  isAccountsOverviewFilterPersistenceDisabledOnDisk,
  readAccountsOverviewFilterPersistenceEnabled,
  readRawAccountsOverviewFilterField,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence';
import {
  areDurableUiSettingsHydrated,
  OVERVIEW_FILTER_VALUES_CHANGED_EVENT,
} from '../utils/durableUiSettings';

function subscribe(listener: () => void): () => void {
  window.addEventListener(OVERVIEW_FILTER_VALUES_CHANGED_EVENT, listener);
  window.addEventListener(ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT, listener);
  return () => {
    window.removeEventListener(OVERVIEW_FILTER_VALUES_CHANGED_EVENT, listener);
    window.removeEventListener(ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT, listener);
  };
}

export function useAccountsOverviewFilterState<T>(
  scope: string,
  field: string | undefined,
  load: () => T,
  alwaysRemember = false,
) {
  const getSnapshot = () => field === undefined ? '[false,false,null,false]' : JSON.stringify([
    areDurableUiSettingsHydrated(),
    alwaysRemember || readAccountsOverviewFilterPersistenceEnabled(scope),
    readRawAccountsOverviewFilterField(scope, field),
    !alwaysRemember && isAccountsOverviewFilterPersistenceDisabledOnDisk(scope),
  ]);
  const source = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const [hydrated, enabled, saved, disabledOnDisk] = JSON.parse(source) as [boolean, boolean, string | null, boolean];
  const [value, setValue] = useState(load);
  const previous = useRef({ scope, field, source, hydrated, enabled, value, shouldClear: false });

  useEffect(() => {
    if (field === undefined) return;
    const last = previous.current;
    const changedScope = last.scope !== scope || last.field !== field;
    const edited = !Object.is(last.value, value);
    const resumed = hydrated && last.hydrated && enabled && !last.enabled && saved === null;
    const shouldClear = hydrated && !enabled && disabledOnDisk;
    previous.current = { scope, field, source, hydrated, enabled, value, shouldClear };

    if (changedScope || (!edited && !resumed && source !== last.source && (enabled || !last.hydrated))) {
      const restored = load();
      previous.current.value = restored;
      setValue(restored);
    } else if (enabled && (edited || resumed)) {
      writeAccountsOverviewFilterField(scope, field, value);
    }

    // A pending toggle can fail. Clear only after the native flag was saved.
    if (shouldClear && (!last.shouldClear || changedScope)) {
      removeAccountsOverviewFilterField(scope, field);
    }
  }, [scope, field, source, hydrated, enabled, saved, disabledOnDisk, value, load]);

  return [value, setValue] as const;
}
