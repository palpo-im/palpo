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
import { contributedFleet, projectBudget, projectAdministrators, acceptProjectReservations } from './project-workflow-fixture.mjs';

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
  return { ...f, app, inbox: server.inbox, workflow: server.inbox.workflow, login, call };
}
async function projectRequest(f, requestId = 'new-project') {
  const fleet = await contributedFleet(f, f.workflow, f.inbox);
  return { requestId, kind: 'project', name: 'Research project', reason: 'Run a research agent', fleetId: fleet.id, resourceIds: [resource], allocations: projectBudget() };
}

test('admin authority is rechecked after waiting in the shared mutation queue', async t => {
  const f = setup(t), admin = await f.login('admin-secret');
  let unblock;
  const blocked = f.service.serial(() => new Promise(resolve => { unblock = resolve; }));
  await new Promise(resolve => setImmediate(resolve));
  const queued = f.call(admin, 'palpo.agents.register', {fleetId: 'missing', agentId: 'queued'});
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
  const input = await projectRequest(f);
  const { action } = await f.call(owner, 'palpo.inbox.submit', input);
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

test('project decision has one winner, survives read/dismiss, and replays without leaking secrets', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret'), stranger = await f.login('other-secret');
  const input = await projectRequest(f);
  const { action } = await f.call(owner, 'palpo.inbox.submit', input);
  assert.equal((await f.call(owner, 'palpo.inbox.list', { view: 'waiting' })).total, 1);
  assert.equal((await f.call(admin, 'palpo.inbox.list')).pendingCount, 1);
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  assert.equal((await f.call(admin, 'palpo.inbox.list')).pendingCount, 1);
  await assert.rejects(f.call(stranger, 'palpo.inbox.get', { id: action.id }), e => e.status === 404);
  const decision = { id: action.id, expectedRevision: action.revision, commandId: 'decision-1', decision: 'approve', reason: 'Approved project', ...projectAdministrators };
  await assert.rejects(f.call(owner, 'palpo.inbox.decide', decision), e => e.status === 403);
  const { action: approved } = await f.call(admin, 'palpo.inbox.decide', decision);
  assert.equal(approved.state, 'approved'); assert.equal(approved.execution, 'awaiting_reservation');
  assert.equal((await f.call(admin, 'palpo.inbox.decide', decision)).action.id, approved.id);
  await assert.rejects(f.call(admin, 'palpo.inbox.decide', { ...decision, commandId: 'another', decision: 'reject' }), e => e.code === 'decision_conflict');
  assert.equal(Object.keys(f.store.state.fleets).length, 1);
  const visible = JSON.stringify(await f.call(owner, 'palpo.inbox.get', { id: action.id }));
  assert.doesNotMatch(visible, /as_token|hs_token|sessionToken|commandId/);

});

test('the owner prepares a project, but only an applied reservation enables agent requests', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret');
  const input = await projectRequest(f, 'project-approval'); input.name = 'Agent research';
  const fleet = f.service.fleet(input.fleetId);
  const row = await f.call(owner, 'palpo.inbox.submit', input);
  await assert.rejects(f.call(owner, 'palpo.projects.create', { requestId: 'bypass', fleetId: fleet.id, name: 'Bypass' }), e => e.code === 'project_approval_required');
  const draft = f.store.state.projects[row.action.result.projectId];
  assert.equal(draft.state, 'awaiting_approval');
  assert.throws(() => f.inbox.resourceGrant(draft, resource), e => e.code === 'project_allocation_required');
  await f.call(admin, 'palpo.inbox.decide', { id: row.action.id, expectedRevision: row.action.revision, commandId: 'project-decision', decision: 'approve', reason: 'Project approved', ...projectAdministrators });
  assert.throws(() => f.inbox.resourceGrant(draft, resource), e => e.code === 'project_allocation_required');
  await acceptProjectReservations(f, f.workflow, f.inbox);
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
  const input = await projectRequest(f);
  const { action } = await f.call(owner, 'palpo.inbox.submit', input);
  const second = fixture({ path, transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  t.after(() => second.store.close());
  const app = createApp({ service: second.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false, startActionWorker: false });
  assert.equal(app.inbox.get(action.id, '@owner:example.test', false).action.state, 'requested');
  assert.equal(Object.keys(app.inbox.state.notices).length, 2);
  await assert.rejects(f.call(owner, 'palpo.inbox.submit', { ...input, name: 'Changed' }), e => e.code === 'idempotency_conflict');
});

test('notifications retry a lost receipt with the same event, remind after reading, and stop once resolved', async t => {
  let now = 1000;
  const f = setup(t, { inboxOptions: { now: () => now } });
  const owner = await f.login(), admin = await f.login('admin-secret');
  const input = await projectRequest(f);
  const { action } = await f.call(owner, 'palpo.inbox.submit', input);
  const worker = new ActionNotifications(f.inbox, { homeserverOrigin: 'https://matrix.example.test', botMxid: '@other:example.test', botToken: 'other-secret', adminToken: 'admin-secret', approvers: ['@admin:example.test'] }, { now: () => now, intervals: [100, 200] });
  const original = f.palpo.fetch; let failOnce = true;
  f.palpo.fetch = async (url, options) => { const result = await original(url, options); if (failOnce && new URL(url).pathname.includes('/send/m.room.message/')) { failOnce = false; throw new Error('lost receipt'); } return result; };
  await worker.tick();
  const count = f.events.size;
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  now += 3000; await worker.tick();
  // Existing transaction was replayed, not duplicated; admin may also get a reminder.
  const ownerEvents = [...f.events.values()].filter(event => event.content['im.palpo.action.v1']?.ownerMxid === '@owner:example.test');
  assert.equal(ownerEvents.length, 1); assert.ok(f.events.size >= count);
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  const before = f.events.size; now += 300; await worker.tick(); assert.equal(f.events.size, before, 'missed reminders are coalesced rather than delivered in a burst');
  await f.call(admin, 'palpo.inbox.decide', { id: action.id, expectedRevision: action.revision, commandId: 'reject', decision: 'reject', reason: 'Insufficient detail' });
  await worker.tick(); const resolvedCount = f.events.size; now += 10000; await worker.tick(); assert.equal(f.events.size, resolvedCount);
  for (const event of f.events.values()) assert.doesNotMatch(JSON.stringify(event.content), /Research project|Run a research agent|as_token|hs_token/);
});


test('only the designated administrator can see and decide other managers project requests', async t => {
  const f = setup(t);
  f.users.get('@other:example.test').admin = true;
  const original = f.palpo.fetch;
  // Authorize a second admin at Matrix's HTTP boundary, retaining its own whoami.
  f.palpo.fetch = (url, options) => original(url,
    new URL(url).pathname.startsWith('/_palpo/admin/') && options.headers.Authorization === 'Bearer other-secret'
      ? { ...options, headers: { ...options.headers, Authorization: 'Bearer admin-secret' } } : options);
  const owner = await f.login(), admin = await f.login('admin-secret'), other = await f.login('other-secret');
  assert.equal((await f.call(admin, 'palpo.session.open')).canApproveProjects, true);
  const secondIdentity = await f.call(other, 'palpo.session.open');
  assert.equal(secondIdentity.isAdmin, true);
  assert.equal(secondIdentity.canApproveProjects, false);
  const { action } = await f.call(owner, 'palpo.inbox.submit', await projectRequest(f));
  assert.equal((await f.call(other, 'palpo.inbox.list', { view: 'all' })).total, 0);
  await assert.rejects(f.call(other, 'palpo.inbox.get', { id: action.id }), e => e.status === 404);
  const decision = { id: action.id, expectedRevision: action.revision, commandId: 'scoped', decision: 'approve', reason: 'Review', ...projectAdministrators };
  await assert.rejects(f.call(other, 'palpo.inbox.decide', decision), e => e.code === 'project_approver_required');
  await assert.rejects(f.inbox.decide(decision, '@other:example.test', 'other-secret'), e => e.code === 'project_approver_required');
  assert.equal(f.inbox.state.records[action.id].state, 'requested');
  assert.throws(() => f.inbox.get(action.id, '@other:example.test', true), e => e.status === 404);
  assert.equal((await f.call(admin, 'palpo.inbox.decide', decision)).action.state, 'approved');
});

test('a project manager cannot approve own or another managers projects', async t => {
  const f = setup(t), owner = await f.login(), other = await f.login('other-secret');
  const { action } = await f.call(owner, 'palpo.inbox.submit', await projectRequest(f));
  assert.equal((await f.call(owner, 'palpo.inbox.get', { id: action.id })).action.canDecide, false);
  for (const session of [owner, other]) {
    await assert.rejects(f.call(session, 'palpo.inbox.decide', { id: action.id, expectedRevision: 1, commandId: 'forged', decision: 'approve', reason: 'Attempt' }), e => e.status === 403);
  }
  await assert.rejects(f.call(other, 'palpo.inbox.get', { id: action.id }), e => e.status === 404);
  assert.equal(f.inbox.state.records[action.id].state, 'requested');
});

test('demotion while a project decision is queued prevents the decision', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret');
  const { action } = await f.call(owner, 'palpo.inbox.submit', await projectRequest(f));
  let unblock;
  const blocked = f.service.serial(() => new Promise(resolve => { unblock = resolve; }));
  await new Promise(resolve => setImmediate(resolve));
  const queued = f.call(admin, 'palpo.inbox.decide', { id: action.id, expectedRevision: 1, commandId: 'demotion', decision: 'approve', reason: 'Review' });
  const rejected = assert.rejects(queued, e => e.status === 403);
  await new Promise(resolve => setImmediate(resolve));
  f.denyAdmin(); unblock(); await blocked; await rejected;
  assert.equal((await f.call(admin, 'palpo.session.open')).canApproveProjects, false);
  assert.equal(f.inbox.state.records[action.id].state, 'requested');
});

test('Rinx cannot originate contributions or registrations, even with an older full-capability bundle', async t => {
  const f = setup(t);
  for (const token of ['owner-secret', 'admin-secret']) {
    const session = await f.login(token);
    assert.equal((await f.call(session, 'palpo.session.open')).features.contributions, false);
    await assert.rejects(f.call(session, 'palpo.inbox.submit', { requestId: 'legacy', kind: 'contribution', name: 'Pool', reason: 'Old app' }), e => e.code === 'hagency_contribution_required');
    await assert.rejects(f.call(session, 'palpo.fleets.register', fleetInput), e => e.code === 'hagency_contribution_required');
  }
  assert.equal(Object.keys(f.inbox.state.records).length, 0);
  assert.equal(Object.keys(f.store.state.fleets).length, 0);
});

test('ambiguous administrator configuration fails closed and explicit identity overrides the notification list', async t => {
  const f = setup(t, { inboxOptions: { approvers: ['@admin:example.test', '@other:example.test'] } });
  const admin = await f.login('admin-secret'), owner = await f.login();
  assert.equal((await f.call(admin, 'palpo.session.open')).canApproveProjects, false);
  await assert.rejects(f.call(owner, 'palpo.inbox.submit', await projectRequest(f)), e => e.code === 'project_approver_unconfigured');
  const explicit = setup(t, { inboxOptions: { approvers: ['@admin:example.test', '@other:example.test'], projectApprover: '@admin:example.test' } });
  const session = await explicit.login();
  const { action } = await explicit.call(session, 'palpo.inbox.submit', await projectRequest(explicit));
  assert.deepEqual(Object.values(explicit.inbox.state.notices).filter(n => n.actionId === action.id).map(n => n.recipient).sort(), ['@admin:example.test', '@owner:example.test']);
});


test('legacy contribution records remain read-only history without changing connected resources', async t => {
  const f = setup(t), owner = await f.login(), admin = await f.login('admin-secret');
  const input = await projectRequest(f);
  const id = 'action_legacy_contribution';
  f.store.atomic(() => {
    f.inbox.state.records[id] = { id, kind: 'contribution', ownerMxid: '@owner:example.test', payload: {name: 'Earlier setup', reason: 'Legacy'}, state: 'approved', execution: 'done', revision: 3, result: {fleetId: input.fleetId}, updatedAt: 1 };
  });
  const previous = JSON.stringify(f.store.state.fleets);
  assert.equal((await f.call(owner, 'palpo.inbox.list')).pendingCount, 0);
  assert.equal((await f.call(owner, 'palpo.inbox.list', {view: 'history'})).total, 1);
  const view = (await f.call(owner, 'palpo.inbox.get', {id})).action;
  assert.equal(view.canContinue, false); assert.equal(view.canDecide, false); assert.equal(view.nextAction, null);
  await assert.rejects(f.call(admin, 'palpo.inbox.decide', {id, commandId:'legacy', expectedRevision:3, decision:'approve', reason:'Old app'}), e => e.code === 'hagency_contribution_required');
  await assert.rejects(f.call(owner, 'palpo.inbox.activate', {id}), e => e.code === 'hagency_contribution_required');
  assert.equal(JSON.stringify(f.store.state.fleets), previous);
});
