// Results execute only inside opaque-origin sandboxed iframes. Keep generated
// HTML out of the host DOM and block network APIs before its scripts execute.
const POLICY = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; webrtc 'block'";
const LOCKDOWN = `(() => {
  for (const key of ['RTCPeerConnection', 'webkitRTCPeerConnection', 'mozRTCPeerConnection', 'RTCDataChannel', 'WebTransport']) {
    try { Object.defineProperty(globalThis, key, {value: undefined, writable: false, configurable: false}); } catch (_) {}
  }
  for (const key of ['alert', 'confirm', 'prompt', 'print']) {
    try { Object.defineProperty(globalThis, key, {value: () => undefined, writable: false, configurable: false}); } catch (_) {}
  }
})()`;

export function pelicanPreviewDocument(html: string): string {
  return `<meta http-equiv="Content-Security-Policy" content="${POLICY}"><meta name="referrer" content="no-referrer"><script>${LOCKDOWN}</script>${html}`;
}
