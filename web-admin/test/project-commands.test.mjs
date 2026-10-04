import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fixture } from './fixture.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { Inbox } from '../lib/inbox.mjs';
import { ProjectCommands, commandDigest, validateProjectCommand } from '../lib/project-commands.mjs';
import { canonical } from '../lib/outbound.mjs';

test('project commands and receipts match the Hagency Rust wire corpus', () => {
  const corpus = JSON.parse(readFileSync(new URL('./fixtures/project-commands.json', import.meta.url)));
  const fleet = { id: corpus.registration.fleetId, projectWorkflow: { v: 1, registrationGeneration: corpus.registration.generation } };
  for (const vector of corpus.vectors) {
    validateProjectCommand(vector.command, fleet, corpus.registration.serverName);
    assert.equal(canonical(vector.command), vector.canonical);
    assert.equal(commandDigest(vector.command), vector.sha256);
    const grant = corpus.vectors[0].command.operation.grant;
    const entry = { command: vector.command, digest: vector.sha256 };
    const commands = new ProjectCommands({ store: { state: {} } }, {} , { now: () => 1000 });
    commands.state.commands[`${fleet.id}:1:${vector.command.commandId}`] = entry;
    commands.state.grants[grant.id] = { fleetId: fleet.id, grant };
    assert.deepEqual(commands.validateReceipts(fleet, [vector.receipt]), [{ entry, receipt: vector.receipt }]);
  }
});

async function setup(t) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const publicFleet = await f.service.create({ requestId: 'workflow-fleet', name: 'Workflow fleet', ownerMxid: '@owner:example.test' }, '@admin:example.test', 'admin-secret');
  const fleet = f.service.fleet(publicFleet.id);
  fleet.projectWorkflow = { v: 1, registrationGeneration: 1 };
  const workflow = new Workflow(f.service), inbox = new Inbox(f.service, workflow, { projectApprover: '@admin:example.test' });
  const commands = new ProjectCommands(f.service, inbox, { adminToken: 'admin-secret', now: () => 1000 });
  f.service.outbound.projectCommands = commands;
  const project = f.store.state.projects.project_one = { id: 'project_one', fleetId: fleet.id, ownerMxid: '@owner:example.test', roomId: '!project:example.test' };
  inbox.state.records.action_one = { id: 'action_one', state: 'approved', ownerMxid: project.ownerMxid, decision: { by: '@admin:example.test' } };
  const grant = { v: 1, id: 'grant_one', revision: 1, delegationId: 'contribution', delegationRevision: 1, projectId: project.id, roomId: project.roomId,
    ownerMxid: project.ownerMxid, administratorMxids: ['@other:example.test'], allowSelfApproval: false,
    limits: { tokens: 800, maxAgents: 4, maxRatePerDay: 1000 }, expiresAtMs: 90000 };
  const enqueue = (kind = 'reserve_project', op = {}, actor = '@admin:example.test', commandId = 'reserve') => f.store.atomic(() => commands.enqueue(fleet, actor,
    kind === 'reserve_project' ? { kind, grant: structuredClone(grant), ...op } : { kind, ...op }, { commandId, projectId: project.id, actionId: 'action_one', expiresAtMs: 20000 }));
  const receipt = (entry, result = { kind: 'grant', grant: structuredClone(grant) }) => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1,
    commandId: entry.command.commandId, commandDigest: entry.digest, completedAtMs: 1001, outcome: { status: 'applied', result } });
  const accept = receipt => f.store.atomic(() => commands.applyReceipts(commands.validateReceipts(fleet, [receipt])));
  const authorize = entry => commands.authorize(fleet, { commandId: entry.command.commandId, commandDigest: entry.digest });
  t.after(() => f.store.close());
  return { ...f, workflow, inbox, fleet, project, grant, commands, enqueue, receipt, accept, authorize };
}

test('decision and outbound command are atomic; changed retries cannot replace original work', async t => {
  const f = await setup(t), before = structuredClone(f.store.state);
  assert.throws(() => f.store.atomic(() => {
    f.commands.enqueue(f.fleet, '@admin:example.test', { kind: 'reserve_project', grant: f.grant }, { commandId: 'rollback', projectId: f.project.id });
    throw new Error('failed human decision write');
  }));
  assert.deepEqual(f.store.state, before);
  assert.equal(f.store.db.prepare('SELECT COUNT(*) n FROM fleet_delivery').get().n, 0);
  const entry = f.enqueue(); assert.equal(entry.state, 'queued');
  assert.equal(f.enqueue().digest, entry.digest);
  assert.throws(() => f.enqueue('reserve_project', { grant: { ...f.grant, limits: { ...f.grant.limits, tokens: 700 } } }), e => e.code === 'command_conflict');
  assert.equal(f.store.db.prepare('SELECT COUNT(*) n FROM fleet_delivery').get().n, 1);
  assert.throws(() => f.commands.enqueue(f.fleet, '@admin:example.test', { kind: 'reserve_project', grant: f.grant }, {}), /Store.atomic/);
});

test('machine authorization rechecks administrator demotion and does not treat network failure as denial', async t => {
  const f = await setup(t), entry = f.enqueue();
  assert.equal((await f.authorize(entry)).allowed, true);
  f.users.get('@admin:example.test').admin = false;
  // Use an independent server service account for lookup after human demotion.
  f.commands.adminToken = 'fixture-authority';
  const user = f.palpo.user.bind(f.palpo);
  f.palpo.user = async mxid => structuredClone(f.users.get(mxid));
  assert.equal((await f.authorize(entry)).allowed, false);
  f.palpo.user = async () => { throw new Error('Matrix unavailable'); };
  await assert.rejects(f.authorize(entry), e => e.code === 'workflow_authority_unavailable' && e.status === 503);
  assert.equal(f.commands.entry(f.fleet, 'reserve').state, 'queued');
  f.palpo.user = user;
});

test('only explicit project administrators authorize top-ups; pending reassignment fences old commands', async t => {
  const f = await setup(t), reserve = f.enqueue(); f.accept(f.receipt(reserve));
  const op = { grantId: f.grant.id, grantRevision: 1, engagementId: `en_${'a'.repeat(32)}`, requesterMxid: f.project.ownerMxid, addTokens: 50 };
  const assigned = f.enqueue('top_up_agent', op, '@other:example.test', 'topup_assigned');
  assert.equal((await f.authorize(assigned)).allowed, true);
  const serverAdmin = f.enqueue('top_up_agent', op, '@admin:example.test', 'topup_server_admin');
  assert.equal((await f.authorize(serverAdmin)).allowed, false);
  f.commands.state.grants[f.grant.id].desiredRevision = 2;
  f.commands.state.grants[f.grant.id].desiredAdministrators = ['@owner:example.test'];
  assert.equal((await f.authorize(assigned)).allowed, false);
});

test('receipts are immutable and cannot claim a different project or capacity; failed update rolls back sequence', async t => {
  const f = await setup(t), entry = f.enqueue(), receipt = f.receipt(entry);
  const wrong = structuredClone(receipt); wrong.outcome.result.grant.ownerMxid = '@other:example.test';
  assert.throws(() => f.commands.validateReceipts(f.fleet, [wrong]), e => e.code === 'command_result_mismatch');
  const foreign = { ...receipt, fleetId: `hf_${'b'.repeat(32)}` };
  assert.throws(() => f.commands.validateReceipts(f.fleet, [foreign]), e => e.code === 'command_receipt_mismatch');
  assert.throws(() => f.commands.validateReceipts(f.fleet, [receipt, receipt]), e => e.code === 'command_receipt_conflict');
  f.accept(receipt); f.accept(receipt);
  assert.equal(f.commands.state.grants[f.grant.id].state, 'accepted');
  const changed = structuredClone(receipt); changed.completedAtMs++;
  assert.throws(() => f.commands.validateReceipts(f.fleet, [changed]), e => e.code === 'command_receipt_conflict');
  const sequence = f.fleet.transport.sequence;
  await assert.rejects(f.service.outbound.updates(f.fleet, { v: 2, generation: f.fleet.transport.generation, sequence: sequence + 1, heartbeat: true, commandReceipts: [wrong] }, f.workflow));
  assert.equal(f.fleet.transport.sequence, sequence);
  assert.equal(f.commands.entry(f.fleet, 'reserve').state, 'applied');
});

test('unknown commands, omitted limits and unadvertised peers fail before enqueue', async t => {
  const f = await setup(t), entry = f.enqueue();
  for (const altered of [
    { ...entry.command, operation: { kind: 'shell', command: 'arbitrary' } },
    { ...entry.command, admin: true },
    { ...entry.command, operation: { kind: 'reserve_project', grant: { ...f.grant, limits: {} } } },
  ]) assert.throws(() => validateProjectCommand(altered, f.fleet, 'example.test'), e => e.code === 'invalid_project_command');
  delete f.fleet.projectWorkflow;
  assert.throws(() => f.enqueue(), e => e.code === 'project_workflow_unavailable');
});

test('old assignment receipts replay after later changes without restoring old grant authority', async t => {
  const f = await setup(t), reserve = f.enqueue(); f.accept(f.receipt(reserve));
  const assign = f.enqueue('assign_project_admins', { grantId: f.grant.id, expectedRevision: 1, administrators: ['@admin:example.test'], allowSelfApproval: false }, '@admin:example.test', 'assign');
  const newerGrant = { ...f.grant, revision: 2, administratorMxids: ['@admin:example.test'] };
  const receipt = f.receipt(assign, { kind: 'grant', grant: newerGrant }); f.accept(receipt);
  const revoke = f.enqueue('revoke_project', { grantId: f.grant.id, expectedRevision: 2 }, '@admin:example.test', 'revoke');
  f.accept(f.receipt(revoke, { kind: 'revoked_project', grantId: f.grant.id, revision: 2 }));
  f.accept(receipt); f.accept(f.receipt(reserve));
  assert.equal(f.commands.state.grants[f.grant.id].state, 'revoked');
  assert.deepEqual(f.commands.state.grants[f.grant.id].grant, newerGrant);
});
