import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { request as httpRequest } from 'node:http';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { Workflow } from '../lib/workflow.mjs';

async function setup(t, options = {}) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.internal', ...options });
  const created = await f.service.create({ requestId: 'retire-test', name: 'Test fleet', ownerMxid: '@owner:example.test' }, '@admin:example.test', 'admin-secret');
  const fleet = f.service.fleet(created.id), mxid = `@${fleet.id}_agent_edison:example.test`;
  const request = { fleetId: fleet.id, requestId: 'edison-request', state: 'active', sourceEventId: '$source',
    payload: { role: 'coding', requestedTokens: 1000, targetProjectId: 'project', targetRoomId: '!project:example.test', sourceRoomId: '!reception:example.test' } };
  request.provider = { v: 1, ...request.payload, fleetId: fleet.id, requestId: request.requestId, sourceEventId: request.sourceEventId,
    state: 'active', agentMxid: mxid, allocatedTokens: 1000, ready: true, bound: true };
  f.store.state.requests = { [`${fleet.id}:${request.requestId}`]: request };
  f.users.set(mxid, { name: mxid, appservice_id: fleet.id, deactivated: false, displayname: 'Edison', rooms: ['!project:example.test', '!dm:example.test', '!group:example.test'] });
  f.store.save();
  const server = createApp({ service: f.service, publicOrigin: 'https://admin.example.test', retirementAdminToken: 'admin-secret' }).listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(async () => { server.closeAllConnections(); await new Promise(r => server.close(r)); f.store.close(); });
  const input = { requestId: request.requestId, agentMxid: mxid, endedAt: Date.now(), localStopped: true };
  const call = (body = input, headers = {}, route = `/api/fleet/v2/${fleet.id}/retire-agent`, method = 'POST') => new Promise((resolve, reject) => {
    const req = httpRequest(`http://127.0.0.1:${server.address().port}${route}`, {
      method, headers: { Host: 'transport.example.test', 'Content-Type': 'application/json',
        Authorization: `Bearer ${fleet.transport.token}`, 'X-HAFleet-Generation': String(fleet.transport.generation), ...headers },
    }, res => { const parts = []; res.on('data', p => parts.push(p)); res.on('end', () => resolve({ status: res.statusCode, body: JSON.parse(Buffer.concat(parts)) })); });
    req.on('error', reject); req.end(method === 'GET' ? undefined : JSON.stringify(body));
  });
  return { ...f, fleet, mxid, request, input, call };
}

test('outbound retirement deactivates only the confirmed Agent and preserves history and fleet', async t => {
  const f = await setup(t), history = { body: 'Earlier discussion', sender: f.mxid };
  f.events.set('$history', history);
  const result = await f.call();
  assert.equal(result.status, 200);
  assert.equal(result.body.agent.matrixIdentity, 'deactivated');
  assert.equal(result.body.agent.appserviceAccess, 'revoked');
  assert.equal(result.body.agent.localTaskStop, 'confirmed');
  assert.deepEqual(result.body.agent.joinedRooms, []);
  assert.equal(f.users.get(f.mxid).deactivated, true);
  assert.equal(f.users.get(f.fleet.ownerMxid).deactivated, false);
  assert.ok(f.registrations.has(f.fleet.id));
  assert.equal(f.events.get('$history'), history);
  assert.deepEqual(f.calls.filter(call => call.path.startsWith('/_palpo/admin/v1/deactivate/')).map(call => call.body), [{ erase: false }]);
  assert.equal(f.request.state, 'ended');
  const again = await f.call({ ...f.input, endedAt: f.input.endedAt + 100 });
  assert.equal(again.status, 200);
  assert.equal(f.request.retirement.endedAt, f.input.endedAt);
  new Workflow(f.service).applyStatus(f.request, { ...f.request.provider, state: 'active', ready: true, bound: true });
  assert.equal(f.request.state, 'ended'); assert.equal(f.request.provider.ready, false);
});

test('outbound retirement rejects human foreign unknown and still allocated identities', async t => {
  const f = await setup(t);
  for (const agentMxid of [f.fleet.ownerMxid, f.fleet.representativeMxid, '@other_agent:example.test']) {
    assert.equal((await f.call({ ...f.input, agentMxid })).status, 403);
  }
  assert.equal((await f.call({ ...f.input, requestId: 'unknown' })).status, 403);
  assert.equal((await f.call(f.input, { Authorization: 'Bearer invalid' })).status, 401);
  assert.equal((await f.call(f.input, { 'X-HAFleet-Generation': '999' })).status, 409);
  f.store.state.requests.other = { ...structuredClone(f.request), requestId: 'other' };
  assert.equal((await f.call()).body.code, 'agent_still_allocated');
  delete f.store.state.requests.other;
  f.users.get(f.mxid).appservice_id = 'another-fleet';
  assert.equal((await f.call()).body.code, 'identity_conflict');
  assert.equal(f.users.get(f.mxid).deactivated, false);
});

test('retirement remains incomplete if App Service authentication is still accepted', async t => {
  const f = await setup(t, { bypassRetirement: true });
  const result = await f.call();
  assert.equal(result.status, 502);
  assert.equal(result.body.code, 'retirement_unverified');
  assert.equal(f.request.retirement.state, 'failed');
});

test('retirement updates legacy management aliases and removes App Service user discovery', async t => {
  const f = await setup(t);
  f.fleet.agents.fulfilled_legacy = { id: 'fulfilled_legacy', mxid: f.mxid, state: 'registered' };
  f.fleet.agents.edison = { id: 'edison', mxid: f.mxid, state: 'registered' };
  const relay = () => f.call(undefined, { Host: 'relay.internal', Authorization: `Bearer ${f.fleet.registration.hs_token}` },
    `/api/relay/v2/${f.fleet.id}/users/${encodeURIComponent(f.mxid)}`, 'GET');
  assert.equal((await relay()).status, 200);
  assert.equal((await f.call()).status, 200);
  assert.equal((await relay()).status, 404);
  for (const row of Object.values(f.fleet.agents)) {
    assert.equal(row.state, 'retired'); assert.equal(row.localTaskStop, 'confirmed');
  }
});
