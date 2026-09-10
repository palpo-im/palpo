import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { request as httpRequest } from 'node:http';
import { randomUUID } from 'node:crypto';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { createApp } from '../server.mjs';
import { Service, publicFleet } from '../lib/service.mjs';
import { Store } from '../lib/store.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { fixture, fleetInput } from './fixture.mjs';

const transportOrigin = 'https://transport.example.test', relayOrigin = 'http://relay.internal:8090', browserOrigin = 'http://admin.example.test';
const owner = ['@owner:example.test', 'owner-secret'];
function capabilities(fleet) { return { v: 1, fleetId: fleet.id, serverName: 'example.test', representativeMxid: fleet.representativeMxid,
  approvalBotMxid: '@approvalbot:example.test', offers: [{ role: 'coding', resources: [{ id: `resource_${'a'.repeat(24)}`, name: 'Offline-capable resource', framework: 'codex', model: 'fixture-model' }] }] }; }
async function setup(t, options = {}) {
  const f = fixture({ transportOrigin, relayOrigin, ...options });
  const fleet = await f.service.create({ requestId: 'outbound-one', name: 'Outbound fleet', ownerMxid: owner[0] }, '@admin:example.test', 'admin-secret');
  const server = createApp({ service: f.service, publicOrigin: browserOrigin }).listen(0, '127.0.0.1'); await once(server, 'listening');
  t.after(async () => { server.closeAllConnections(); await new Promise(r => server.close(r)); if (f.store.db.isOpen) f.store.close(); });
  const call = (path, { method = 'GET', body, headers = {}, host = new URL(browserOrigin).host, origin = browserOrigin } = {}) => new Promise((resolve, reject) => {
    const req = httpRequest(`http://127.0.0.1:${server.address().port}${path}`, { method, headers: { Host: host, ...(origin ? { Origin: origin } : {}), 'Content-Type': 'application/json', ...headers } }, res => {
      const chunks = []; res.on('data', chunk => chunks.push(chunk)); res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, data: JSON.parse(Buffer.concat(chunks)) }));
    }); req.on('error', reject); req.end(body === undefined ? undefined : JSON.stringify(body));
  });
  const login = async mxid => {
    const r = await call('/api/login', { method: 'POST', body: { username: mxid, password: 'correct-password' } });
    assert.equal(r.status, 200); return { Cookie: r.headers['set-cookie'][0].split(';')[0], 'X-CSRF-Token': r.data.csrf };
  };
  const stored = () => f.service.fleet(fleet.id);
  const machine = (path, body, overrides = {}) => call(`/api/fleet/v2/${fleet.id}${path}`, { method: body === undefined ? 'GET' : 'POST', body,
    host: new URL(transportOrigin).host, origin: null, headers: { Authorization: `Bearer ${stored().transport.token}`, 'X-Hagency-Generation': String(stored().transport.generation) }, ...overrides });
  const relay = (transactionId, body) => call(`/api/relay/v2/${fleet.id}/_matrix/app/v1/transactions/${transactionId}`, { method: 'PUT', body, host: new URL(relayOrigin).host, origin: null, headers: { Authorization: `Bearer ${stored().registration.hs_token}` } });
  const update = (sequence, extra = {}) => machine('/updates', { v: 2, generation: stored().transport.generation, sequence, heartbeat: true, ...extra });
  const poll = lane => machine(`/poll?lane=${lane}&consumer=${randomUUID()}&wait=0`);
  const ack = delivery => machine('/ack', { id: delivery.id, lane: delivery.lane, token: delivery.token });
  return { ...f, fleet, stored, server, call, login, machine, relay, update, poll, ack };
}
async function prove(f) {
  assert.equal((await f.update(1, { capabilities: capabilities(f.fleet) })).status, 200);
  const auth = await f.login(owner[0]);
  const connected = await f.call(`/api/my/fleets/${f.fleet.id}/connect`, { method: 'POST', headers: auth, body: {} });
  assert.equal(connected.status, 202); assert.equal(connected.data.readiness.ready, false);
  const event = f.events.get(f.stored().probe.eventId);
  assert.equal((await f.relay('proof-transaction', { events: [event] })).status, 200);
  const matrix = (await f.poll('matrix')).data.delivery;
  const work = (await f.poll('work')).data.delivery;
  assert.equal(work.kind, 'probe'); assert.equal(matrix.kind, 'transaction');
  const receipt = { v: 1, received: true, ...work.payload, mode: 'edge' };
  assert.equal((await f.update(2, { probeReceipts: [receipt] })).data.code, 'matrix_receipt_pending');
  assert.equal((await f.ack(matrix)).status, 200); assert.equal((await f.ack(work)).status, 200);
  assert.equal((await f.update(2, { probeReceipts: [receipt] })).status, 200);
  assert.equal(publicFleet(f.stored()).readiness.ready, true);
  return auth;
}

test('outbound HTTP onboarding requires exact relay receipt and exposes secrets only in owner credentials', async t => {
  const f = await setup(t), auth = await prove(f);
  assert.equal(f.stored().registration.url, `${relayOrigin}/api/relay/v2/${f.fleet.id}`);
  assert.equal(f.stored().callbackUrl, null);
  const paired = await f.call(`/api/my/fleets/${f.fleet.id}/pair`, { method: 'POST', headers: auth, body: {} });
  assert.equal(paired.data.transport.mode, 'outbound'); assert.equal(paired.data.transport.token, f.stored().transport.token);
  assert.equal(paired.data.credentialVersion, 1); assert.equal(paired.data.v, undefined);
  assert.notEqual(paired.data.transport.token, paired.data.registration.hs_token);
  for (const path of ['/api/catalog', '/api/my/fleets']) {
    const result = await f.call(path, { headers: auth }); assert.equal(result.status, 200);
    const raw = JSON.stringify(result.data); for (const secret of [f.stored().transport.token, f.stored().registration.hs_token, f.stored().registration.as_token]) assert.ok(!raw.includes(secret));
  }
  const other = await f.login('@other:example.test');
  assert.equal((await f.call(`/api/my/fleets/${f.fleet.id}/pair`, { method: 'POST', headers: other, body: {} })).status, 404);
  assert.equal((await f.machine('/poll?lane=matrix&consumer=' + randomUUID() + '&wait=0', undefined, { host: 'attacker.test' })).status, 403);
  assert.equal((await f.machine('/poll?lane=matrix&consumer=' + randomUUID() + '&wait=0', undefined, { origin: browserOrigin })).status, 403);
});

test('repeated owner verification preserves the generation probe and an uncertain publication can recover', async t => {
  const f = await setup(t), auth = await prove(f), original = structuredClone(f.stored().probe);
  const receipt = { v: 1, received: true, fleetId: f.fleet.id, sourceRoomId: original.roomId, sourceEventId: original.eventId, challenge: original.challenge, mode: 'edge' };
  assert.equal((await f.call(`/api/my/fleets/${f.fleet.id}/connect`, { method: 'POST', headers: auth, body: {} })).status, 200);
  assert.deepEqual(f.stored().probe, original);
  assert.equal((await f.update(2, { probeReceipts: [receipt] })).status, 200);
  assert.equal((await f.update(3, { probeReceipts: [receipt] })).status, 200);
  assert.equal((await f.poll('work')).data.delivery, null);
  const fetch = f.palpo.fetch;
  f.palpo.fetch = async () => { throw new Error('fixture Matrix outage'); };
  await assert.rejects(new Workflow(f.service).connect(f.fleet.id, ...owner));
  assert.equal(f.stored().state, 'pending_connection');
  f.palpo.fetch = fetch;
  assert.equal((await f.call(`/api/my/fleets/${f.fleet.id}/connect`, { method: 'POST', headers: auth, body: {} })).data.readiness.ready, true);
  assert.deepEqual(f.stored().probe.challenge, original.challenge);
});

test('invalid machine payloads fail without committing and queue maintenance metrics require an administrator', async t => {
  const f = await setup(t);
  for (const input of [{ capabilities: null }, { statuses: null }, { probeReceipts: [null] }]) {
    assert.equal((await f.update(1, input)).status, 400); assert.equal(f.stored().transport.sequence, 0);
  }
  assert.equal((await f.relay('invalid-body', { events: [null] })).status, 400);
  const admin = await f.login('@admin:example.test'), auth = await f.login(owner[0]);
  const path = `/api/fleets/${f.fleet.id}/outbound`;
  assert.equal((await f.call(path, { headers: auth })).status, 403);
  await f.relay('count-one', { events: [] });
  const result = await f.call(path, { headers: admin });
  assert.equal(result.data.queue.records, 1); assert.equal(result.data.queue.pending, 1); assert.ok(result.data.queue.bytes > 0);
  assert.deepEqual(result.data.queue.limits, { records: 10000, pending: 1000, bytes: 16777216 });
  assert.ok(!JSON.stringify(result.data).includes(f.stored().transport.token));
});

test('a failed SQLite save cannot acknowledge an uncommitted update or lose its probe transaction', async t => {
  const f = await setup(t), save = f.store.save.bind(f.store);
  f.store.save = () => { throw new Error('fixture disk write failure'); };
  assert.equal((await f.update(1, { capabilities: capabilities(f.fleet) })).status, 500);
  assert.equal(f.stored().transport.sequence, 0); assert.equal(f.stored().capabilities, undefined);
  f.store.save = save;
  assert.equal((await f.update(1, { capabilities: capabilities(f.fleet) })).status, 200);
  const auth = await f.login(owner[0]);
  await f.call(`/api/my/fleets/${f.fleet.id}/connect`, { method: 'POST', headers: auth, body: {} });
  const probe = f.stored().probe, before = structuredClone(probe), event = f.events.get(probe.eventId);
  f.store.save = () => { throw new Error('fixture disk write failure'); };
  assert.equal((await f.relay('failed-save-proof', { events: [event] })).status, 500);
  assert.equal(f.stored().probe, probe); assert.deepEqual(probe, before);
  assert.equal((await f.poll('matrix')).data.delivery, null);
  f.store.save = save;
  assert.equal((await f.relay('failed-save-proof', { events: [event] })).status, 200);
  assert.equal((await f.poll('matrix')).data.delivery.id, 'failed-save-proof');
});

test('relay can arrive before Matrix send returns and duplicate transactions cannot replace its proof evidence', async t => {
  const f = await setup(t);
  f.palpo.fetch = async (url, options) => {
    const response = await f.fetch(url, options);
    if (new URL(url).pathname.includes('/send/com.hagency.connection.probe.v1/')) {
      const { event_id: id } = await response.clone().json();
      f.service.outbound.transaction(f.stored(), 'early-proof', { events: [f.events.get(id)] });
    }
    return response;
  };
  await prove(f);
  assert.equal(f.stored().probe.matrixTransactionId, 'early-proof');
  assert.equal(publicFleet(f.stored()).readiness.ready, true);
});

test('cross-fleet credentials and generic namespace claims cannot access another registration', async t => {
  const f = await setup(t);
  const other = await f.service.create({ requestId: 'other-fleet', name: 'Other', ownerMxid: '@other:example.test' }, '@admin:example.test', 'admin-secret');
  const headers = { Authorization: `Bearer ${f.stored().transport.token}`, 'X-Hagency-Generation': '1' };
  assert.equal((await f.call(`/api/fleet/v2/${other.id}/poll?lane=matrix&consumer=${randomUUID()}&wait=0`, { host: new URL(transportOrigin).host, origin: null, headers })).status, 401);
  const query = (kind, identity) => f.call(`/api/relay/v2/${f.fleet.id}/_matrix/app/v1/${kind}/${encodeURIComponent(identity)}`, { host: new URL(relayOrigin).host, origin: null, headers: { Authorization: `Bearer ${f.stored().registration.hs_token}` } });
  assert.equal((await query('users', f.fleet.representativeMxid)).status, 200);
  assert.equal((await query('users', other.representativeMxid)).status, 404);
  assert.equal((await query('users', `@${f.fleet.id}_unregistered:example.test`)).status, 404);
  assert.equal((await query('rooms', '#unknown:example.test')).status, 404);
});

test('heartbeat and a probe whose owner left cannot establish readiness', async t => {
  const f = await setup(t), auth = await f.login(owner[0]);
  await f.update(1, { capabilities: capabilities(f.fleet) }); assert.equal(publicFleet(f.stored()).readiness.ready, false);
  await f.call(`/api/my/fleets/${f.fleet.id}/connect`, { method: 'POST', headers: auth, body: {} });
  await f.relay('membership-proof', { events: [f.events.get(f.stored().probe.eventId)] });
  const matrix = (await f.poll('matrix')).data.delivery, work = (await f.poll('work')).data.delivery;
  await f.ack(matrix); await f.ack(work);
  f.putState(f.rooms.get(f.stored().probe.roomId), 'm.room.member', owner[0], { membership: 'leave' }, owner[0]);
  const result = await f.update(2, { probeReceipts: [{ received: true, ...work.payload, mode: 'edge' }] });
  assert.equal(result.data.code, 'reception_membership_pending'); assert.equal(f.stored().transport.sequence, 1);
  assert.equal(publicFleet(f.stored()).readiness.ready, false); assert.equal(f.stored().connection, undefined);
});

test('offline requests queue once and outbound catalog/status reads never call Hagency', async t => {
  const f = await setup(t), auth = await prove(f);
  f.palpo.fetch = (url, options) => { assert.ok(!new URL(url).pathname.startsWith('/api/fleet/v1')); return f.fetch(url, options); };
  const project = (await f.call('/api/projects', { method: 'POST', headers: auth, body: { fleetId: f.fleet.id, requestId: 'outbound-project', name: 'Project' } })).data.project;
  f.stored().transport.lastSeenAt = new Date(Date.now() - 100000).toISOString(); f.store.save();
  assert.equal(publicFleet(f.stored()).readiness.ready, false); assert.equal(publicFleet(f.stored()).readiness.canQueue, true);
  const input = { projectId: project.id, requestId: 'offline-request', agentName: 'offline-agent', resourceId: `resource_${'a'.repeat(24)}`, role: 'coding', requestedTokens: 10, ratePerDay: 2 };
  const first = await f.call('/api/requests', { method: 'POST', headers: auth, body: input });
  assert.equal(first.status, 201); assert.equal(first.data.request.state, 'queued');
  const retry = await f.call('/api/requests', { method: 'POST', headers: auth, body: input });
  assert.equal(retry.data.request.sourceEventId, first.data.request.sourceEventId);
  assert.equal((await f.call('/api/requests', { method: 'POST', headers: auth, body: { ...input, requestedTokens: 11 } })).status, 409);
  const delivered = (await f.poll('work')).data.delivery;
  assert.equal(delivered.kind, 'request'); assert.equal(delivered.payload.sourceEventId, first.data.request.sourceEventId);
  assert.equal(delivered.payload.ownerDmRoomId, project.ownerDmRoomId);
  assert.equal((await f.ack(delivered)).status, 200); assert.equal((await f.ack(delivered)).status, 200);
  assert.equal((await f.poll('work')).data.delivery, null);
  const status = { v: 1, ...delivered.payload, state: 'submission_pending', ready: false, fulfillment: { phase: 'admission', incomplete: true, error: 'owner_pending' } };
  assert.equal((await f.update(3, { statuses: [status] })).status, 200);
  const listed = await f.call('/api/requests', { headers: auth });
  assert.equal(listed.data.requests[0].state, 'submission_pending'); assert.equal(listed.data.requests[0].usable, false);
  assert.equal((await f.call('/api/catalog', { headers: auth })).status, 200);
  assert.equal(Object.keys(f.stored().agents).length, 0);
});

test('durable queue leases expire, redelivery changes receipt token and capacity never drops data', async t => {
  const f = await setup(t, { outboundOptions: { leaseMs: 30, maxPending: 2, maxRecords: 3 } });
  assert.equal((await f.relay('one', { events: [] })).status, 200);
  assert.equal((await f.relay('one', { events: [] })).status, 200);
  assert.equal((await f.relay('one', { events: [{ type: 'changed' }] })).status, 409);
  const first = (await f.poll('matrix')).data.delivery;
  assert.equal((await f.poll('matrix')).data.delivery, null);
  await delay(40); assert.equal((await f.ack(first)).data.code, 'stale_lease');
  const next = (await f.poll('matrix')).data.delivery; assert.equal(next.id, first.id); assert.notEqual(next.token, first.token);
  assert.equal((await f.ack(first)).status, 409); assert.equal((await f.ack(next)).status, 200);
  assert.equal((await f.relay('two', { events: [] })).status, 200); assert.equal((await f.relay('three', { events: [] })).status, 200);
  assert.equal((await f.relay('four', { events: [] })).data.code, 'queue_full');
  assert.equal(f.store.db.prepare('SELECT count(*) total FROM fleet_delivery').get().total, 3);
});

test('updates reject stale/conflicting sequences, foreign requests and changed bindings atomically', async t => {
  const f = await setup(t);
  const payload = { capabilities: capabilities(f.fleet) };
  assert.equal((await f.update(2, payload)).status, 200); assert.equal((await f.update(2, payload)).status, 200);
  assert.equal((await f.update(1, payload)).data.code, 'stale_sequence');
  assert.equal((await f.update(2, {})).data.code, 'sequence_conflict');
  assert.equal((await f.update(3, { statuses: [{ v: 1, requestId: 'unknown', fleetId: f.fleet.id }] })).data.code, 'unknown_request');
  assert.equal(f.stored().transport.sequence, 2);
  assert.equal((await f.machine('/updates', { v: 2, generation: 2, sequence: 3, heartbeat: true })).status, 400);
  const wrong = { Authorization: `Bearer ${f.stored().registration.as_token}`, 'X-Hagency-Generation': '1' };
  assert.equal((await f.machine('/updates', { v: 2, generation: 1, sequence: 3, heartbeat: true }, { headers: wrong })).status, 401);
});

test('published fulfillment needs the same bindings, current proof and actual joined membership', async t => {
  const f = await setup(t), auth = await prove(f);
  const project = (await f.call('/api/projects', { method: 'POST', headers: auth, body: { fleetId: f.fleet.id, requestId: 'fulfillment-project', name: 'Project' } })).data.project;
  const input = { projectId: project.id, requestId: 'fulfilled-request', role: 'coding', requestedTokens: 10, ratePerDay: 2, agentName: 'fulfilled-agent', resourceId: `resource_${'a'.repeat(24)}` };
  await f.call('/api/requests', { method: 'POST', headers: auth, body: input });
  const delivery = (await f.poll('work')).data.delivery, mxid = `@${f.fleet.id}_agent_serving:example.test`;
  const status = { ...delivery.payload, v: 1, observedAt: new Date().toISOString(), state: 'active', engagementId: 'engagement-one', agentMxid: mxid, ready: true, bound: true, serving: { framework: 'codex', model: 'fixture-model' } };
  const before = structuredClone(f.stored().capabilities);
  assert.equal((await f.update(3, { capabilities: { ...capabilities(f.fleet), offers: [] }, statuses: [{ ...status, sourceEventId: '$wrong' }] })).data.code, 'request_binding_conflict');
  assert.deepEqual(f.stored().capabilities, before); assert.equal(f.stored().transport.sequence, 2);
  assert.equal((await f.update(3, { statuses: [{ ...status, agentDefinition: { ...status.agentDefinition, name: 'changed' } }] })).data.code, 'request_binding_conflict');
  assert.equal((await f.update(3, { statuses: [{ ...status, agentMxid: '@foreign:example.test' }] })).data.code, 'agent_namespace_conflict');
  assert.equal((await f.update(3, { statuses: [status] })).status, 200);
  let view = (await f.call('/api/requests', { headers: auth })).data.requests[0];
  assert.equal(view.usable, false); assert.equal(view.state, 'admission_pending'); assert.equal(Object.keys(f.stored().agents).length, 0);
  f.putState(f.rooms.get(project.roomId), 'm.room.member', mxid, { membership: 'join' }, '@owner:example.test');
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0];
  assert.equal(view.usable, true); assert.equal(Object.keys(f.stored().agents).length, 1);
  const request = f.store.state.requests[`${f.fleet.id}:${input.requestId}`];
  request.outboundStatus.receivedAt = new Date(Date.now() - 100000).toISOString();
  await f.update(4);
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0]; assert.equal(view.usable, false);
  assert.equal((await f.update(5, { statuses: [status] })).status, 200);
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0]; assert.equal(view.usable, true);
  request.outboundStatus.generation = 0;
  await f.update(6);
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0]; assert.equal(view.usable, false);
  assert.equal((await f.update(7, { statuses: [{ ...status, fulfillment: { phase: 'verification', incomplete: true }, ready: false }] })).status, 200);
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0]; assert.equal(view.usable, false);
  f.stored().connection = null;
  view = (await f.call('/api/requests', { headers: auth })).data.requests[0]; assert.equal(view.usable, false);
});

test('late first publication and invalid observation clocks cannot refresh an old ready status', async t => {
  const f = await setup(t), auth = await prove(f);
  const project = (await f.call('/api/projects', { method: 'POST', headers: auth, body: { fleetId: f.fleet.id, requestId: 'clock-project', name: 'Project' } })).data.project;
  await f.call('/api/requests', { method: 'POST', headers: auth, body: { projectId: project.id, requestId: 'clock-request', role: 'coding', requestedTokens: 10, ratePerDay: 2 } });
  const delivery = (await f.poll('work')).data.delivery, mxid = `@${f.fleet.id}_clock_agent:example.test`;
  f.putState(f.rooms.get(project.roomId), 'm.room.member', mxid, { membership: 'join' }, owner[0]);
  const status = { ...delivery.payload, v: 1, state: 'active', ready: true, bound: true, agentMxid: mxid, serving: { framework: 'codex', model: 'fixture-model' } };
  let sequence = 3;
  for (const observedAt of [new Date(Date.now() - 100000).toISOString(), undefined, 'invalid timestamp', new Date(Date.now() + 60000).toISOString()]) {
    assert.equal((await f.update(sequence++, { statuses: [{ ...status, observedAt }] })).status, 200);
    const view = (await f.call('/api/requests', { headers: auth })).data.requests[0];
    assert.equal(view.usable, false); assert.equal(view.statusVerified, false); assert.equal(view.lastError.code, 'outbound_status_stale');
    assert.equal(Object.keys(f.stored().agents).length, 0);
  }
  assert.equal((await f.update(sequence++, { statuses: [{ ...status, observedAt: new Date().toISOString() }] })).status, 200);
  assert.equal((await f.call('/api/requests', { headers: auth })).data.requests[0].usable, true);
  assert.equal((await f.update(sequence, { statuses: [{ ...status, observedAt: new Date(Date.now() + 3000).toISOString() }] })).status, 200);
  assert.equal((await f.call('/api/requests', { headers: auth })).data.requests[0].usable, true);
});

test('SQLite restart retains leases, tombstones, update sequence and exact transaction contents', async t => {
  const directory = mkdtempSync(join(tmpdir(), 'palpo-outbound-')); t.after(() => rmSync(directory, { recursive: true, force: true }));
  const f = await setup(t, { path: join(directory, 'state.sqlite') });
  await f.update(5, { capabilities: capabilities(f.fleet) }); await f.relay('restart-one', { events: [], to_device: { events: [{ type: 'm.room.encrypted', content: { ciphertext: 'fixture' } }] } });
  const delivery = (await f.poll('matrix')).data.delivery;
  // Reopen the same database after closing the original store; no second writer.
  f.server.closeAllConnections(); await new Promise(r => f.server.close(r));
  f.store.close();
  const reopened = new Store(join(directory, 'state.sqlite'));
  const service = new Service({ store: reopened, palpo: f.palpo, serverName: 'example.test', transportOrigin, relayOrigin });
  const fleet = service.fleet(f.fleet.id), transport = service.outbound;
  const restarted = createApp({ service, publicOrigin: browserOrigin }).listen(0, '127.0.0.1'); await once(restarted, 'listening');
  t.after(async () => { restarted.closeAllConnections(); await new Promise(r => restarted.close(r)); if (reopened.db.isOpen) reopened.close(); });
  assert.equal(fleet.transport.sequence, 5);
  assert.equal(transport.claim(fleet, 'matrix', randomUUID()), null);
  const resumed = await new Promise((resolve, reject) => {
    const req = httpRequest(`http://127.0.0.1:${restarted.address().port}/api/fleet/v2/${fleet.id}/updates`, { method: 'POST', headers: { Host: new URL(transportOrigin).host, Authorization: `Bearer ${fleet.transport.token}`, 'X-Hagency-Generation': '1', 'Content-Type': 'application/json' } }, res => { res.resume(); res.on('end', () => resolve(res.statusCode)); });
    req.on('error', reject); req.end(JSON.stringify({ v: 2, generation: 1, sequence: 6, heartbeat: true }));
  });
  assert.equal(resumed, 200);
  assert.deepEqual(delivery.payload.body.to_device.events[0].content, { ciphertext: 'fixture' });
  assert.equal(transport.ack(fleet, delivery).ok, true); assert.equal(transport.enqueue(fleet, 'matrix', 'transaction', 'restart-one', delivery.payload), 'restart-one');
  assert.equal(transport.claim(fleet, 'matrix', randomUUID()), null);
  restarted.closeAllConnections(); await new Promise(r => restarted.close(r)); reopened.close();
});

test('migration recovers a completed URL CAS and rolls back a full replay queue without deleting data', async t => {
  const f = await setup(t), legacy = await f.service.create(fleetInput, '@admin:example.test', 'admin-secret');
  const workflow = new Workflow(f.service); await workflow.connect(legacy.id, ...owner);
  const project = await workflow.createProject({ fleetId: legacy.id, requestId: 'recovery-project', name: 'Recovery' }, ...owner);
  const existing = await workflow.request({ projectId: project.id, requestId: 'recovery-request', role: 'coding', requestedTokens: 10, ratePerDay: 2 }, ...owner);
  f.service.outbound.maxPending = 0;
  await assert.rejects(f.service.migrateOutbound(legacy.id, { requestId: 'recover-cas' }, '@admin:example.test', 'admin-secret'), { code: 'queue_full' });
  assert.equal(f.service.fleet(legacy.id).transport, undefined);
  assert.equal(f.registrations.get(legacy.id).url, `${relayOrigin}/api/relay/v2/${legacy.id}`);
  f.service.outbound.maxPending = 1000;
  await f.service.migrateOutbound(legacy.id, { requestId: 'recover-cas' }, '@admin:example.test', 'admin-secret');
  const fleet = f.service.fleet(legacy.id), delivery = f.service.outbound.claim(fleet, 'work', randomUUID());
  assert.equal(delivery.payload.sourceEventId, existing.sourceEventId);
  assert.equal(f.calls.filter(call => call.method === 'PUT' && call.path.endsWith('/url')).length, 1);
});

test('atomic URL migration and credential rotation preserve identities and replay existing requests', async t => {
  const f = await setup(t), legacy = await f.service.create(fleetInput, '@admin:example.test', 'admin-secret');
  const workflow = new Workflow(f.service); await workflow.connect(legacy.id, ...owner);
  const project = await workflow.createProject({ fleetId: legacy.id, requestId: 'legacy-project', name: 'Legacy project' }, ...owner);
  const request = await workflow.request({ projectId: project.id, requestId: 'legacy-request', role: 'coding', requestedTokens: 10, ratePerDay: 2 }, ...owner);
  const stored = f.service.fleet(legacy.id), original = structuredClone(stored.registration);
  await f.service.migrateOutbound(legacy.id, { requestId: 'migration-one' }, '@admin:example.test', 'admin-secret');
  assert.deepEqual({ ...stored.registration, url: original.url }, original);
  const oldToken = stored.transport.token;
  const delivery = f.service.outbound.claim(stored, 'work', randomUUID()); assert.equal(delivery.payload.sourceEventId, request.sourceEventId);
  f.service.outbound.transaction(stored, 'stable-as-transaction', { events: [] });
  f.service.outbound.ack(stored, f.service.outbound.claim(stored, 'matrix', randomUUID()));
  await f.service.migrateOutbound(legacy.id, { requestId: 'migration-one' }, '@admin:example.test', 'admin-secret');
  assert.equal(stored.transport.token, oldToken);
  await f.service.migrateOutbound(legacy.id, { requestId: 'rotate-one', rotate: true }, '@admin:example.test', 'admin-secret');
  assert.equal(stored.transport.generation, 2); assert.notEqual(stored.transport.token, oldToken);
  assert.throws(() => f.service.outbound.authenticate(legacy.id, oldToken, 1), { code: 'transport_unauthorized' });
  assert.throws(() => f.service.outbound.ack(stored, delivery), { code: 'stale_lease' });
  assert.equal(f.service.outbound.claim(stored, 'work', randomUUID()).payload.sourceEventId, request.sourceEventId);
  f.service.outbound.transaction(stored, 'stable-as-transaction', { events: [] });
  assert.equal(f.service.outbound.claim(stored, 'matrix', randomUUID()), null);
  assert.throws(() => f.service.outbound.transaction(stored, 'stable-as-transaction', { events: [{ type: 'changed' }] }), { code: 'delivery_conflict' });
  assert.equal(f.store.db.prepare("SELECT count(*) total FROM fleet_delivery WHERE fleet=? AND lane='matrix' AND id='stable-as-transaction'").get(legacy.id).total, 1);
  await f.service.setState(legacy.id, 'revoke', '@admin:example.test', 'admin-secret');
  assert.throws(() => f.service.outbound.authenticate(legacy.id, stored.transport.token, 2), { code: 'transport_unauthorized' });
  assert.ok(!f.calls.some(call => call.method === 'DELETE'));
});
