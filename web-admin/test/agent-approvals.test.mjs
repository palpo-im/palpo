import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { ActionNotifications } from '../lib/action-notifications.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';
import { contributedFleet, projectBudget, projectAdministrators, acceptProjectReservations, projectResource } from './project-workflow-fixture.mjs';

async function setup(t, policy = projectAdministrators) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false,
    startActionWorker: false, inboxOptions: { projectApprover: '@admin:example.test', requireProjectApproval: true } });
  t.after(() => f.store.close());
  const inbox = server.inbox, workflow = inbox.workflow, commands = server.projectCommands;
  commands.adminToken = 'admin-secret';
  const fleet = await contributedFleet(f, workflow, inbox);
  const login = async token => (await server.miniapp.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: Object.keys(SERVICES) })).sessionToken;
  const owner = await login('owner-secret'), admin = await login('admin-secret'), assigned = await login('other-secret');
  const call = (session, service, args) => server.miniapp.call(`Bearer ${session}`, { service, args });
  const projectRequest = (await call(owner, 'palpo.inbox.submit', { requestId: 'project_for_agent', kind: 'project', name: 'Agent project', reason: 'Research', fleetId: fleet.id,
    resourceIds: [projectResource], allocations: projectBudget() })).action;
  await call(admin, 'palpo.inbox.decide', { id: projectRequest.id, expectedRevision: projectRequest.revision, commandId: 'project_decision', decision: 'approve', reason: 'Budget approved', ...policy });
  await acceptProjectReservations(f, workflow, inbox);
  const project = f.store.state.projects[projectRequest.result.projectId];
  const input = { requestId: 'agent_one', projectId: project.id, role: 'coding', requestedTokens: 100000, ratePerDay: 10000,
    agentDefinition: { name: 'ResearchBot', resourceId: projectResource } };
  const submit = (body = input) => call(owner, 'palpo.requests.create', body);
  const decideInput = row => ({ id: row.id, expectedRevision: row.revision, commandId: 'agent_decision', decision: 'approve', reason: 'Reviewed', allocatedTokens: 80000 });
  const decide = (row, extra = {}, session = assigned) => call(session, 'palpo.inbox.decide', { ...decideInput(row), ...extra });
  const entry = () => Object.values(commands.state.commands).find(e => e.command.operation.kind === 'approve_agent' || e.command.operation.kind === 'reject_agent');
  const receipt = e => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest,
    completedAtMs: inbox.now(), outcome: { status: 'applied', result: { kind: 'agent',
      engagementId: `en_${createHash('sha256').update(JSON.stringify([fleet.id, e.command.operation.request.requestId])).digest('hex').slice(0, 32)}`,
      state: e.command.operation.kind === 'approve_agent' ? 'reserved' : 'rejected', allocatedTokens: e.command.operation.allocatedTokens ?? input.requestedTokens, cleanup: 'not_required' } } });
  const accept = async e => f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1,
    heartbeat: true, commandReceipts: [receipt(e)] }, workflow);
  return { ...f, inbox, workflow, commands, fleet, project, input, owner, admin, assigned, call, submit, decide, decideInput, entry, receipt, accept };
}

test('budgeted agents go only to assigned project administrators, never the legacy work queue', async t => {
  const f = await setup(t), { request } = await f.submit();
  assert.equal(request.state, 'pending_project_admin');
  const rows = await f.call(f.assigned, 'palpo.inbox.list', { view: 'needs_action' });
  assert.equal(rows.actions.length, 1); const action = rows.actions[0];
  assert.equal(action.kind, 'agent'); assert.equal(action.canDecide, true);
  assert.equal(action.payload.name, 'ResearchBot');
  assert.ok(!JSON.stringify(action).includes(f.project.ownerDmRoomId));
  assert.equal((await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action.canDecide, false);
  assert.equal((await f.call(f.admin, 'palpo.inbox.list', { view: 'all' })).actions.some(r => r.id === action.id), false);
  await assert.rejects(f.call(f.admin, 'palpo.inbox.get', { id: action.id }), e => e.status === 404);
  await assert.rejects(f.decide(action, {}, f.owner), e => e.code === 'project_administrator_required');
  assert.equal(f.store.db.prepare("SELECT count(*) n FROM fleet_delivery WHERE kind='request'").get().n, 0);
  assert.deepEqual(new Set(Object.values(f.inbox.state.notices).filter(n => n.actionId === action.id).map(n => n.recipient)), new Set(['@owner:example.test', '@other:example.test']));
  const second = await f.submit(); assert.equal(second.request.actionId, action.id);
  assert.equal(Object.values(f.inbox.state.records).filter(r => r.kind === 'agent').length, 1);
});

test('agent decisions are atomic, source-bound and replay safely; a receipt is not a ready agent', async t => {
  const f = await setup(t), { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  const before = structuredClone(f.store.state);
  f.service.outbound.maxPending = f.service.outbound.usage(f.fleet).pending;
  await assert.rejects(f.decide(row), e => e.code === 'queue_full'); assert.deepEqual(f.store.state, before);
  f.service.outbound.maxPending = 100;
  const command = f.decideInput(row);
  const decided = await f.call(f.assigned, 'palpo.inbox.decide', command), e = f.entry();
  assert.equal(decided.action.execution, 'awaiting_hagency');
  assert.equal(e.command.actorMxid, '@other:example.test');
  assert.equal(e.command.operation.request.sourceEventId, request.sourceEventId);
  assert.equal((await f.commands.authorize(f.service.fleet(f.fleet.id), { commandId: e.command.commandId, commandDigest: e.digest })).allowed, true);
  assert.equal((await f.call(f.assigned, 'palpo.inbox.decide', command)).action.revision, decided.action.revision);
  await assert.rejects(f.call(f.assigned, 'palpo.inbox.decide', { ...command, allocatedTokens: 90000 }), er => er.code === 'decision_conflict');
  const currentFleet = f.service.fleet(f.fleet.id);
  const records = f.commands.validateReceipts(currentFleet, [f.receipt(e)]);
  f.store.atomic(() => f.commands.applyReceipts(records));
  const latest = (await f.call(f.owner, 'palpo.inbox.get', { id: row.id })).action;
  assert.equal(latest.execution, 'done');
  const currentRequest = f.store.state.requests[request.id];
  assert.equal(currentRequest.state, 'provisioning'); assert.equal(currentRequest.usable, false);
  const noticeCount = Object.keys(f.inbox.state.notices).length;
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(currentFleet, [e.receipt])));
  assert.equal(Object.keys(f.inbox.state.notices).length, noticeCount);
});

test('unassigned server admins, self approval and changed policy cannot decide agents', async t => {
  const f = await setup(t, { administrators: ['@owner:example.test', '@other:example.test'], allowSelfApproval: false });
  const { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  await assert.rejects(f.decide(row, {}, f.owner), e => e.code === 'project_administrator_required');
  await assert.rejects(f.decide(row, {}, f.admin), e => e.status === 404);
  const grant = f.commands.state.grants[row.grantId];
  grant.desiredAdministrators = ['@owner:example.test']; grant.desiredRevision = grant.grant.revision + 1;
  await assert.rejects(f.decide(row), e => e.status === 404);
  assert.equal((await f.call(f.assigned, 'palpo.inbox.list', {})).pendingCount, 0);
  assert.equal(row.state, 'requested'); assert.equal(f.entry(), undefined);
});

test('an explicitly allowed self approval is accepted and locked actors are refused', async t => {
  const f = await setup(t, { administrators: ['@owner:example.test'], allowSelfApproval: true });
  const { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  f.users.get('@owner:example.test').locked = true;
  await assert.rejects(f.decide(row, {}, f.owner), e => e.code === 'project_administrator_required');
  f.users.get('@owner:example.test').locked = false;
  await f.decide(row, {}, f.owner); assert.equal(f.entry().command.actorMxid, '@owner:example.test');
});

test('changed room ownership and competing agent promises fail before a decision is committed', async t => {
  const f = await setup(t), first = (await f.submit()).request;
  const row = f.inbox.state.records[first.actionId], state = f.rooms.get(f.project.roomId).state;
  const member = state.find(e => e.type === 'm.room.member' && e.state_key === '@owner:example.test');
  member.content.membership = 'leave';
  await assert.rejects(f.decide(row), e => e.code === 'project_binding_conflict');
  member.content.membership = 'join';
  await f.decide(row, { allocatedTokens: 100000 });
  const second = (await f.submit({ ...f.input, requestId: 'agent_two', requestedTokens: 350000,
    agentDefinition: { ...f.input.agentDefinition, name: 'AnotherBot' } })).request;
  await assert.rejects(f.decide(f.inbox.state.records[second.actionId], { commandId: 'second_decision', allocatedTokens: 350000 }), e => e.code === 'project_capacity_unavailable');
  assert.equal(f.inbox.state.records[second.actionId].state, 'requested');
});

test('lost source-event replies recover one agent action and rejected decisions use the same scoped path', async t => {
  const f = await setup(t), fetch = f.palpo.fetch;
  let lose = true;
  f.palpo.fetch = async (url, options) => {
    const result = await fetch(url, options);
    if (lose && new URL(url).pathname.includes('/send/com.hagency.engagement.request.v1/')) { lose = false; throw new Error('lost event reply'); }
    return result;
  };
  await assert.rejects(f.submit());
  assert.equal(Object.values(f.inbox.state.records).filter(r => r.kind === 'agent').length, 0);
  const { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  assert.equal([...f.events.values()].filter(e => e.type === 'com.hagency.engagement.request.v1').length, 1);
  await f.decide(row, { decision: 'reject', allocatedTokens: undefined });
  assert.equal(f.entry().command.operation.kind, 'reject_agent');
  await f.accept(f.entry());
  assert.equal(f.inbox.get(row.id, '@owner:example.test', false).action.execution, 'done');
  assert.equal(f.store.state.requests[request.id].state, 'rejected');
});

test('status cannot bypass a project decision, substitute an engagement, or arrive before its receipt', async t => {
  const f = await setup(t), { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  const stored = f.store.state.requests[request.id];
  const status = { ...stored.payload, sourceEventId: stored.sourceEventId, engagementId: `en_${'a'.repeat(32)}`,
    state: 'active', ready: true, bound: true, observedAt: new Date().toISOString() };
  const publish = (statuses, receipts = []) => f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation,
    sequence: f.fleet.transport.sequence + 1, heartbeat: true, statuses, commandReceipts: receipts }, f.workflow);
  await publish([status]); assert.equal(stored.provider, undefined);
  await f.decide(row); const receipt = f.receipt(f.entry());
  status.engagementId = receipt.outcome.result.engagementId;
  await publish([status]); assert.equal(stored.provider, undefined);
  await publish([status], [receipt]);
  assert.equal(f.inbox.get(row.id, '@owner:example.test', false).action.execution, 'done');
  assert.equal(stored.provider.engagementId, status.engagementId);
  assert.equal(stored.usable, false); // Actual room readiness still needs its read check.
  await assert.rejects(publish([{ ...status, engagementId: `en_${'b'.repeat(32)}` }]), e => e.code === 'agent_decision_pending');
});

test('execution authorization is bound to the canonical decision and current contribution', async t => {
  const f = await setup(t), { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  await f.decide(row); const e = f.entry(), query = { commandId: e.command.commandId, commandDigest: e.digest };
  assert.equal((await f.commands.authorize(f.fleet, query)).allowed, true);
  row.state = 'rejected'; assert.equal((await f.commands.authorize(f.fleet, query)).allowed, false);
  row.state = 'approved';
  Object.values(f.commands.state.contributions)[0].state = 'revoked';
  assert.equal((await f.commands.authorize(f.fleet, query)).allowed, false);
});

test('owner token increases keep the same agent and require an assigned administrator and applied receipt', async t => {
  const f = await setup(t), { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  await f.decide(row); await f.accept(f.entry());
  const events = f.events.size;
  const input = { requestId: 'token_increase', kind: 'top_up', agentRequestId: request.id, addTokens: '50000', reason: 'Continue research' };
  await assert.rejects(f.call(f.assigned, 'palpo.inbox.submit', input), e => e.status === 404);
  const action = (await f.call(f.owner, 'palpo.inbox.submit', input)).action;
  assert.equal(action.kind, 'top_up'); assert.equal(f.events.size, events);
  assert.equal((await f.call(f.owner, 'palpo.inbox.submit', input)).action.id, action.id);
  await assert.rejects(f.call(f.owner, 'palpo.inbox.submit', { ...input, addTokens: 60000 }), e => e.code === 'idempotency_conflict');
  await assert.rejects(f.decide(action, { allocatedTokens: 40000 }, f.owner), e => e.code === 'project_administrator_required');
  const decision = { ...f.decideInput(action), allocatedTokens: 40000 };
  await f.call(f.assigned, 'palpo.inbox.decide', decision);
  const e = Object.values(f.commands.state.commands).find(e => e.actionId === action.id);
  assert.equal(e.command.operation.kind, 'top_up_agent');
  assert.equal(e.command.operation.engagementId, row.result.engagementId);
  assert.equal((await f.commands.authorize(f.fleet, { commandId: e.command.commandId, commandDigest: e.digest })).allowed, true);
  const stored = f.store.state.requests[request.id];
  assert.deepEqual(f.inbox.agents.allocation(stored), { tokens: 80000, pendingTokens: 40000 });
  const receipt = { v: 1, fleetId: f.fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest,
    completedAtMs: f.inbox.now(), outcome: { status: 'applied', result: { kind: 'agent', engagementId: row.result.engagementId,
      state: 'reserved', allocatedTokens: 120000, cleanup: 'not_required' } } };
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(f.fleet, [receipt])));
  assert.deepEqual(f.inbox.agents.allocation(stored), { tokens: 120000, pendingTokens: 0 });
  await f.call(f.assigned, 'palpo.inbox.decide', decision);
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(f.fleet, [receipt])));
  assert.deepEqual(f.inbox.agents.allocation(stored), { tokens: 120000, pendingTokens: 0 });
  assert.equal(Object.keys(f.store.state.requests).length, 1); assert.equal(f.events.size, events);
});

test('rejected top-ups never enqueue work or change an existing allocation', async t => {
  const f = await setup(t), { request } = await f.submit(), row = f.inbox.state.records[request.actionId];
  await f.decide(row); await f.accept(f.entry());
  const input = { requestId: 'rejected_increase', kind: 'top_up', agentRequestId: request.id, addTokens: 50000, reason: 'More work' };
  const action = (await f.call(f.owner, 'palpo.inbox.submit', input)).action;
  const before = Object.keys(f.commands.state.commands).length;
  const result = await f.decide(action, { decision: 'reject', allocatedTokens: undefined });
  assert.equal(result.action.state, 'rejected'); assert.equal(result.action.execution, 'done');
  assert.equal(Object.keys(f.commands.state.commands).length, before);
  assert.deepEqual(f.inbox.agents.allocation(f.store.state.requests[request.id]), { tokens: 80000, pendingTokens: 0 });
  await assert.rejects(f.call(f.owner, 'palpo.inbox.submit', { ...input, requestId: 'too_much', addTokens: 400000 }), e => e.code === 'project_capacity_unavailable');
});

test('agent notices reach assigned users and stop after their account is locked', async t => {
  const f = await setup(t), { request } = await f.submit();
  // Reuse a test identity for the sender; only this action's recipients are due.
  for (const notice of Object.values(f.inbox.state.notices)) if (notice.actionId !== request.actionId) notice.cancelled = true;
  const worker = new ActionNotifications(f.inbox, { homeserverOrigin: 'https://matrix.example.test',
    botMxid: '@admin:example.test', botToken: 'admin-secret', adminToken: 'admin-secret', approvers: ['@owner:example.test'] });
  await worker.tick();
  const notices = Object.values(f.inbox.state.notices).filter(n => n.actionId === request.actionId);
  assert.equal(notices.length, 2); assert.ok(notices.every(n => n.delivered === 1));
  const assigned = notices.find(n => n.recipient === '@other:example.test');
  const event = f.events.get(assigned.eventId);
  assert.equal(event.content['im.palpo.action.v1'].ownerMxid, '@other:example.test');
  assert.ok(!JSON.stringify(event.content).includes(f.project.ownerDmRoomId));
  f.users.get('@other:example.test').locked = true; assigned.dueAt = 0;
  await worker.tick(); assert.equal(assigned.cancelled, true); assert.equal(assigned.delivered, 1);
  assert.equal(f.inbox.state.records[request.actionId].state, 'requested');
});
