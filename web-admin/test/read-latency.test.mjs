import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request as httpRequest } from 'node:http';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { createApp } from '../server.mjs';
import { Palpo } from '../lib/service.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { fixture, fleetInput } from './fixture.mjs';

const owner = ['@owner:example.test', 'owner-secret'];
const deferred = () => { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; };
async function within(promise, ms = 1000) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('read remained blocked')), ms); })]); }
  finally { clearTimeout(timer); }
}
async function setup(t, { count = 1, ...options } = {}) {
  const f = fixture(), workflow = new Workflow(f.service);
  const fleet = await f.service.create(fleetInput, '@admin:example.test', 'admin-secret');
  await workflow.connect(fleet.id, ...owner);
  const project = await workflow.createProject({ fleetId: fleet.id, requestId: 'project-one', name: 'Project' }, ...owner);
  for (let i = 0; i < count; i++) await workflow.request({ projectId: project.id, requestId: `read-${i}`, role: 'coding', requestedTokens: 10, ratePerDay: 2 }, ...owner);
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', ...options });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  t.after(async () => { server.closeAllConnections(); await new Promise(r => server.close(r)); f.store.close(); });
  const call = (path, auth = {}, body) => new Promise((resolve, reject) => {
    const req = httpRequest(`http://127.0.0.1:${server.address().port}${path}`, {
      method: body === undefined ? 'GET' : 'POST', headers: { Host: 'admin.example.test', Origin: 'http://admin.example.test', 'Content-Type': 'application/json', ...auth },
    }, res => { const chunks = []; res.on('data', b => chunks.push(b)); res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, data: JSON.parse(Buffer.concat(chunks)) })); });
    req.on('error', reject); req.end(body === undefined ? undefined : JSON.stringify(body));
  });
  const login = await call('/api/login', {}, { username: owner[0], password: 'correct-password' });
  const auth = { Cookie: login.headers['set-cookie'][0].split(';')[0], 'X-CSRF-Token': login.data.csrf };
  return { ...f, workflow, fleet, project, call, auth };
}

test('status reads finish while a connection mutation is waiting and do not claim fresh usability', async t => {
  const f = await setup(t), gate = deferred();
  const before = structuredClone(Object.values(f.store.state.requests)[0]);
  const mutation = f.service.serial(() => gate.promise);
  t.after(() => gate.resolve());
  const response = await within(f.call('/api/requests', f.auth));
  assert.equal(response.status, 200);
  assert.equal(response.data.requests[0].statusVerified, false);
  assert.equal(response.data.requests[0].lastError.code, 'status_refresh_pending');
  assert.deepEqual(f.store.state.requests[response.data.requests[0].id], before);
  gate.resolve(); await mutation;
});

test('concurrent browser status reads share bounded callbacks while every caller revalidates identity', async t => {
  const f = await setup(t, { count: 4 }), gate = deferred(), entered = deferred();
  let active = 0, maxActive = 0, statusCalls = 0, identities = 0;
  f.palpo.fetch = async (target, options) => {
    const path = new URL(target).pathname;
    if (path.endsWith('/whoami')) identities++;
    if (path.startsWith('/api/fleet/v1/requests/')) {
      statusCalls++; active++; maxActive = Math.max(maxActive, active); if (active === 3) entered.resolve();
      await gate.promise; active--;
    }
    return f.fetch(target, options);
  };
  const reads = Array.from({ length: 4 }, () => f.call('/api/requests', f.auth));
  await within(entered.promise); await delay(20); gate.resolve();
  const responses = await within(Promise.all(reads));
  assert.equal(statusCalls, 4); assert.equal(maxActive, 3); assert.equal(identities, 4);
  for (const response of responses) { assert.equal(response.status, 200); assert.equal(response.data.requests.length, 4); }
});

test('a single deadline aborts slow status callbacks without multiplying by saved requests or blocking writes', async t => {
  const f = await setup(t, { count: 7, readTimeoutMs: 80 });
  let started = 0, cancelled = 0;
  f.palpo.fetch = (target, options) => {
    if (!new URL(target).pathname.startsWith('/api/fleet/v1/requests/')) return f.fetch(target, options);
    started++;
    return new Promise((_, reject) => options.signal.addEventListener('abort', () => { cancelled++; reject(options.signal.reason); }, { once: true }));
  };
  const startedAt = performance.now(), response = await within(f.call('/api/requests', f.auth));
  assert.equal(response.status, 504); assert.equal(response.data.code, 'read_timeout');
  assert.ok(performance.now() - startedAt < 500); await delay(10);
  assert.equal(started, 3); assert.equal(cancelled, 3);
  assert.equal(await within(f.service.serial(() => 'write proceeded')), 'write proceeded');
  assert.ok(Object.values(f.store.state.requests).every(row => row.state === 'pending' && row.usable !== true));
});

test('stale fulfilled poll cannot overwrite a concurrent retry or link its old identity', async t => {
  const f = await setup(t), gate = deferred(), entered = deferred();
  f.fulfill(f.fleet.id, 'read-0');
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname === '/api/fleet/v1/requests' && options.method === 'POST') {
      return new Response(JSON.stringify({ code: 'provider_retry_unavailable' }), { status: 503 });
    }
    const response = await f.fetch(target, options);
    if (new URL(target).pathname.startsWith('/api/fleet/v1/requests/')) { entered.resolve(); await gate.promise; }
    return response;
  };
  const read = f.call('/api/requests', f.auth); await within(entered.promise);
  const current = Object.values(f.store.state.requests)[0];
  const eventId = current.sourceEventId;
  const retried = await within(f.call('/api/requests', f.auth, { projectId: f.project.id, requestId: 'read-0', role: 'coding', requestedTokens: 10, ratePerDay: 2 }));
  assert.equal(retried.status, 503);
  gate.resolve(); const response = await within(read);
  assert.equal(response.data.requests[0].lastError.code, 'status_refresh_pending');
  assert.equal(response.data.requests[0].usable, false);
  assert.equal(current.state, 'submission_pending'); assert.equal(current.lastError.code, 'provider_retry_unavailable');
  assert.equal(current.sourceEventId, eventId); assert.equal(f.requests.size, 1);
  assert.equal(Object.keys(f.service.fleet(f.fleet.id).agents).length, 0);
});

test('catalog outage has one deadline, cancels callbacks and retains explicitly failed cached offers', async t => {
  const f = await setup(t, { readTimeoutMs: 60 });
  const saved = structuredClone(f.service.fleet(f.fleet.id).capabilities);
  let cancelled = 0;
  f.palpo.fetch = (target, options) => {
    if (!new URL(target).pathname.endsWith('/capabilities')) return f.fetch(target, options);
    return new Promise((_, reject) => options.signal.addEventListener('abort', () => { cancelled++; reject(options.signal.reason); }, { once: true }));
  };
  const response = await within(f.call('/api/catalog', f.auth));
  assert.equal(response.status, 504); await delay(10);
  const current = f.service.fleet(f.fleet.id);
  assert.equal(cancelled, 1); assert.deepEqual(current.capabilities, saved);
  assert.equal(current.capabilityRead.state, 'failed'); assert.equal(current.capabilityRead.code, 'read_timeout');
});

test('expired browser session cannot receive status after a delayed read', async t => {
  const f = await setup(t, { sessionTtl: 60 });
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname.startsWith('/api/fleet/v1/requests/')) await delay(80);
    return f.fetch(target, options);
  };
  const response = await within(f.call('/api/requests', f.auth));
  assert.equal(response.status, 401); assert.equal(response.data.code, 'sign_in_required');
});

test('Palpo deadline closes real stalled headers and response bodies', async t => {
  for (const headers of [false, true]) {
    const closed = deferred();
    const server = createServer((req, res) => {
      req.socket.once('close', closed.resolve);
      if (headers) { res.writeHead(200, { 'Content-Type': 'application/json' }); res.write('{"unfinished":'); }
    });
    server.listen(0, '127.0.0.1'); await once(server, 'listening');
    try {
      const palpo = new Palpo(`http://127.0.0.1:${server.address().port}`);
      await assert.rejects(palpo.call('/slow', null, { signal: AbortSignal.timeout(40) }), { name: 'TimeoutError' });
      await within(closed.promise);
    } finally { server.closeAllConnections(); await new Promise(r => server.close(r)); }
  }
});
