import test from 'node:test';
import assert from 'node:assert/strict';
import { agentWorkflow } from './agent-workflow-fixture.mjs';

async function setup(t) {
  const f = await agentWorkflow(t), submitted = (await f.submit()).request;
  await f.decide(f.inbox.state.records[submitted.actionId]); await f.accept(f.entry());
  const request = f.store.state.requests[submitted.id], engagement = f.entry().receipt.outcome.result.engagementId;
  const mxid = `@${f.fleet.id}_${engagement}:example.test`;
  const input = { requestId: 'remove_one', kind: 'agent_removal', agentRequestId: request.id, reason: 'This work is complete' };
  const remove = (extra = {}, actor = f.owner) => f.call(actor, 'palpo.inbox.submit', { ...input, ...extra });
  const removalEntry = () => f.commands.entry(f.fleet, f.inbox.state.records[request.removalActionId].commandRef);
  const acceptRemoval = async (entry = removalEntry()) => {
    await f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: f.fleet.transport.sequence + 1, heartbeat: true,
      commandReceipts: [{ v: 1, fleetId: f.fleet.id, registrationGeneration: 1, commandId: entry.command.commandId, commandDigest: entry.digest,
        completedAtMs: Date.now(), outcome: { status: 'applied', result: { kind: 'agent', engagementId: engagement, state: 'revoked', allocatedTokens: 80000, cleanup: 'pending' } } }] }, f.workflow);
  };
  const observation = (patch = {}) => ({ v: 1, agentMxid: mxid, localCleanup: 'pending', runtimeStopped: false, cleanupRetryable: false, cleanupAttempt: 1,
    matrixRetired: false, endedAtMs: Date.now(), allocatedTokens: 80000, spentTokensLowerBound: null, quotaPaused: false, ...patch });
  const publish = async (patch = {}, extra = {}) => {
    const status = { ...request.payload, sourceEventId: request.sourceEventId, engagementId: engagement, state: 'ended',
      agentMxid: mxid, bound: false, ready: false, observedAt: new Date().toISOString(), lifecycle: observation(patch), ...extra };
    await f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: f.fleet.transport.sequence + 1, heartbeat: true, statuses: [status] }, f.workflow);
  };
  return { ...f, request, mxid, input, remove, removalEntry, acceptRemoval, publish };
}

test('owner removal is immediate scoped intent, not another human approval; retry is atomic and canonical', async t => {
  const f = await setup(t);
  await assert.rejects(f.remove({}, f.admin), e => e.status === 404);
  const before = structuredClone(f.store.state);
  f.service.outbound.maxPending = f.service.outbound.usage(f.fleet).pending;
  await assert.rejects(f.remove(), e => e.code === 'queue_full'); assert.deepEqual(f.store.state, before);
  f.service.outbound.maxPending = 100;
  // A rollback replaces state objects; resolve the live request again.
  f.request = f.store.state.requests[f.request.id];
  const { action } = await f.remove();
  const request = f.store.state.requests[f.request.id], row = f.inbox.state.records[action.id];
  assert.equal(action.execution, 'awaiting_hagency'); assert.equal(action.canDecide, false);
  const entry = f.commands.entry(f.service.fleet(f.fleet.id), row.commandRef);
  assert.equal(entry.command.actorMxid, '@owner:example.test'); assert.equal(entry.command.operation.kind, 'revoke_agent');
  assert.equal((await f.remove()).action.id, action.id);
  await assert.rejects(f.remove({ reason: 'Changed decision' }), e => e.code === 'idempotency_conflict');
  await assert.rejects(f.remove({ requestId: 'remove_again' }), e => e.status === 404);
  assert.equal(f.inbox.agents.canTopUp(request, '@owner:example.test'), false);
  const query = { commandId: entry.command.commandId, commandDigest: entry.digest };
  assert.equal((await f.commands.authorize(f.service.fleet(f.fleet.id), query)).allowed, true);
  f.users.get('@owner:example.test').locked = true;
  assert.equal((await f.commands.authorize(f.service.fleet(f.fleet.id), query)).allowed, false);
  assert.equal(Object.values(f.commands.state.commands).filter(e => e.command.operation.kind === 'revoke_agent').length, 1);
});

test('expired or revoked allocation still permits cleanup by its owner and current assigned administrator', async t => {
  const f = await setup(t), original = f.inbox.state.records[f.request.actionId];
  const record = f.commands.state.grants[original.grantId]; record.state = 'revoked'; record.grant.expiresAtMs = 1;
  const { action } = await f.remove({}, f.assigned);
  assert.equal(action.requesterMxid, '@other:example.test'); assert.equal(action.ownerMxid, '@owner:example.test');
  const entry = f.removalEntry();
  assert.equal((await f.commands.authorize(f.fleet, { commandId: entry.command.commandId, commandDigest: entry.digest })).allowed, true);
  record.desiredAdministrators = ['@owner:example.test']; record.desiredRevision = record.grant.revision + 1;
  assert.equal((await f.commands.authorize(f.fleet, { commandId: entry.command.commandId, commandDigest: entry.digest })).allowed, false);
});

test('removal waits for runtime and verified Matrix retirement; old status cannot revive it', async t => {
  const f = await setup(t); const { action } = await f.remove(); await f.acceptRemoval();
  const grant = f.commands.state.grants[f.inbox.state.records[action.id].grantId].grant;
  const held = f.inbox.agents.remaining(grant);
  const row = f.inbox.state.records[action.id]; assert.equal(row.execution, 'retiring');
  await f.publish({ localCleanup: 'complete', runtimeStopped: true, matrixRetired: true });
  assert.equal(row.execution, 'retiring', 'a provider claim cannot replace Palpo deactivation verification');
  assert.deepEqual(f.inbox.agents.remaining(grant), held);
  f.users.set(f.mxid, { name: f.mxid, appservice_id: f.fleet.id, deactivated: false, displayname: 'ResearchBot', rooms: [f.project.roomId] });
  await f.service.retireAllocatedAgent(f.fleet, { requestId: f.request.requestId, agentMxid: f.mxid, endedAt: Date.now(), localStopped: true }, 'admin-secret');
  await f.publish({ localCleanup: 'complete', runtimeStopped: true, matrixRetired: true });
  assert.equal(row.execution, 'done'); assert.equal(f.request.state, 'removed');
  assert.equal(f.users.get(f.mxid).deactivated, true);
  assert.deepEqual(f.inbox.agents.remaining(grant), { tokens: held.tokens, maxAgents: held.maxAgents + 1,
    maxRatePerDay: held.maxRatePerDay + f.request.payload.ratePerDay });
  await f.publish({}, { state: 'active', ready: true, bound: true });
  assert.equal(f.request.state, 'removed'); assert.equal(f.request.usable, false);
  const view = await f.call(f.owner, 'palpo.requests.list', {});
  assert.equal(view.requests[0].state, 'removed'); assert.equal(view.requests[0].canRemove, false);
});

test('uncertain cleanup is never blindly retried; a definitive failure permits one fresh cleanup command', async t => {
  const f = await setup(t); const { action } = await f.remove(); await f.acceptRemoval();
  await f.publish({ localCleanup: 'uncertain' });
  let row = (await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action;
  assert.equal(row.execution, 'inspection_required'); assert.equal(row.canRetry, false); assert.equal(row.needsMyAction, true);
  await assert.rejects(f.remove({ requestId: 'unsafe_retry' }), e => e.status === 404);
  await f.publish({ cleanupRetryable: true });
  row = (await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action;
  assert.equal(row.execution, 'cleanup_failed'); assert.equal(row.canRetry, true);
  const retry = await f.remove({ requestId: 'retry_cleanup' });
  assert.notEqual(retry.action.id, action.id);
  assert.equal((await f.remove({ requestId: 'retry_cleanup' })).action.id, retry.action.id);
  assert.equal((await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action.canRetry, false);
  await f.acceptRemoval();
  await f.publish({ cleanupRetryable: true, cleanupAttempt: 1 });
  assert.equal((await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action.canRetry, false, 'an old failure cannot trigger a second physical retry');
  await assert.rejects(f.remove({ requestId: 'stale_failure_retry' }), e => e.status === 404);
  await f.publish({ cleanupRetryable: true, cleanupAttempt: 2 });
  assert.equal((await f.call(f.owner, 'palpo.inbox.get', { id: action.id })).action.canRetry, true);
  assert.equal(Object.values(f.commands.state.commands).filter(e => e.command.operation.kind === 'revoke_agent').length, 2);
});

test('lifecycle rejects invented cleanup facts and keeps unknown, stale usage explicit', async t => {
  const f = await setup(t); await f.remove(); await f.acceptRemoval();
  await f.publish({ spentTokensLowerBound: null });
  let view = f.inbox.lifecycle.view(f.request); assert.equal(view.status.spentTokensLowerBound, null); assert.equal(view.stale, false);
  const sequence = f.fleet.transport.sequence;
  await assert.rejects(f.publish({ runtimeStopped: true, localCleanup: 'pending' }), e => e.code === 'invalid_agent_lifecycle');
  await assert.rejects(f.publish({ cleanupRetryable: true, localCleanup: 'uncertain' }), e => e.code === 'invalid_agent_lifecycle');
  await assert.rejects(f.publish({ cleanupRetryable: true, localCleanup: 'complete' }), e => e.code === 'invalid_agent_lifecycle');
  assert.equal(f.fleet.transport.sequence, sequence);
  await f.publish({ spentTokensLowerBound: 500 }, { observedAt: '2020-01-01T00:00:00Z' });
  view = f.inbox.lifecycle.view(f.request); assert.equal(view.stale, true); assert.equal(view.status.spentTokensLowerBound, 500);
});
