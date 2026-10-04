import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './fixture.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { ProjectCommands } from '../lib/project-commands.mjs';
import { contributionKey } from '../lib/project-contributions.mjs';

async function setup(t) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const created = await f.service.create({ requestId: 'contribution-fleet', name: 'Contribution fleet', ownerMxid: '@owner:example.test' }, '@admin:example.test', 'admin-secret');
  const fleet = f.service.fleet(created.id), workflow = new Workflow(f.service);
  const commands = new ProjectCommands(f.service, {}, { now: () => 1000 });
  f.service.outbound.projectCommands = commands;
  const row = { grant: { v: 1, id: 'contribution_one', revision: 1, fleetId: fleet.id, registrationGeneration: 7, issuer: 'example.test',
    resourceId: `resource_${'a'.repeat(24)}`, limits: { tokens: 400, maxAgents: 4, maxRatePerDay: 100 }, expiresAtMs: 90000 },
    state: 'active', reserved: { tokens: 0, maxAgents: 0, maxRatePerDay: 0 } };
  const page = { v: 1, registrationGeneration: 7, observedAtMs: 1000, after: '', contributions: [row], nextAfter: null };
  const update = (sequence, contributionPage = page) => f.service.outbound.updates(fleet,
    { v: 2, generation: fleet.transport.generation, sequence, heartbeat: true, contributionPage }, workflow);
  t.after(() => f.store.close());
  return { ...f, fleet, commands, row, page, update, key: contributionKey(fleet.id, 7, row.grant.id) };
}

test('contributions commit with the update, retry once, and do not imply command support', async t => {
  const f = await setup(t);
  await f.update(1); await f.update(1);
  assert.equal(Object.keys(f.commands.state.contributions).length, 1);
  assert.deepEqual(f.commands.state.contributions[f.key].grant, f.row.grant);
  assert.equal(f.fleet.projectWorkflow, undefined);
  assert.throws(() => f.commands.requireSupport(f.fleet), e => e.code === 'project_workflow_unavailable');
  await f.update(2, { ...f.page, contributions: [] });
  assert.equal(Object.keys(f.commands.state.contributions).length, 1, 'an empty page is not withdrawal');
});

test('changed grants, foreign fleets, unknown fields, overdraw and bad cursors leave no update committed', async t => {
  const f = await setup(t);
  for (const change of [
    p => p.contributions[0].grant.fleetId = 'another_fleet',
    p => p.contributions[0].grant.issuer = 'other.test',
    p => p.contributions[0].grant.limits.tokens = 0,
    p => p.contributions[0].reserved.tokens = 401,
    p => p.contributions[0].grant.as_token = 'unwanted_secret',
    p => p.nextAfter = 'contribution_one',
    p => p.contributions.push(structuredClone(p.contributions[0])),
    p => p.observedAtMs = 6001,
    p => p.contributions[0].grant.registrationGeneration = 8,
  ]) {
    const page = structuredClone(f.page); change(page);
    await assert.rejects(f.update(1, page), e => e.code === 'invalid_contribution_page');
    assert.equal(f.fleet.transport.sequence, 0);
    assert.deepEqual(f.commands.state.contributions, {});
  }
  await f.update(1);
  const changed = structuredClone(f.page); changed.contributions[0].grant.limits.tokens++;
  await assert.rejects(f.update(2, changed), e => e.code === 'contribution_conflict');
  assert.equal(f.fleet.transport.sequence, 1);
});

test('retirement and reservations are monotonic; registration rotation preserves history', async t => {
  const f = await setup(t);
  f.row.reserved = { tokens: 100, maxAgents: 1, maxRatePerDay: 25 };
  await f.update(1);
  const refunded = structuredClone(f.page); refunded.contributions[0].reserved.tokens = 99;
  await assert.rejects(f.update(2, refunded), e => e.code === 'contribution_conflict');
  const revoked = structuredClone(f.page); revoked.contributions[0].state = 'revoked';
  await f.update(2, revoked);
  await assert.rejects(f.update(3), e => e.code === 'contribution_conflict');
  await f.update(3, { ...f.page, registrationGeneration: 8, contributions: [] });
  await assert.rejects(f.update(4), e => e.code === 'contribution_generation_conflict');
  assert.equal(f.commands.state.contributions[f.key].state, 'revoked');
  assert.equal(f.fleet.contributionPublication.registrationGeneration, 8);
});

test('a failed SQLite commit rolls back contributions and the sequence together', async t => {
  const f = await setup(t);
  const save = f.store.save.bind(f.store);
  f.store.save = () => { throw new Error('simulated disk failure'); };
  await assert.rejects(f.update(1), /disk failure/);
  assert.deepEqual(f.store.state.projectWorkflow.contributions, {});
  assert.equal(f.store.state.fleets[f.fleet.id].transport.sequence, 0);
  f.store.save = save;
});
