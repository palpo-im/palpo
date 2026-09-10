import { createServer } from 'node:http';
import { readFile, mkdir, open, unlink } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomBytes } from 'node:crypto';
import { Store } from './lib/store.mjs';
import { Service, Palpo, ApiError, publicFleet, fixedTransportOrigin } from './lib/service.mjs';
import { Workflow } from './lib/workflow.mjs';
import { isOutbound } from './lib/outbound.mjs';
import { Accounts } from './lib/accounts.mjs';

const base = dirname(fileURLToPath(import.meta.url));
const opaque = () => randomBytes(32).toString('base64url');
const assets = new Map([['/', ['index.html', 'text/html']], ['/app.js', ['app.js', 'text/javascript']], ['/accounts.js', ['accounts.js', 'text/javascript']], ['/style.css', ['style.css', 'text/css']]]);
const json = (res, status, body) => { res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' }); res.end(JSON.stringify(body)); };
const beforeDeadline = (promise, signal) => new Promise((resolve, reject) => {
  const abort = () => reject(new ApiError(504, 'read_timeout', 'Status verification timed out. Refresh to verify the current state.'));
  Promise.resolve(promise).then(resolve, reject).finally(() => signal.removeEventListener('abort', abort));
  if (signal.aborted) abort();
  else signal.addEventListener('abort', abort, { once: true });
});

async function body(req, limit = 16384) {
  if (req.headers['content-type']?.split(';')[0] !== 'application/json') throw new ApiError(415, 'json_required', 'Use application/json.');
  const chunks = []; let length = 0;
  for await (const chunk of req) {
    length += chunk.length;
    if (length > limit) throw new ApiError(413, 'body_too_large', 'Request body exceeds the endpoint limit.');
    chunks.push(chunk);
  }
  try {
    const parsed = JSON.parse(Buffer.concat(chunks).toString());
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error();
    return parsed;
  } catch { throw new ApiError(400, 'invalid_json', 'A JSON object is required.'); }
}

export function createApp({ service, publicOrigin, sessionTtl = 30 * 60 * 1000, readTimeoutMs = 8000, accountConfig, accountOptions, startAccountWorker = true, retirementAdminToken = accountConfig?.adminToken }) {
  const origin = new URL(publicOrigin);
  const workflow = new Workflow(service, { readTimeoutMs });
  const sessions = new Map(), attempts = new Map();
  const accounts = new Accounts(service, accountConfig, accountOptions), signupAttempts = new Map();
  const cookie = (value, clear = false) => `palpo_admin=${value}; Path=/; HttpOnly; SameSite=Strict${origin.protocol === 'https:' ? '; Secure' : ''}; Max-Age=${clear ? 0 : Math.floor(sessionTtl / 1000)}`;
  const error = (status, code, message) => { throw new ApiError(status, code, message); };
  const server = createServer(async (req, res) => {
    res.setHeader('Cache-Control', 'no-store');
    res.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'");
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.setHeader('Referrer-Policy', 'no-referrer');
    try {
      const url = new URL(req.url, origin), path = url.pathname;
      const machine = /^\/api\/fleet\/v2\/(hf_[a-f0-9]{32})\/(poll|ack|updates|retire-agent)$/.exec(path);
      const relay = /^\/api\/relay\/v2\/(hf_[a-f0-9]{32})\/(?:_matrix\/app\/v1\/)?(transactions|users|rooms)\/([^/]+)$/.exec(path);
      if (machine || relay) {
        const expectedOrigin = machine ? service.transportOrigin : service.relayOrigin;
        if (!expectedOrigin || req.headers.host !== new URL(expectedOrigin).host || req.headers.origin) error(403, 'host_forbidden', 'This machine endpoint requires its fixed server origin and no browser Origin.');
        const token = /^Bearer (\S+)$/.exec(req.headers.authorization ?? '')?.[1] ?? (relay ? url.searchParams.get('access_token') : null);
        const generation = req.headers['x-hafleet-generation'];
        const fleet = service.outbound.authenticate((machine ?? relay)[1], token, generation, !!relay);
        if (machine?.[2] === 'poll' && req.method === 'GET') {
          const controller = new AbortController(); res.once('close', () => controller.abort());
          const result = await service.outbound.poll(fleet.id, token, generation, { lane: url.searchParams.get('lane'), consumer: url.searchParams.get('consumer'), wait: url.searchParams.get('wait') ?? '25000' }, controller.signal);
          if (!res.destroyed) json(res, 200, result); return;
        }
        if (machine?.[2] === 'retire-agent' && req.method === 'POST') {
          const input = await body(req, 16384);
          const result = await service.serial(() => service.retireAllocatedAgent(
            service.outbound.authenticate(fleet.id, token, generation), input, retirementAdminToken));
          json(res, 200, result); return;
        }
        if (machine && req.method === 'POST' && ['ack', 'updates'].includes(machine[2])) {
          const input = await body(req, machine[2] === 'updates' ? 1024 * 1024 : 16384);
          // Authenticate again when the serialized update actually starts; a
          // rotation or revoke while the body/queue waited invalidates it.
          const result = machine[2] === 'ack' ? service.outbound.ack(service.outbound.authenticate(fleet.id, token, generation), input)
            : await service.serial(() => service.outbound.updates(service.outbound.authenticate(fleet.id, token, generation), input, workflow));
          json(res, 200, result); return;
        }
        if (relay?.[2] === 'transactions' && req.method === 'PUT') {
          const input = await body(req, 1024 * 1024);
          service.outbound.transaction(service.outbound.authenticate(fleet.id, token, null, true), decodeURIComponent(relay[3]), input);
          json(res, 200, {}); return;
        }
        if (relay && req.method === 'GET' && ['users', 'rooms'].includes(relay[2])) {
          const identity = decodeURIComponent(relay[3]);
          const known = relay[2] === 'users' && fleet.registration.namespaces.users.some(ns => new RegExp(ns.regex).test(identity))
            && (identity === fleet.representativeMxid || Object.values(fleet.agents).some(agent => agent.mxid === identity && agent.state === 'registered'));
          json(res, known ? 200 : 404, known ? {} : { errcode: 'M_NOT_FOUND', error: 'No registered identity exists in this namespace.' }); return;
        }
        error(405, 'method_not_allowed', 'Unsupported machine operation.');
      }
      if (req.headers.host !== origin.host) error(403, 'host_forbidden', 'Unexpected admin application host.');
      const readSignal = req.method === 'GET' && ['/api/requests', '/api/catalog', '/api/projects'].includes(path) ? AbortSignal.timeout(readTimeoutMs) : undefined;
      const mutation = !['GET', 'HEAD'].includes(req.method);
      const pairMatch = /^\/api\/pair\/(hf_[a-f0-9]{32})$/.exec(path);
      if (mutation && !pairMatch && req.headers.origin !== origin.origin) error(403, 'origin_forbidden', 'A same-origin request is required.');
      if (assets.has(path) && req.method === 'GET') {
        const [file, type] = assets.get(path);
        res.writeHead(200, { 'Content-Type': `${type}; charset=utf-8` }); res.end(await readFile(resolve(base, 'public', file))); return;
      }
      if (path === '/api/account-access' && req.method === 'GET') { json(res, 200, accounts.publicConfig()); return; }
      if (['/api/account-requests', '/api/account-requests/status'].includes(path) && req.method === 'POST') {
        const input = await body(req);
        const time = Date.now(), address = req.socket.remoteAddress, statusRead = path.endsWith('/status');
        for (const [key, value] of signupAttempts) if (value.until <= time) signupAttempts.delete(key);
        const key = `${address}:${statusRead ? 'read' : 'submit'}`;
        const rate = signupAttempts.get(key) ?? { count: 0, until: time + (statusRead ? 60000 : 3600000) };
        rate.count++; signupAttempts.set(key, rate);
        if (rate.count > (statusRead ? 180 : 30)) error(429, 'account_request_rate_limit', 'Too many account requests. Please try again later.');
        const request = statusRead ? accounts.status(input.id, input.receipt) : accounts.submit(input);
        json(res, statusRead ? 200 : 202, { request }); return;
      }
      if (path === '/api/login' && req.method === 'POST') {
        const address = req.socket.remoteAddress;
        const time = Date.now();
        for (const [key, value] of attempts) if (value.until <= time) attempts.delete(key);
        const rate = attempts.get(address) ?? { count: 0, until: time + 15 * 60 * 1000 };
        rate.count++; attempts.set(address, rate);
        if (rate.count > 10) error(429, 'login_rate_limit', 'Too many sign-in attempts. Try again later.');
        const input = await body(req);
        const username = service.owner(input.username);
        if (typeof input.password !== 'string' || !input.password || input.password.length > 4096) error(400, 'invalid_login', 'Password is required.');
        let login, isAdmin = false;
        try {
          login = await service.palpo.call('/_matrix/client/v3/login', null, { method: 'POST', body: {
            type: 'm.login.password', identifier: { type: 'm.id.user', user: username }, password: input.password,
            initial_device_display_name: 'Palpo web administration',
          } });
          if (!login.access_token || login.user_id !== username) error(502, 'invalid_login', 'Palpo did not verify the requested identity.');
          try { await service.palpo.requireAdmin(login.access_token); isAdmin = true; }
          catch (cause) { if (cause.status !== 403) throw cause; }
        } catch (cause) {
          if (login?.access_token) await service.palpo.call('/_matrix/client/v3/logout', login.access_token, { method: 'POST', body: {} }).catch(() => {});
          throw cause;
        }
        const id = opaque(), csrf = opaque();
        // Expired server sessions do not retain administrator tokens indefinitely.
        for (const [key, value] of sessions) if (value.expires <= time) sessions.delete(key);
        sessions.set(id, { token: login.access_token, userId: login.user_id, csrf, isAdmin, expires: time + sessionTtl });
        attempts.delete(address);
        res.setHeader('Set-Cookie', cookie(id));
        json(res, 200, { userId: login.user_id, csrf, isAdmin }); return;
      }
      if (pairMatch && req.method === 'POST') {
        if (req.headers.origin && req.headers.origin !== origin.origin) error(403, 'origin_forbidden', 'Cross-origin pairing is not allowed.');
        const token = /^Bearer (\S+)$/.exec(req.headers.authorization ?? '')?.[1];
        if (!token) error(401, 'owner_token_required', 'Pairing requires the fleet owner Matrix access token.');
        const identity = await service.palpo.call('/_matrix/client/v3/account/whoami', token);
        const result = await service.serial(() => service.credentials(pairMatch[1], identity.user_id));
        json(res, 200, result); return;
      }
      const sessionId = /(?:^|;\s*)palpo_admin=([a-zA-Z0-9_-]+)/.exec(req.headers.cookie ?? '')?.[1];
      const session = sessions.get(sessionId);
      if (!session || session.expires <= Date.now()) {
        if (sessionId) sessions.delete(sessionId);
        error(401, 'sign_in_required', 'Sign in with your local Matrix account.');
      }
      if (mutation && req.headers['x-csrf-token'] !== session.csrf) error(403, 'csrf_forbidden', 'Refresh the session before trying again.');
      if (path === '/api/logout' && req.method === 'POST') {
        await service.palpo.call('/_matrix/client/v3/logout', session.token, { method: 'POST', body: {} });
        sessions.delete(sessionId); res.setHeader('Set-Cookie', cookie('', true)); json(res, 200, {}); return;
      }
      // Recheck the actual Matrix identity for ordinary users as well as admins.
      let identity;
      try { identity = await service.palpo.call('/_matrix/client/v3/account/whoami', session.token, { signal: readSignal }); }
      catch (cause) {
        if (readSignal?.aborted) error(504, 'read_timeout', 'Status verification timed out. Refresh to verify the current state.');
        if (cause.status === 401) { sessions.delete(sessionId); error(401, 'sign_in_required', 'Your Matrix session has expired. Sign in again.'); }
        throw cause;
      }
      if (identity.user_id !== session.userId || identity.is_guest) error(403, 'identity_mismatch', 'The Matrix session no longer identifies the signed-in user.');
      if (readSignal) {
        // Share only this authenticated browser session's in-flight read. Keep
        // rechecking whoami for every caller; no cross-user authority cache.
        session.reads ??= new Map();
        if (!session.reads.has(path)) {
          const read = path === '/api/requests' ? workflow.requests(session.userId, session.token, readSignal)
            : path === '/api/catalog' ? workflow.catalog(session.userId, readSignal) : workflow.projects(session.userId, session.token, readSignal);
          const pending = read.finally(() => session.reads.delete(path));
          session.reads.set(path, pending);
        }
        const data = await beforeDeadline(session.reads.get(path), readSignal);
        if (sessions.get(sessionId) !== session || session.expires <= Date.now()) error(401, 'sign_in_required', 'Your session has expired. Sign in again.');
        json(res, 200, { [path === '/api/catalog' ? 'fleets' : path.slice('/api/'.length)]: data }); return;
      }
      if (path === '/api/session' && req.method === 'GET') {
        if (session.isAdmin) await service.palpo.requireAdmin(session.token);
        json(res, 200, { userId: session.userId, csrf: session.csrf, isAdmin: session.isAdmin, serverName: service.serverName, callbackOrigins: session.isAdmin ? [...service.callbackOrigins] : [], outboundAvailable: !!(service.transportOrigin && service.relayOrigin) }); return;
      }
      if (path === '/api/my/fleets' && req.method === 'GET') { json(res, 200, { fleets: Object.values(service.store.state.fleets).filter(fleet => fleet.ownerMxid === session.userId).map(publicFleet) }); return; }
      const ownerAction = /^\/api\/my\/fleets\/(hf_[a-f0-9]{32})\/(pair|connect)$/.exec(path);
      if (ownerAction && req.method === 'POST') {
        await body(req);
        const result = await service.serial(() => ownerAction[2] === 'pair' ? service.credentials(ownerAction[1], session.userId) : workflow.connect(ownerAction[1], session.userId, session.token));
        json(res, ownerAction[2] === 'connect' && isOutbound(service.fleet(ownerAction[1])) && !result.readiness.ready ? 202 : 200, result); return;
      }
      if (path === '/api/projects' && req.method === 'POST') {
        const input = await body(req);
        json(res, 201, { project: await service.serial(() => workflow.createProject(input, session.userId, session.token)) }); return;
      }
      if (path === '/api/requests' && req.method === 'POST') {
        const input = await body(req);
        json(res, 201, { request: await service.serial(() => workflow.request(input, session.userId, session.token)) }); return;
      }
      // All remaining management endpoints retain live Palpo admin enforcement.
      await service.palpo.requireAdmin(session.token);
      if (path === '/api/account-requests' && req.method === 'GET') { json(res, 200, accounts.adminView()); return; }
      const outboundMigration = /^\/api\/fleets\/(hf_[a-f0-9]{32})\/outbound$/.exec(path);
      if (outboundMigration && req.method === 'GET') {
        json(res, 200, { queue: service.outbound.usage(service.fleet(outboundMigration[1])) }); return;
      }
      if (outboundMigration && req.method === 'POST') {
        const input = await body(req);
        const fleet = await service.serial(() => service.migrateOutbound(outboundMigration[1], input, session.userId, session.token));
        json(res, 200, { fleet }); return;
      }
      if (path === '/api/fleets' && req.method === 'GET') { json(res, 200, { fleets: Object.values(service.store.state.fleets).map(publicFleet) }); return; }
      if (path === '/api/audit' && req.method === 'GET') { json(res, 200, { events: service.store.state.audit.slice(-200).reverse() }); return; }
      if (path === '/api/fleets' && req.method === 'POST') {
        const input = await body(req);
        const fleet = await service.serial(() => service.create(input, session.userId, session.token));
        json(res, 201, { fleet }); return;
      }
      const match = /^\/api\/fleets\/(hf_[a-f0-9]{32})(?:\/(install|pause|resume|revoke|agents)(?:\/([a-z0-9_]+)(?:\/(retire))?)?)?$/.exec(path);
      if (match) {
        const [, id, action, agentId, retire] = match;
        service.fleet(id);
        if (!action && req.method === 'GET') { json(res, 200, { fleet: publicFleet(service.fleet(id)) }); return; }
        if (['install', 'pause', 'resume', 'revoke'].includes(action) && req.method === 'POST') {
          await body(req);
          const fleet = await service.serial(() => action === 'install' ? service.install(id, session.userId, session.token) : service.setState(id, action, session.userId, session.token));
          json(res, 200, { fleet }); return;
        }
        if (action === 'agents') {
          if (!agentId && req.method === 'GET') { json(res, 200, { agents: await service.agents(id, session.token) }); return; }
          if (!agentId && req.method === 'POST') {
            const input = await body(req);
            const agent = await service.serial(() => service.createAgent(id, input, session.userId, session.token)); json(res, 201, { agent }); return;
          }
          if (agentId && !retire && req.method === 'PATCH') {
            const input = await body(req);
            const agent = await service.serial(() => service.updateAgent(id, agentId, input, session.userId, session.token)); json(res, 200, { agent }); return;
          }
          if (retire && req.method === 'POST') {
            await body(req);
            const agent = await service.serial(() => service.retireAgent(id, agentId, session.userId, session.token)); json(res, 200, { agent }); return;
          }
        }
      }
      error(404, 'not_found', 'Endpoint not found.');
    } catch (cause) {
      if (!res.headersSent) json(res, cause instanceof ApiError ? cause.status : 500, {
        code: cause instanceof ApiError ? cause.code : 'internal_error',
        error: cause instanceof ApiError ? cause.message : 'The admin operation failed. Check the operation record and retry.',
      });
      else res.end();
    }
  });
  server.accounts = accounts;
  if (startAccountWorker && accountConfig) server.once('listening', () => accounts.start());
  server.once('close', () => { void accounts.stop(); });
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const serverName = process.env.PALPO_SERVER_NAME;
  const palpoUrl = process.env.PALPO_URL;
  const port = Number(process.env.PORT ?? 8090);
  const publicOrigin = process.env.PUBLIC_ORIGIN ?? `http://127.0.0.1:${port}`;
  const callbackOrigins = (process.env.PALPO_CALLBACK_ORIGINS ?? '').split(',').filter(Boolean).map(value => new URL(value).origin);
  if (!serverName || !palpoUrl) throw new Error('PALPO_SERVER_NAME and PALPO_URL are required.');
  const upstream = new URL(palpoUrl), origin = new URL(publicOrigin);
  if (!['http:', 'https:'].includes(upstream.protocol) || upstream.username || upstream.password || upstream.pathname !== '/' || upstream.search || upstream.hash) throw new Error('PALPO_URL must be a fixed HTTP(S) origin without credentials.');
  if (!['http:', 'https:'].includes(origin.protocol) || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) throw new Error('PUBLIC_ORIGIN must be an HTTP(S) origin without credentials.');
  if (origin.protocol !== 'https:' && !['127.0.0.1', 'localhost', '[::1]'].includes(origin.hostname)) throw new Error('A public admin application requires HTTPS.');
  const transportOrigin = fixedTransportOrigin(process.env.PALPO_TRANSPORT_ORIGIN, true), relayOrigin = fixedTransportOrigin(process.env.PALPO_RELAY_ORIGIN);
  const queueLimit = (name, fallback) => {
    if (process.env[name] === undefined) return fallback;
    const value = Number(process.env[name]);
    if (!Number.isSafeInteger(value) || value < 1) throw new Error(`${name} must be a positive safe integer`);
    return value;
  };
  const outboundOptions = { maxPending: queueLimit('PALPO_FLEET_QUEUE_MAX_PENDING', 1000), maxRecords: queueLimit('PALPO_FLEET_QUEUE_MAX_RECORDS', 10000), maxBytes: queueLimit('PALPO_FLEET_QUEUE_MAX_BYTES', 16 * 1024 * 1024) };
  const databasePath = resolve(process.env.PALPO_ADMIN_DATABASE ?? resolve(base, 'data', 'admin.sqlite'));
  await mkdir(dirname(databasePath), { recursive: true, mode: 0o700 });
  // An explicit process lock prevents two service instances racing Matrix effects.
  const lockPath = `${databasePath}.lock`;
  const lock = await open(lockPath, 'wx', 0o600);
  await lock.writeFile(`${process.pid}\n`);
  const store = new Store(databasePath);
  const service = new Service({ store, palpo: new Palpo(palpoUrl), serverName, callbackOrigins, transportOrigin, relayOrigin, outboundOptions });
  const accountConfig = process.env.PALPO_ACCOUNT_CONFIG ? JSON.parse(await readFile(process.env.PALPO_ACCOUNT_CONFIG, 'utf8')) : undefined;
  const retirementAdminToken = process.env.PALPO_AGENT_ADMIN_TOKEN_FILE
    ? (await readFile(process.env.PALPO_AGENT_ADMIN_TOKEN_FILE, 'utf8')).trim() : accountConfig?.adminToken;
  const server = createApp({ service, publicOrigin, accountConfig, retirementAdminToken });
  let shuttingDown = false;
  const shutdown = async () => {
    if (shuttingDown) return; shuttingDown = true;
    const closed = new Promise(resolve => server.close(resolve)); server.closeIdleConnections();
    const deadline = setTimeout(() => server.closeAllConnections(), 8000); deadline.unref();
    await Promise.all([server.accounts.stop(), closed]); clearTimeout(deadline);
    store.close(); await lock.close(); await unlink(lockPath); process.exit(0);
  };
  process.on('SIGINT', shutdown); process.on('SIGTERM', shutdown);
  server.listen(port, process.env.LISTEN_HOST ?? '127.0.0.1', () => console.log(`Palpo web administration: ${publicOrigin}`));
}
