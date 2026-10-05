import test from 'node:test';
import assert from 'node:assert/strict';
import { agentWorkflow } from './agent-workflow-fixture.mjs';
import { publishReadyAgent } from './project-workflow-fixture.mjs';

async function setup(t) {
  const f = await agentWorkflow(t), submitted = (await f.submit()).request;
  await f.decide(f.inbox.state.records[submitted.actionId]); await f.accept(f.entry());
  const request = f.store.state.requests[submitted.id];
  const open = (session = f.owner, extra = {}) => f.call(session, 'palpo.requests.open', { requestId: request.id, ...extra });
  const ready = () => publishReadyAgent(f, f.workflow, f.inbox, request);
  return { ...f, request, open, ready };
}
const unavailable = e => e.code === 'agent_chat_unavailable';

test('chat navigation requires actual readiness, exact grant and current owner; it sends no Matrix mutation', async t => {
  const f = await setup(t);
  assert.equal((await f.call(f.owner, 'palpo.requests.list', {})).requests[0].canOpenChat, false);
  await assert.rejects(f.open(), unavailable);
  const agentMxid = await f.ready();
  const writes = f.calls.filter(c => c.method !== 'GET').length;
  const commands = Object.keys(f.commands.state.commands);
  assert.equal((await f.call(f.owner, 'palpo.requests.list', {})).requests[0].canOpenChat, true);
  assert.deepEqual(await f.open(), { v: 1, requestId: f.request.id, account: '@owner:example.test', roomId: f.project.roomId, agentMxid });
  const readOnly = await f.login('owner-secret', ['palpo.requests.list']);
  await assert.rejects(f.open(readOnly), e => e.status === 403);
  await assert.rejects(f.open(f.admin), unavailable);
  await assert.rejects(f.open(f.assigned), unavailable);
  await assert.rejects(f.open(f.owner, { roomId: '!attacker:example.test' }), e => e.code === 'invalid_arguments');
  assert.equal(f.calls.filter(c => c.method !== 'GET').length, writes);
  assert.deepEqual(Object.keys(f.commands.state.commands), commands);
});

test('fresh navigation refuses missing membership, expired grants and removal even after an earlier ready view', async t => {
  const f = await setup(t), mxid = await f.ready();
  await f.open();
  const room = f.rooms.get(f.project.roomId);
  f.putState(room, 'm.room.member', mxid, { membership: 'leave' }, mxid);
  await assert.rejects(f.open(), unavailable);
  await f.ready();
  f.putState(room, 'm.room.member', '@owner:example.test', { membership: 'leave' }, '@owner:example.test');
  await assert.rejects(f.open(), unavailable);
  f.putState(room, 'm.room.member', '@owner:example.test', { membership: 'join' }, '@owner:example.test');
  await f.ready();
  const grant = f.commands.state.grants[f.inbox.state.records[f.request.actionId].grantId];
  const expiry = grant.grant.expiresAtMs; grant.grant.expiresAtMs = 1;
  await assert.rejects(f.open(), unavailable); grant.grant.expiresAtMs = expiry;
  await f.open();
  await f.call(f.owner, 'palpo.inbox.submit', { requestId: 'remove_for_navigation', kind: 'agent_removal', agentRequestId: f.request.id, reason: 'Stop work' });
  await assert.rejects(f.open(), unavailable);
});

test('authorization changes while Matrix readiness is being read cannot emit a destination', async t => {
  const f = await setup(t); await f.ready();
  const roomState = f.workflow.roomState.bind(f.workflow);
  f.workflow.roomState = async (...args) => {
    const state = await roomState(...args);
    f.store.state.projects[f.project.id].ownerMxid = '@other:example.test';
    return state;
  };
  await assert.rejects(f.open(), unavailable);
});

test('stale provider status, paused fleets and changed grant revisions disable navigation', async t => {
  const f = await setup(t); await f.ready(); await f.open();
  f.request.outboundStatus.observedAt = '2020-01-01T00:00:00Z';
  await assert.rejects(f.open(), unavailable);
  await f.ready(); f.fleet.state = 'paused';
  await assert.rejects(f.open(), unavailable);
  f.fleet.state = 'ready'; await f.ready();
  const grant = f.commands.state.grants[f.inbox.state.records[f.request.actionId].grantId];
  grant.desiredRevision = grant.grant.revision + 1;
  await assert.rejects(f.open(), unavailable);
});
