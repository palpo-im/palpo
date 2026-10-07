import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture, fleetInput } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';
import { ActionNotifications } from '../lib/action-notifications.mjs';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';
import { request as httpRequest } from 'node:http';

test('native HTTP boundary rejects browser context and oversized bodies; expiry requires a fresh host session', async t => {
  let now = 1000;
  const f = fixture();
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false,
    startActionWorker: false, miniappOptions: { now: () => now, ttlMs: 100 } });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  t.after(async () => { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); f.store.close(); });
  const base = `http://127.0.0.1:${server.address().port}/_palpo/miniapp/v1/`;
  const input = { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: ['palpo.session.open'] };
  const post = (path, body, headers = {}) => new Promise((resolve, reject) => {
    const req = httpRequest(base + path, { method: 'POST', headers: {
      Host: 'admin.example.test', 'Content-Type': 'application/json', Authorization: 'Bearer owner-secret', ...headers } }, res => {
      const chunks = []; res.on('data', chunk => chunks.push(chunk));
      res.on('end', () => resolve(new Response(Buffer.concat(chunks), { status: res.statusCode })));
    });
    req.on('error', reject); req.end(JSON.stringify(body));
  });
  assert.equal((await post('session', input, { Origin: 'https://evil.example' })).status, 403);
  assert.equal((await post('session', input, { Cookie: 'session=anything' })).status, 403);
  assert.equal((await post('session', input, { Host: 'evil.example' })).status, 403);
  assert.equal((await post('session', { ...input, padding: 'x'.repeat(17000) })).status, 413);
  const response = await post('session', input); assert.equal(response.status, 200);
  const session = await response.json(); assert.equal(session.userId, '@owner:example.test');
  assert.notEqual(session.sessionToken, 'owner-secret');
  const headers = { Authorization: `Bearer ${session.sessionToken}` };
  assert.equal((await post('call', { service: 'palpo.session.open', args: {} }, headers)).status, 200);
  now += 101;
  assert.equal((await post('call', { service: 'palpo.session.open', args: {} }, headers)).status, 401);
  assert.equal((await post('session', input)).status, 200);
  assert.equal(f.calls.filter(call => call.path.endsWith('/logout')).length, 0);
});

const resource = `resource_${'a'.repeat(24)}`;
function setup(t, options = {}) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test', ...options });
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false,
    startActionWorker: false, inboxOptions: { approvers: ['@admin:example.test'], requireProjectApproval: true, ...options.inboxOptions }, miniappOptions: options.miniappOptions });
  const app = server.miniapp;
  t.after(() => f.store.close());
  const login = async (token = 'owner-secret', services = Object.keys(SERVICES)) => (await app.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services })).sessionToken;
  const call = (session, service, args = {}) => app.call(`Bearer ${session}`, { service, args });
  return { ...f, app, inbox: server.inbox, login, call };
}
const contribution = { requestId: 'new-contribution', kind: 'contribution', name: 'Research pool', reason: 'Contribute an agent resource pool' };

test('admin authority is rechecked after waiting in the shared mutation queue', async t => {
  const f = setup(t), admin = await f.login('admin-secret');
  let unblock;
  const blocked = f.service.serial(() => new Promise(resolve => { unblock = resolve; }));
  await new Promise(resolve => setImmediate(resolve));
  const queued = f.call(admin, 'palpo.fleets.register', fleetInput);
  const rejected = assert.rejects(queued, e => e.status === 403);
  await new Promise(resolve => setImmediate(resolve));
  f.denyAdmin(); unblock();
  await blocked; await rejected;
  assert.equal(Object.keys(f.store.state.fleets).length, 0);
});

test('unsafe action-room powers stop delivery while the canonical action remains pending', async t => {
  let now = 1000;
  const f = setup(t, { inboxOptions: { now: () => now } });
  const owner = await f.login();
  const { action } = await f.call(owner, 'palpo.inbox.submit', contribution);
  const worker = new ActionNotifications(f.inbox, { homeserverOrigin: 'https://matrix.example.test',
    botMxid: '@other:example.test', botToken: 'other-secret', adminToken: 'admin-secret',
    approvers: ['@admin:example.test'] }, { now: () => now });
  await worker.tick();
  const room = f.rooms.get(f.inbox.state.rooms['@admin:example.test'].roomId);
  const powers = room.state.find(e => e.type === 'm.room.power_levels');
  powers.content.events = { 'm.room.power_levels': 0 };
  const before = f.events.size;
  now += 3600000; await worker.tick();
  assert.equal(f.events.size, before);
  assert.equal(f.inbox.get(action.id, '@admin:example.test', true).action.needsMyAction, true);
  assert.ok(Object.values(f.inbox.state.notices).some(n => n.lastError === 'action_room_not_private'));
});

test('shared login verifies current identity, exact grants and demotion; disconnect does not log out Matrix', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret');
  assert.equal((await f.call(owner, 'palpo.session.open')).userId, '@owner:example.test');
  assert.equal((await f.call(owner, 'palpo.session.open')).isAdmin, false);
  await assert.rejects(f.call(owner, 'palpo.fleets.register', fleetInput), e => e.status === 403);
  await assert.rejects(f.app.open('Bearer owner-secret', { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: ['palpo.http'] }), e => e.status === 403);
  const narrow = await f.login('owner-secret', ['palpo.catalog.list']);
  await assert.rejects(f.call(narrow, 'palpo.fleets.export', { fleetId: 'anything' }), e => e.code === 'service_not_granted');
  f.denyAdmin();
  assert.equal((await f.call(admin, 'palpo.session.open')).isAdmin, false);
  await assert.rejects(f.call(admin, 'palpo.activity.list'), e => e.status === 403);
  await f.app.disconnect(`Bearer ${owner}`);
  await assert.rejects(f.call(owner, 'palpo.session.open'), e => e.status === 401);
  assert.equal(f.calls.filter(call => call.path.endsWith('/logout')).length, 0);
  await f.login(); // The host's session remains valid.
});

test('contribution decision has one winner, survives read/dismiss, and replays the same installation without leaking secrets', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret'), stranger = await f.login('other-secret');
  const { action } = await f.call(owner, 'palpo.inbox.submit', contribution);
  assert.equal((await f.call(owner, 'palpo.inbox.list', { view: 'waiting' })).total, 1);
  assert.equal((await f.call(admin, 'palpo.inbox.list')).pendingCount, 1);
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  assert.equal((await f.call(admin, 'palpo.inbox.list')).pendingCount, 1);
  await assert.rejects(f.call(stranger, 'palpo.inbox.get', { id: action.id }), e => e.status === 404);
  const decision = { id: action.id, expectedRevision: action.revision, commandId: 'decision-1', decision: 'approve', reason: 'Approved contribution' };
  await assert.rejects(f.call(owner, 'palpo.inbox.decide', decision), e => e.status === 403);
  const { action: approved } = await f.call(admin, 'palpo.inbox.decide', decision);
  assert.equal(approved.state, 'approved'); assert.equal(approved.execution, 'done');
  assert.equal((await f.call(admin, 'palpo.inbox.decide', decision)).action.result.fleetId, approved.result.fleetId);
  await assert.rejects(f.call(admin, 'palpo.inbox.decide', { ...decision, commandId: 'another', decision: 'reject' }), e => e.code === 'decision_conflict');
  assert.equal(Object.keys(f.store.state.fleets).length, 1);
  const visible = JSON.stringify(await f.call(owner, 'palpo.inbox.get', { id: action.id }));
  assert.doesNotMatch(visible, /as_token|hs_token|sessionToken|commandId/);
  const config = await f.call(owner, 'palpo.fleets.export', { fleetId: approved.result.fleetId });
  assert.ok(config.registration.as_token);
  await assert.rejects(f.call(admin, 'palpo.fleets.export', { fleetId: approved.result.fleetId }), e => e.status === 404);
});

test('approved project activates as owner and enforces its resource grant on all request paths', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret');
  const fleet = await f.service.create({ ...fleetInput, transportMode: 'callback' }, '@admin:example.test', 'admin-secret');
  const row = await f.call(owner, 'palpo.inbox.submit', { requestId: 'project-approval', kind: 'project', name: 'Agent research', reason: 'Research agent', fleetId: fleet.id, resourceIds: [resource] });
  await assert.rejects(f.call(owner, 'palpo.projects.create', { requestId: 'bypass', fleetId: fleet.id, name: 'Bypass' }), e => e.code === 'project_approval_required');
  await f.call(admin, 'palpo.inbox.decide', { id: row.action.id, expectedRevision: 1, commandId: 'project-decision', decision: 'approve', reason: 'Project approved' });
  await assert.rejects(f.call(admin, 'palpo.inbox.activate', { id: row.action.id }), e => e.status === 404);
  const active = await f.call(owner, 'palpo.inbox.activate', { id: row.action.id });
  const project = f.store.state.projects[active.action.result.projectId];
  assert.equal(project.ownerMxid, '@owner:example.test');
  assert.deepEqual(project.resourceGrant.resourceIds, [resource]);
  assert.throws(() => f.inbox.resourceGrant(project, `resource_${'b'.repeat(24)}`), e => e.code === 'resource_not_granted');
  assert.throws(() => f.inbox.resourceGrant(project, undefined), e => e.code === 'resource_not_granted');
  assert.equal((await f.call(owner, 'palpo.inbox.activate', { id: row.action.id })).action.result.projectId, project.id);
  assert.equal(f.calls.filter(c => c.path === '/_matrix/client/v3/createRoom' && c.body.name === 'Agent research')[0].token, 'owner-secret');
});

test('restart keeps pending actions, notification reads and immutable request content', async t => {
  const dir = mkdtempSync(join(tmpdir(), 'palpo-miniapp-')); t.after(() => rmSync(dir, { recursive: true, force: true }));
  const path = join(dir, 'state.sqlite');
  const f = setup(t, { path }), owner = await f.login();
  const { action } = await f.call(owner, 'palpo.inbox.submit', contribution);
  const second = fixture({ path, transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  t.after(() => second.store.close());
  const app = createApp({ service: second.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false, startActionWorker: false });
  assert.equal(app.inbox.get(action.id, '@owner:example.test', false).action.state, 'requested');
  assert.equal(Object.keys(app.inbox.state.notices).length, 2);
  await assert.rejects(f.call(owner, 'palpo.inbox.submit', { ...contribution, name: 'Changed' }), e => e.code === 'idempotency_conflict');
});

test('notifications retry a lost receipt with the same event, remind after reading, and stop once resolved', async t => {
  let now = 1000;
  const f = setup(t, { inboxOptions: { now: () => now } });
  const owner = await f.login(), admin = await f.login('admin-secret');
  const { action } = await f.call(owner, 'palpo.inbox.submit', contribution);
  const worker = new ActionNotifications(f.inbox, { homeserverOrigin: 'https://matrix.example.test', botMxid: '@other:example.test', botToken: 'other-secret', adminToken: 'admin-secret', approvers: ['@admin:example.test'] }, { now: () => now, intervals: [100, 200] });
  const original = f.palpo.fetch; let failOnce = true;
  f.palpo.fetch = async (url, options) => { const result = await original(url, options); if (failOnce && new URL(url).pathname.includes('/send/m.room.message/')) { failOnce = false; throw new Error('lost receipt'); } return result; };
  await worker.tick();
  const count = f.events.size;
  now += 3000; await worker.tick();
  // Existing transaction was replayed, not duplicated; admin may also get a reminder.
  const ownerEvents = [...f.events.values()].filter(event => event.content['im.palpo.action.v1']?.ownerMxid === '@owner:example.test');
  assert.equal(ownerEvents.length, 1); assert.ok(f.events.size >= count);
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  const before = f.events.size; now += 300; await worker.tick(); assert.ok(f.events.size > before);
  await f.call(admin, 'palpo.inbox.decide', { id: action.id, expectedRevision: 1, commandId: 'reject', decision: 'reject', reason: 'Insufficient detail' });
  await worker.tick(); const resolvedCount = f.events.size; now += 10000; await worker.tick(); assert.equal(f.events.size, resolvedCount);
  for (const event of f.events.values()) assert.doesNotMatch(JSON.stringify(event.content), /Research pool|Contribute an agent|as_token|hs_token/);
});
