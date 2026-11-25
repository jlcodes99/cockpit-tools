(() => {
  function findClerk() {
    const isClerk = value => value && typeof value === 'object' &&
      ('loaded' in value || 'session' in value) && 'user' in value;
    if (isClerk(window.Clerk)) return window.Clerk;
    const app = document.getElementById('root');
    const key = app && Object.keys(app).find(key => key.startsWith('__reactContainer$'));
    const container = key && app[key];
    const queue = [container?.stateNode?.current || container?.current || container];
    for (let i = 0; i < queue.length && i < 512; i++) {
      const fiber = queue[i];
      if (!fiber) continue;
      const value = fiber.memoizedProps?.value;
      const candidates = Array.isArray(value)
        ? [value[0]?.clerk, value[0]?.value, value[0], value[1]?.clerk, value[1]]
        : [value?.clerk, value?.value, value];
      const clerk = candidates.find(isClerk);
      if (clerk) return clerk;
      if (fiber.child) queue.push(fiber.child);
      if (fiber.sibling) queue.push(fiber.sibling);
    }
    return null;
  }

  function relaySnapshot(environments, entitlement) {
    if (!Array.isArray(environments?.environments) || typeof entitlement?.available !== 'boolean') return null;
    const link = entitlement.link;
    const plan = entitlement.available && ['free', 'pro', 'premier'].includes(link?.plan) ? link.plan : null;
    const linked = entitlement.available ? (link === null ? false : plan ? true : null) : null;
    const caps = { free: 3, pro: 10, premier: 100 };
    const override = entitlement.maxTunnelsOverride;
    const limit = override !== undefined
      ? (Number.isSafeInteger(override) && override >= 0 ? override : null)
      : plan ? caps[plan] : linked === false || !entitlement.available ? caps.free : null;
    if (limit === null || environments.environments.some(row => typeof row?.endpoint?.providerKind !== 'string')) return null;
    return {
      planType: plan,
      t3chatLinked: linked,
      tunnelsUsed: environments.environments.filter(row => row.endpoint.providerKind === 'cloudflare_tunnel').length,
      tunnelsLimit: limit,
    };
  }

  function read(root, state) {
    const clerk = findClerk();
    const session = clerk?.session;
    const user = clerk?.user;
    if (!clerk || clerk.loaded === false) {
      root.clerkIdentity = null;
      return null;
    }
    if (!user?.id || !session?.id || (session.status && session.status !== 'active')) {
      root.clerkIdentity = null;
      return clerk.loaded ? { signedIn: false } : null;
    }
    const identity = user.id + ':' + session.id;
    if (root.clerkIdentity !== identity) {
      root.clerkIdentity = identity;
      root.clerkRelay = null;
      root.clerkRelayJob = null;
      root.clerkRelayAttempt = null;
    }
    const forceToken = state.clerkForceRefresh || null;
    const last = root.clerkRelayAttempt;
    if (!root.clerkRelayJob && typeof session.getToken === 'function' &&
        (!last || Date.now() - last.at >= (last.failed ? 15000 : 60000) ||
         (forceToken && forceToken !== last.token))) {
      const job = { token: forceToken };
      root.clerkRelayJob = job;
      (async () => {
        let snapshot = null;
        let tokenTimeout;
        try {
          const token = await Promise.race([
            session.getToken({ template: 't3-relay' }),
            new Promise(resolve => { tokenTimeout = setTimeout(() => resolve(null), 3500); }),
          ]);
          if (token && root.clerkIdentity === identity) {
            const signal = AbortSignal.timeout(5500);
            const get = async path => {
              const response = await fetch('https://relay.t3.codes' + path, {
                headers: { Authorization: 'Bearer ' + token }, signal, redirect: 'error',
              });
              return response.ok ? response.json() : null;
            };
            const [environments, entitlement] = await Promise.all([
              get('/v1/environments'), get('/v1/client/t3chat-link'),
            ]);
            snapshot = relaySnapshot(environments, entitlement);
          }
        } catch {
          // The last verified snapshot stays available for this session.
        } finally {
          clearTimeout(tokenTimeout);
          if (root.clerkRelayJob === job && root.clerkIdentity === identity) {
            const at = Date.now();
            if (snapshot) root.clerkRelay = { ...snapshot, relayVerifiedAt: Math.floor(at / 1000) };
            root.clerkRelayAttempt = { at, token: job.token, failed: !snapshot };
            root.clerkRelayJob = null;
          }
        }
      })();
    }
    const attempt = root.clerkRelayAttempt;
    const expires = session.expireAt ? new Date(session.expireAt).getTime() : NaN;
    return {
      signedIn: true,
      userId: user.id,
      sessionId: session.id,
      email: user.primaryEmailAddress?.emailAddress ?? null,
      displayName: user.fullName || user.username || user.firstName || null,
      avatarUrl: user.imageUrl || null,
      expiresAt: Number.isFinite(expires) ? Math.floor(expires / 1000) : null,
      ...root.clerkRelay,
      relayFailedAt: attempt?.failed ? Math.floor(attempt.at / 1000) : null,
      forceRefreshHandled: attempt?.token || null,
    };
  }

  return { read, relaySnapshot };
})()
