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
  return { ...f, inbox, workflow, commands, fleet, owner, admin, call, input, submit, decide };
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
