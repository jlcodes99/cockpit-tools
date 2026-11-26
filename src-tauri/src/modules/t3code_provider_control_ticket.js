async expectedEnvironmentId => {
  const desktop = window.desktopBridge;
  const local = desktop?.getLocalEnvironmentBootstraps?.().find(entry => entry.id === 'primary');
  if (!local?.httpBaseUrl || !desktop.getLocalEnvironmentBearerToken) return null;
  const base = new URL(local.httpBaseUrl);
  if (base.protocol !== 'http:' || !['127.0.0.1', 'localhost', '[::1]'].includes(base.hostname)) return null;

  const signal = AbortSignal.timeout(5500);
  const descriptorResponse = await fetch(new URL('/.well-known/t3/environment', base), { signal });
  if (!descriptorResponse.ok) return null;
  const descriptor = await descriptorResponse.json();
  if (descriptor.environmentId !== expectedEnvironmentId || descriptor.orchestrationProtocolVersion !== 2) return null;

  const bearer = await desktop.getLocalEnvironmentBearerToken();
  const ticketResponse = await fetch(new URL('/api/auth/websocket-ticket/rpc', base), {
    method: 'POST',
    headers: { Authorization: `Bearer ${bearer}` },
    redirect: 'error',
    signal,
  });
  if (!ticketResponse.ok) return null;
  const { ticket } = await ticketResponse.json();
  if (typeof ticket !== 'string' || !ticket) return null;

  const socket = new URL('/ws', base);
  socket.protocol = 'ws:';
  socket.searchParams.set('orchestrationProtocol', '2');
  socket.searchParams.set('clientCapabilities', 'settingsSecretReferences');
  socket.searchParams.set('wsTicket', ticket);
  return socket.href;
}
