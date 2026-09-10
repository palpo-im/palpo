import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { request } from 'node:http';
import { createApp } from '../server.mjs';
import { fixture, fleetInput, agentInput } from './fixture.mjs';

async function app(t) {
  const f = fixture();
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test' });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  t.after(async () => { await new Promise(resolve => server.close(resolve)); f.store.close(); });
  const url = `http://127.0.0.1:${server.address().port}`;
  const call = (path, { method = 'GET', body, cookie, csrf, origin = 'http://admin.example.test', host = 'admin.example.test', token } = {}) => new Promise((resolve, reject) => {
    const req = request(`${url}${path}`, {
      method, headers: { Host: host, Origin: origin, 'Content-Type': 'application/json', ...(cookie ? { Cookie: cookie } : {}), ...(csrf ? { 'X-CSRF-Token': csrf } : {}), ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    }, res => { const chunks = []; res.on('data', chunk => chunks.push(chunk)); res.on('end', () => resolve(new Response(Buffer.concat(chunks), { status: res.statusCode, headers: res.headers }))); });
    req.on('error', reject); req.end(body === undefined ? undefined : JSON.stringify(body));
  });
  const login = async () => {
    const response = await call('/api/login', { method: 'POST', body: { username: '@admin:example.test', password: 'correct-password' } });
    assert.equal(response.status, 200); const data = await response.json();
    return { cookie: response.headers.get('set-cookie').split(';')[0], csrf: data.csrf };
  };
  return { ...f, server, url, call, login };
}

test('administrator sign-in keeps tokens server-side and rejects unauthorized, cross-origin and non-CSRF writes', async t => {
  const f = await app(t);
  assert.equal((await f.call('/api/fleets')).status, 401);
  const ordinary = await f.call('/api/login', { method: 'POST', body: { username: '@owner:example.test', password: 'correct-password' } });
  assert.equal(ordinary.status, 200);
  const ordinaryCookie = ordinary.headers.get('set-cookie').split(';')[0];
  assert.equal((await ordinary.json()).isAdmin, false);
  assert.equal((await f.call('/api/fleets', { cookie: ordinaryCookie })).status, 403);
  const auth = await f.login();
  assert.ok(!JSON.stringify(auth).includes('admin-secret'));
  const session = await f.call('/api/session', auth);
  assert.ok(!(await session.text()).includes('admin-secret'));
  const noCsrf = await f.call('/api/fleets', { cookie: auth.cookie, method: 'POST', body: fleetInput });
  assert.equal(noCsrf.status, 403);
  const wrongOrigin = await f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput, origin: 'https://attacker.test' });
  assert.equal(wrongOrigin.status, 403);
  assert.equal((await f.call('/api/fleets', { ...auth, host: 'attacker.test' })).status, 403);
  assert.equal(f.registrations.size, 0);
});

test('HTTP fleet and agent workflow returns redacted state and scoped pairing is resumable', async t => {
  const f = await app(t), auth = await f.login();
  const created = await f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput });
  assert.equal(created.status, 201); const { fleet } = await created.json();
  assert.ok(!JSON.stringify(fleet).includes('as_token'));
  const list = await f.call('/api/fleets', auth);
  const detail = await f.call(`/api/fleets/${fleet.id}`, auth);
  const raw = `${await list.text()} ${await detail.text()}`;
  const registration = f.registrations.get(fleet.id);
  assert.ok(!raw.includes(registration.as_token)); assert.ok(!raw.includes(registration.hs_token));
  const denied = await f.call(`/api/pair/${fleet.id}`, { method: 'POST', token: 'other-secret' });
  assert.equal(denied.status, 404);
  const paired = await f.call(`/api/pair/${fleet.id}`, { method: 'POST', token: 'owner-secret' });
  assert.equal(paired.status, 200); assert.equal((await paired.json()).registration.id, fleet.id);
  const resumed = await f.call(`/api/pair/${fleet.id}`, { method: 'POST', token: 'owner-secret' });
  assert.equal(resumed.status, 200); assert.equal((await resumed.json()).registration.as_token, registration.as_token);
  const agentResponse = await f.call(`/api/fleets/${fleet.id}/agents`, { ...auth, method: 'POST', body: agentInput });
  assert.equal(agentResponse.status, 201); const { agent } = await agentResponse.json();
  const update = await f.call(`/api/fleets/${fleet.id}/agents/${agent.id}`, { ...auth, method: 'PATCH', body: { displayName: 'Updated coding agent' } });
  assert.equal(update.status, 200);
  const retired = await f.call(`/api/fleets/${fleet.id}/agents/${agent.id}/retire`, { ...auth, method: 'POST', body: {} });
  assert.equal(retired.status, 200); assert.equal((await retired.json()).agent.localTaskStop, 'unconfirmed');
  const audit = await f.call('/api/audit', auth);
  const auditText = await audit.text();
  assert.ok(auditText.includes('agent.retire')); assert.ok(!auditText.includes(registration.as_token));
});

test('revoking administrator privilege invalidates subsequent browser reads and writes', async t => {
  const f = await app(t), auth = await f.login();
  f.denyAdmin();
  assert.equal((await f.call('/api/fleets', auth)).status, 403);
  assert.equal((await f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput })).status, 403);
  assert.equal(f.registrations.size, 0);
});

test('catalog refresh observes published offers and preserves the last valid cache on callback failure', async t => {
  const f = await app(t), auth = await f.login();
  const { fleet } = await (await f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput })).json();
  f.publishedOffers.set(fleet.id, [{ role: 'coding', published: false }]);
  let view = (await (await f.call('/api/catalog', auth)).json()).fleets[0];
  assert.deepEqual(view.capabilities.offers, []); assert.equal(view.capabilityRead.state, 'current');
  f.publishedOffers.set(fleet.id, [{ role: 'coding', published: true }]);
  view = (await (await f.call('/api/catalog', auth)).json()).fleets[0];
  assert.equal(view.capabilities.offers[0].role, 'coding');
  const lastValid = structuredClone(view.capabilities);
  f.palpo.fetch = async (target, options) => new URL(target).pathname === '/api/fleet/v1/capabilities'
    ? new Response(JSON.stringify({ code: 'provider_unavailable', error: 'secret-upstream-detail' }), { status: 503 }) : f.fetch(target, options);
  view = (await (await f.call('/api/catalog', auth)).json()).fleets[0];
  assert.equal(view.capabilityRead.state, 'failed'); assert.equal(view.capabilityRead.code, 'provider_unavailable');
  assert.deepEqual(view.capabilities, lastValid);
  assert.ok(!JSON.stringify(view).includes('secret-upstream-detail'));
  assert.ok(!JSON.stringify(view).includes(f.registrations.get(fleet.id).hs_token));
  f.palpo.fetch = f.fetch; f.publishedOffers.set(fleet.id, []);
  view = (await (await f.call('/api/catalog', auth)).json()).fleets[0];
  assert.equal(view.capabilityRead.state, 'current'); assert.deepEqual(view.capabilities.offers, []);
});

test('resource catalog exposes stable public IDs and strips private Agent definitions', async t => {
  const f = await app(t), auth = await f.login();
  const { fleet } = await (await f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput })).json();
  f.publishedOffers.set(fleet.id, [{ role: 'coding', published: true, resources: [{
    id: `resource_${'a'.repeat(24)}`, name: 'Fast resource', framework: 'codex', model: 'gpt-5.6-sol', reasoning: 'medium',
    apiKey: 'secret-key', presetId: 'private-id', workspace: '/private/work',
    agents: ['one', 'two'].map(name => ({ name, role: 'coding', status: 'defined', ownerDmRoomId: '!private:test' })),
  }] }]);
  const view = (await (await f.call('/api/catalog', auth)).json()).fleets[0];
  assert.equal(view.capabilities.offers[0].resources[0].id, `resource_${'a'.repeat(24)}`);
  assert.equal(view.capabilities.offers[0].resources[0].agents, undefined);
  assert.equal(view.capabilities.offers[0].resources[0].reasoning, 'medium');
  assert.doesNotMatch(JSON.stringify(view), /secret-key|private-id|\/private\/work|ownerDmRoomId/);
});

test('concurrent retries share one durable installation and Matrix representative', async t => {
  const f = await app(t), auth = await f.login();
  const responses = await Promise.all(Array.from({ length: 4 }, () => f.call('/api/fleets', { ...auth, method: 'POST', body: fleetInput })));
  const ids = [];
  for (const response of responses) { assert.equal(response.status, 201); ids.push((await response.json()).fleet.id); }
  assert.equal(new Set(ids).size, 1); assert.equal(f.registrations.size, 1);
  assert.equal(f.calls.filter(c => c.method === 'POST' && c.path === '/_palpo/admin/v1/appservices').length, 1);
});

test('static app sets restrictive browser headers and never embeds credentials', async t => {
  const f = await app(t);
  const response = await f.call('/');
  assert.equal(response.status, 200);
  assert.match(response.headers.get('content-security-policy'), /frame-ancestors 'none'/);
  assert.equal(response.headers.get('cache-control'), 'no-store');
  assert.equal(response.headers.get('x-content-type-options'), 'nosniff');
  assert.match(await response.text(), /Hagency administration/);
});
