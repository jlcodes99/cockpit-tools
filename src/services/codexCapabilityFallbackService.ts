import { listCodexAccounts } from './codexService';
import { getCodexLocalAccessState } from './codexLocalAccessService';

let reading: ReturnType<typeof read> | null = null;
async function read() {
  const [state, accounts] = await Promise.all([getCodexLocalAccessState(), listCodexAccounts()]);
  return { state, accounts };
}
/** Concurrent settings entrances share reads; no credentials or preferences are copied elsewhere. */
export function readCapabilityFallbackSnapshot() {
  if (!reading) {
    const request = read();
    reading = request;
    const clear = () => { if (reading === request) reading = null; };
    void request.then(clear, clear);
  }
  const request = reading;
  // A stalled local read must not leave the settings permanently loading.
  // Retain the underlying flight after timeout to avoid spawning duplicate work.
  let timer: ReturnType<typeof setTimeout> | undefined;
  return Promise.race([request, new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error('CAPABILITY_FALLBACK_TIMEOUT')), 15_000);
  })]).finally(() => clearTimeout(timer));
}
