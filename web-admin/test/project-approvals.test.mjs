import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';
import { contributedFleet, projectBudget, projectAdministrators, acceptProjectReservations, projectResource } from './project-workflow-fixture.mjs';

async function setup(t) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false,
    startActionWorker: false, inboxOptions: { projectApprover: '@admin:example.test', requireProjectApproval: true } });
  t.after(() => f.store.close());
  const inbox = server.inbox, workflow = inbox.workflow, commands = server.projectCommands;
  commands.adminToken = 'admin-secret';
  const fleet = await contributedFleet(f, workflow, inbox);
  const login = async token => (await server.miniapp.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: Object.keys(SERVICES) })).sessionToken;
  const owner = await login('owner-secret'), admin = await login('admin-secret');
  const call = (session, service, args) => server.miniapp.call(`Bearer ${session}`, { service, args });
  const input = { requestId: 'budgeted_project', kind: 'project', name: 'Budgeted project', reason: 'Build an agent', fleetId: fleet.id,
    resourceIds: [projectResource], allocations: projectBudget() };
  const submit = (body = input) => call(owner, 'palpo.inbox.submit', body);
  const decide = (action, extra = {}) => call(admin, 'palpo.inbox.decide', { id: action.id, expectedRevision: action.revision,
    commandId: 'project_approval', decision: 'approve', reason: 'Reviewed finite budget', ...projectAdministrators, ...extra });
  return { ...f, server, inbox, workflow, commands, fleet, owner, admin, call, input, submit, decide };
}

test('catalog publication, missing budgets, stale contributions and invalid limits cannot prepare an allocated project', async t => {
  const f = await setup(t), before = f.rooms.size;
  const missing = structuredClone(f.input); delete missing.allocations;
  await assert.rejects(f.submit(missing), e => e.code === 'project_budget_required');
  for (const value of [0, true, ' ', Number.MAX_SAFE_INTEGER + 1]) {
    const input = structuredClone(f.input); input.allocations[0].limits.tokens = value;
    await assert.rejects(f.submit(input), e => e.code === 'invalid_project_budget');
  }
  const old = Object.values(f.commands.state.contributions)[0];
  old.observedAtMs -= 90001;
  await assert.rejects(f.submit(), e => e.code === 'contribution_unavailable');
  old.observedAtMs += 90001;
  delete f.fleet.projectWorkflow;
  await assert.rejects(f.submit(), e => e.code === 'project_workflow_unavailable');
  assert.equal(f.rooms.size, before);
  assert.equal(Object.keys(f.inbox.state.records).length, 0);
});

test('room preparation recovers a lost Matrix reply using the same operation and preserves owner', async t => {
  const f = await setup(t), fetch = f.palpo.fetch;
  let lose = true;
  f.palpo.fetch = async (url, options) => {
    const response = await fetch(url, options);
    if (lose && new URL(url).pathname.endsWith('/createRoom') && JSON.parse(options.body).name === f.input.name) {
      lose = false; throw new Error('lost room reply after creation');
    }
    return response;
  };
  await assert.rejects(f.submit());
  const pending = Object.values(f.inbox.state.records)[0];
  assert.equal(pending.execution, 'preparing');
  assert.equal(f.inbox.get(pending.id, '@admin:example.test', true).action.canDecide, false);
  const { action } = await f.submit();
  assert.equal(action.id, pending.id); assert.equal(action.execution, 'pending'); assert.equal(action.revision, 2);
  assert.equal(f.calls.filter(c => c.path.endsWith('/createRoom') && c.body.name === f.input.name).length, 1);
  assert.equal(f.store.state.projects[action.result.projectId].ownerMxid, '@owner:example.test');
  assert.equal(f.store.state.projects[action.result.projectId].state, 'awaiting_approval');
});

test('missing administrator policy, inactive assignees and a changed room prevent approval', async t => {
  const f = await setup(t), { action } = await f.submit();
  await assert.rejects(f.decide(action, { administrators: [] }), e => e.code === 'project_administrators_required');
  await assert.rejects(f.decide(action, { allowSelfApproval: undefined }), e => e.code === 'project_administrators_required');
  f.users.get('@other:example.test').locked = true;
  await assert.rejects(f.decide(action), e => e.code === 'project_administrator_unavailable');
  f.users.get('@other:example.test').locked = false;
  const room = f.rooms.get(action.result.roomId);
  room.state.find(e => e.type === 'm.room.member' && e.state_key === '@owner:example.test').content.membership = 'leave';
  await assert.rejects(f.decide(action), e => e.code === 'project_binding_conflict');
  assert.equal(f.inbox.state.records[action.id].state, 'requested');
  assert.equal(Object.keys(f.commands.state.commands).length, 0);
});

test('a human decision, its outbound reservations and notifications roll back together', async t => {
  const f = await setup(t), { action } = await f.submit();
  const before = structuredClone(f.store.state), pending = f.service.outbound.usage(f.fleet).pending;
  f.service.outbound.maxPending = pending;
  await assert.rejects(f.decide(action), e => e.code === 'queue_full');
  assert.deepEqual(f.store.state, before);
  assert.equal(f.service.outbound.usage(f.service.fleet(f.fleet.id)).pending, pending);
  f.service.outbound.maxPending = 1000;
  const result = await f.decide(action);
  assert.equal(result.action.execution, 'awaiting_reservation');
  assert.equal(Object.keys(f.commands.state.commands).length, 1);
});

test('a delivery ACK is not allocation; exact receipts commit progress once and old cards read the latest result', async t => {
  const f = await setup(t), { action } = await f.submit();
  await f.decide(action, { administrators: '@other:example.test' });
  const entry = Object.values(f.commands.state.commands)[0], project = f.store.state.projects[action.result.projectId];
  assert.equal(entry.command.operation.grant.roomId, project.roomId);
  assert.equal(entry.command.operation.grant.ownerMxid, '@owner:example.test');
  assert.deepEqual(entry.command.operation.grant.administratorMxids, ['@other:example.test']);
  assert.equal((await f.commands.authorize(f.fleet, { commandId: entry.command.commandId, commandDigest: entry.digest })).allowed, true);
  const delivery = f.service.outbound.claim(f.fleet, 'work', '12345678-1234-4234-8234-123456789012');
  f.service.outbound.ack(f.fleet, { lane: 'work', id: delivery.id, token: delivery.token });
  assert.equal(f.inbox.get(action.id, '@owner:example.test', false).action.execution, 'awaiting_reservation');
  assert.equal((await f.workflow.projectView(project, '@owner:example.test', 'owner-secret')).canRequest, false);
  await acceptProjectReservations(f, f.workflow, f.inbox);
  const result = f.inbox.get(action.id, '@owner:example.test', false).action;
  assert.equal(result.execution, 'done'); assert.equal(result.revision, action.revision + 2);
  assert.equal((await f.workflow.projectView(project, '@owner:example.test', 'owner-secret')).canRequest, true);
  const count = Object.keys(f.inbox.state.notices).length;
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(f.fleet, [entry.receipt])));
  assert.equal(f.inbox.get(action.id, '@owner:example.test', false).action.revision, result.revision);
  assert.equal(Object.keys(f.inbox.state.notices).length, count);
});

test('two queued approvals cannot promise the same remaining contribution budget', async t => {
  const f = await setup(t);
  const input = structuredClone(f.input); input.allocations[0].limits.tokens = 1500000;
  const first = (await f.submit(input)).action;
  const second = (await f.submit({ ...input, requestId: 'second_project' })).action;
  await f.decide(first);
  await assert.rejects(f.decide(second, { commandId: 'second_decision' }), e => e.code === 'project_capacity_unavailable');
  assert.equal(f.inbox.state.records[second.id].state, 'requested');
  assert.equal(Object.keys(f.commands.state.commands).length, 1);
});

test('a refused reservation remains unallocated and legacy projects never gain an implicit budget', async t => {
  const f = await setup(t), { action } = await f.submit(); await f.decide(action);
  const entry = Object.values(f.commands.state.commands)[0];
  await f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: f.fleet.transport.sequence + 1, heartbeat: true,
    commandReceipts: [{ v: 1, fleetId: f.fleet.id, registrationGeneration: 1, commandId: entry.command.commandId, commandDigest: entry.digest,
      completedAtMs: f.inbox.now(), outcome: { status: 'refused', code: 'insufficient_capacity' } }] }, f.workflow);
  const project = f.store.state.projects[action.result.projectId];
  assert.equal(f.inbox.get(action.id, '@owner:example.test', false).action.execution, 'reservation_refused');
  assert.equal((await f.workflow.projectView(project, '@owner:example.test', 'owner-secret')).canRequest, false);
  assert.throws(() => f.inbox.resourceGrant({ ...project, resourceGrant: undefined }, projectResource), e => e.code === 'project_allocation_required');
});

async function partial(t) {
  const f = await setup(t), resourceId = `resource_${'b'.repeat(24)}`;
  f.fleet.capabilities.offers[0].resources.push({ ...f.fleet.capabilities.offers[0].resources[0], id: resourceId, name: 'Recovery analysis resource' });
  const original = Object.values(f.commands.state.contributions)[0];
  const second = { grant: { ...original.grant, id: 'contribution_second', resourceId }, state: 'active', reserved: { tokens: 0, maxAgents: 0, maxRatePerDay: 0 } };
  await f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: f.fleet.transport.sequence + 1, heartbeat: true,
    contributionPage: { v: 1, registrationGeneration: 1, observedAtMs: f.inbox.now(), after: '', nextAfter: null, contributions: [second] } }, f.workflow);
  const input = structuredClone(f.input); input.resourceIds.push(resourceId);
  input.allocations.push({ ...structuredClone(input.allocations[0]), contributionId: second.grant.id, resourceId });
  const { action } = await f.submit(input); await f.decide(action);
  const entries = Object.values(f.commands.state.commands);
  const publish = async (entry, status, code = 'insufficient_capacity') => {
    const op = entry.command.operation;
    const outcome = status === 'refused' ? { status, code } : { status, result: op.kind === 'reserve_project'
      ? { kind: 'grant', grant: op.grant } : { kind: 'released_unused_project', grantId: op.grantId, revision: op.expectedRevision } };
    const receipt = { v: 1, fleetId: f.fleet.id, registrationGeneration: 1, commandId: entry.command.commandId, commandDigest: entry.digest, completedAtMs: f.inbox.now(), outcome };
    await f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: f.fleet.transport.sequence + 1, heartbeat: true, commandReceipts: [receipt] }, f.workflow);
    return receipt;
  };
  await publish(entries[0], 'applied'); await publish(entries[1], 'refused');
  const latest = () => f.inbox.get(action.id, '@admin:example.test', true).action;
  const body = (operation = 'retry', commandId = 'recover') => ({ id: action.id, expectedRevision: latest().revision, commandId, operation, reason: 'Recover the failed allocation' });
  const recover = (input = body(), session = f.admin) => f.call(session, 'palpo.inbox.recover', input);
  return { ...f, action, entries, publish, latest, body, recover };
}

test('partial refusal retries only the failed budget with current designated authority and exact replay', async t => {
  const f = await partial(t), before = structuredClone(f.entries[0].command), input = f.body();
  const old = (await f.server.miniapp.open('Bearer admin-secret', { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: ['palpo.inbox.decide'] })).sessionToken;
  await assert.rejects(f.recover(input, old), e => e.code === 'service_not_granted');
  assert.equal(f.latest().needsMyAction, true); assert.equal(f.latest().canContinue, false);
  assert.equal(f.latest().payload.allocations[1].resourceName, 'Recovery analysis resource');
  assert.equal(f.latest().reservations[1].resourceName, 'Recovery analysis resource');
  assert.notEqual(f.latest().reservations[0].resourceName, f.latest().reservations[1].resourceName);
  await assert.rejects(f.recover(input, f.owner), e => e.status === 403);
  f.users.get('@other:example.test').admin = true;
  // The base Matrix fixture has one admin token; explicitly model a second
  // server admin here to exercise the separate designated business role.
  const requireAdmin = f.palpo.requireAdmin.bind(f.palpo);
  f.palpo.requireAdmin = token => token === 'other-secret' ? Promise.resolve() : requireAdmin(token);
  const otherAdmin = (await f.server.miniapp.open('Bearer other-secret', { appId: APP_ID, bundleDigest: 'a'.repeat(64), services: ['palpo.inbox.recover'] })).sessionToken;
  await assert.rejects(f.recover(input, otherAdmin), e => e.code === 'project_approver_required');
  await assert.rejects(f.recover({ ...input, expectedRevision: input.expectedRevision - 1 }), e => e.code === 'recovery_conflict');
  await f.recover(input); await f.recover(input);
  const entries = Object.values(f.commands.state.commands), retry = entries.at(-1);
  assert.equal(entries.length, 3); assert.deepEqual(f.entries[0].command, before);
  assert.deepEqual(retry.command.operation, f.entries[1].command.operation);
  assert.notEqual(retry.command.commandId, f.entries[1].command.commandId);
  assert.equal((await f.commands.authorize(f.fleet, { commandId: retry.command.commandId, commandDigest: retry.digest })).allowed, true);
  await assert.rejects(f.recover({ ...input, reason: 'Changed retry' }), e => e.code === 'recovery_conflict');
  await f.publish(retry, 'applied');
  assert.equal(f.latest().execution, 'done');
  assert.equal(f.inbox.projects.allocation(f.store.state.projects[f.action.result.projectId]).ready, true);
  const revision = f.latest().revision;
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(f.fleet, [f.entries[1].receipt])));
  await f.recover(input); assert.equal(f.latest().revision, revision); assert.equal(f.latest().execution, 'done');
});

test('failed allocation release waits for exact unused proof and never revives from old receipts', async t => {
  const f = await partial(t), input = f.body('release');
  await f.recover(input); await f.recover(input);
  const release = Object.values(f.commands.state.commands).at(-1);
  assert.equal(release.command.operation.kind, 'release_unused_project');
  assert.equal(f.latest().execution, 'releasing_reservations');
  assert.equal(f.latest().releases[0].resourceName, f.latest().reservations[0].resourceName);
  assert.equal(f.commands.state.grants[release.command.operation.grantId].state, 'accepted');
  assert.equal((await f.commands.authorize(f.fleet, { commandId: release.command.commandId, commandDigest: release.digest })).allowed, true);
  await f.publish(release, 'refused', 'authority');
  assert.equal(f.latest().execution, 'release_refused'); assert.equal(f.latest().canReleaseReservation, true);
  const retryInput = f.body('release', 'release_retry'); await f.recover(retryInput);
  const retry = Object.values(f.commands.state.commands).at(-1); await f.publish(retry, 'applied');
  assert.equal(f.latest().state, 'cancelled'); assert.equal(f.latest().execution, 'released');
  assert.equal(f.latest().canReleaseReservation, false);
  assert.equal(f.inbox.list('@admin:example.test', true).pendingCount, 0);
  assert.equal(f.inbox.list('@owner:example.test', false, { view: 'history' }).total, 1);
  assert.equal(f.commands.state.grants[retry.command.operation.grantId].state, 'released');
  f.store.atomic(() => f.commands.applyReceipts(f.commands.validateReceipts(f.fleet, [f.entries[0].receipt, release.receipt])));
  assert.equal(f.commands.state.grants[retry.command.operation.grantId].state, 'released');
  assert.equal(f.latest().execution, 'released');
  assert.equal(f.inbox.projects.allocation(f.store.state.projects[f.action.result.projectId]).ready, false);
  assert.equal(f.store.state.projects[f.action.result.projectId].ownerMxid, '@owner:example.test');
});

test('recovery refuses pending results, unsupported release, expiry, demotion and changed room binding', async t => {
  const f = await partial(t), input = f.body();
  delete f.fleet.projectWorkflow.unusedRelease;
  assert.equal(f.latest().canReleaseReservation, false);
  await assert.rejects(f.recover(f.body('release')), e => e.code === 'recovery_conflict');
  f.fleet.projectWorkflow.unusedRelease = true;
  const row = f.inbox.state.records[f.action.id]; row.reservations[1].state = 'queued';
  await assert.rejects(f.recover(input), e => e.code === 'recovery_conflict'); row.reservations[1].state = 'refused';
  f.users.get('@admin:example.test').locked = true;
  await assert.rejects(f.recover(input), e => e.status === 403); f.users.get('@admin:example.test').locked = false;
  const room = f.rooms.get(f.action.result.roomId), member = room.state.find(e => e.type === 'm.room.member' && e.state_key === '@owner:example.test');
  member.content.membership = 'leave';
  await assert.rejects(f.recover(input), e => e.code === 'project_binding_conflict'); member.content.membership = 'join';
  row.payload.allocations[0].expiresAtMs = f.inbox.now() - 1;
  assert.equal(f.latest().canRetryReservation, false); assert.equal(f.latest().canReleaseReservation, true);
  await assert.rejects(f.recover(input), e => e.code === 'recovery_conflict');
  assert.equal(Object.keys(f.commands.state.commands).length, 2);
});

test('recovery enqueue failure rolls back action, commands, notices and release state', async t => {
  const f = await partial(t), before = structuredClone(f.store.state);
  f.service.outbound.maxPending = f.service.outbound.usage(f.fleet).pending;
  await assert.rejects(f.recover(f.body('release')), e => e.code === 'queue_full');
  assert.deepEqual(f.store.state, before);
});
