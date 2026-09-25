import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, statSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fixture, fleetInput, agentInput } from './fixture.mjs';
import { Store } from '../lib/store.mjs';
import { Palpo, Service, publicFleet } from '../lib/service.mjs';

const admin = ['@admin:example.test', 'admin-secret'];
test('installs and verifies actual identity; retries reuse IDs and omit credentials from public reads', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  assert.equal(fleet.installation, 'installed'); assert.equal(fleet.state, 'pending_connection');
  assert.equal(fleet.readiness.ready, false); assert.equal(fleet.readiness.eventDelivery, 'unverified');
  assert.equal(f.users.get(fleet.representativeMxid).appservice_id, fleet.id);
  const retried = await f.service.create(fleetInput, ...admin);
  assert.equal(retried.id, fleet.id); assert.equal(f.registrations.size, 1);
  assert.equal(f.calls.filter(c => c.path === '/_palpo/admin/v1/appservices' && c.method === 'POST').length, 1);
  const raw = JSON.stringify(fleet);
  for (const value of Object.values(f.registrations.get(fleet.id)).filter(v => typeof v === 'string')) {
    if ([f.registrations.get(fleet.id).as_token, f.registrations.get(fleet.id).hs_token].includes(value)) assert.ok(!raw.includes(value));
  }
  assert.ok(!raw.includes('as_token')); assert.ok(!raw.includes('hs_token'));
  await assert.rejects(f.service.create({ ...fleetInput, name: 'Changed' }, ...admin), { code: 'idempotency_conflict' });
});

test('callback policy and active local owner are enforced before registration', async t => {
  const f = fixture(); t.after(() => f.store.close());
  for (const callbackUrl of ['http://127.0.0.1:8008', 'https://fleet.example.test@evil.test', 'https://fleet.example.test/matrix?token=secret', 'file:///etc/passwd']) {
    await assert.rejects(f.service.create({ ...fleetInput, callbackUrl }, ...admin));
  }
  await assert.rejects(f.service.create({ ...fleetInput, ownerMxid: '@owner:elsewhere.test' }, ...admin), { code: 'invalid_owner' });
  f.users.get('@owner:example.test').deactivated = true;
  await assert.rejects(f.service.create(fleetInput, ...admin), { code: 'invalid_owner' });
  assert.equal(f.registrations.size, 0);
});

test('two fleets use disjoint namespaces and owner pairing cannot cross ownership', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const a = await f.service.create(fleetInput, ...admin);
  const b = await f.service.create({ ...fleetInput, requestId: 'request-two', ownerMxid: '@other:example.test' }, ...admin);
  const regex = new RegExp(a.registration.namespaces.users[0].regex);
  assert.equal(regex.test(a.representativeMxid), true); assert.equal(regex.test(b.representativeMxid), false);
  await assert.rejects(f.service.credentials(b.id, '@owner:example.test'), { status: 404 });
  const credentials = await f.service.credentials(a.id, '@owner:example.test');
  assert.equal(credentials.registration.as_token, f.registrations.get(a.id).as_token);
  assert.deepEqual(await f.service.credentials(a.id, '@owner:example.test'), credentials);
  assert.equal(publicFleet(f.service.fleet(b.id)).credentialDeliveredAt, null);
});

test('pairing verifies the currently effective service token before marking credentials delivered', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  f.registrations.get(fleet.id).disabled = true;
  await assert.rejects(f.service.credentials(fleet.id, '@owner:example.test'), { status: 401 });
  assert.equal(f.service.fleet(fleet.id).credentialDeliveredAt, null);
});

test('the durable admin database cannot silently move to a different Matrix server', t => {
  const f = fixture(); t.after(() => f.store.close());
  assert.throws(() => new Service({ store: f.store, palpo: f.palpo, serverName: 'another.test', callbackOrigins: [] }), /different Palpo server/);
  assert.throws(() => new Service({ store: f.store, palpo: new Palpo('https://elsewhere.test'), serverName: 'example.test', callbackOrigins: [] }), /different Palpo server/);
});

test('unknown broad namespaces fail closed and record an actionable failed installation', async t => {
  const f = fixture(); t.after(() => f.store.close());
  f.registrations.set('legacy', { id: 'legacy', namespaces: { users: [{ exclusive: true, regex: '.*' }] } });
  await assert.rejects(f.service.create(fleetInput, ...admin), { code: 'namespace_policy' });
  const fleet = Object.values(f.store.state.fleets)[0];
  assert.equal(fleet.installation, 'failed'); assert.equal(fleet.lastError.code, 'namespace_policy');
  f.registrations.delete('legacy');
  assert.equal((await f.service.create(fleetInput, ...admin)).id, fleet.id);
});

test('installed credentials drifting never get replaced or reported as verified', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  const prior = f.registrations.get(fleet.id).as_token;
  f.registrations.get(fleet.id).as_token = 'changed-on-server';
  await assert.rejects(f.service.install(fleet.id, ...admin), { code: 'registration_drift' });
  assert.equal(f.service.fleet(fleet.id).registration.as_token, prior);
  assert.equal(f.service.fleet(fleet.id).installation, 'failed');
});

test('agent CRUD observes actual Matrix ownership, profile and deactivation without claiming runtime stop', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  const agent = await f.service.createAgent(fleet.id, agentInput, ...admin);
  assert.equal(agent.matrixIdentity, 'active'); assert.equal(agent.runtimeHealth, 'unknown');
  assert.equal(f.users.get(agent.mxid).appservice_id, fleet.id);
  const retry = await f.service.createAgent(fleet.id, agentInput, ...admin);
  assert.equal(agent.mxid, retry.mxid);
  await assert.rejects(f.service.updateAgent(fleet.id, agent.id, { displayName: 'New', mxid: '@human:example.test' }, ...admin), { code: 'immutable_identity' });
  const edited = await f.service.updateAgent(fleet.id, agent.id, { displayName: 'New name' }, ...admin);
  assert.equal(edited.displayName, 'New name'); assert.equal(edited.mxid, agent.mxid);
  f.users.get(agent.mxid).rooms = ['!project:example.test'];
  const retired = await f.service.retireAgent(fleet.id, agent.id, ...admin);
  assert.equal(retired.state, 'retired'); assert.equal(retired.matrixIdentity, 'deactivated');
  assert.deepEqual(retired.joinedRooms, []); assert.equal(retired.localTaskStop, 'unconfirmed');
  await assert.rejects(f.service.createAgent(fleet.id, agentInput, ...admin), { code: 'agent_retired' });
});

test('existing foreign identity is not adopted or updated', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  const mxid = `@${fleet.id}_agent_coding_01:example.test`;
  f.users.set(mxid, { name: mxid, displayname: 'Foreign', appservice_id: 'another-fleet' });
  await assert.rejects(f.service.createAgent(fleet.id, agentInput, ...admin), { code: 'identity_conflict' });
  assert.equal(f.users.get(mxid).displayname, 'Foreign');
  assert.equal(f.service.fleet(fleet.id).agents.coding_01.state, 'failed');
});

test('retirement cannot pass while an App Service can still authenticate as the agent', async t => {
  const f = fixture({ bypassRetirement: true }); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  const agent = await f.service.createAgent(fleet.id, agentInput, ...admin);
  await assert.rejects(f.service.retireAgent(fleet.id, agent.id, ...admin), { code: 'retirement_unverified' });
  assert.equal(f.service.fleet(fleet.id).agents[agent.id].state, 'retiring');
});

test('pause and revoke disable the actual registration and prohibit identity changes', async t => {
  const f = fixture(); t.after(() => f.store.close());
  const fleet = await f.service.create(fleetInput, ...admin);
  await f.service.setState(fleet.id, 'pause', ...admin);
  assert.equal(f.registrations.get(fleet.id).disabled, true);
  await assert.rejects(f.service.createAgent(fleet.id, agentInput, ...admin), { code: 'fleet_inactive' });
  await f.service.setState(fleet.id, 'resume', ...admin);
  assert.equal(f.registrations.get(fleet.id).disabled, false);
  const revoked = await f.service.setState(fleet.id, 'revoke', ...admin);
  assert.equal(revoked.state, 'revoked'); assert.equal(revoked.localTaskStop, 'unconfirmed');
  await assert.rejects(f.service.credentials(fleet.id, '@owner:example.test'), { code: 'fleet_inactive' });
  await assert.rejects(f.service.setState(fleet.id, 'resume', ...admin), { code: 'fleet_revoked' });
});

test('restart preserves registration plans, audit and resumable credential delivery; database is private', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'palpo-admin-')), path = join(dir, 'admin.sqlite');
  const f = fixture({ path });
  try {
    const fleet = await f.service.create(fleetInput, ...admin);
    await f.service.credentials(fleet.id, '@owner:example.test');
    f.store.close();
    const store = new Store(path);
    try {
      const service = new Service({ store, palpo: f.palpo, serverName: 'example.test', callbackOrigins: ['https://fleet.example.test'] });
      assert.equal((await service.create(fleetInput, ...admin)).id, fleet.id);
      assert.ok(store.state.audit.length >= 3);
      assert.equal((await service.credentials(fleet.id, '@owner:example.test')).registration.as_token, f.registrations.get(fleet.id).as_token);
      assert.equal(statSync(path).mode & 0o777, 0o600);
    } finally { store.close(); }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('Palpo transport refuses redirects and strips secret-bearing upstream errors', async () => {
  const palpo = new Palpo('https://palpo.example.test', async (_, options) => {
    assert.equal(options.redirect, 'error');
    return new Response(JSON.stringify({ errcode: 'M_FORBIDDEN', error: 'sensitive-secret-token' }), { status: 403 });
  });
  await assert.rejects(palpo.call('/_palpo/admin/v1/appservices', 'token'), error => !error.message.includes('sensitive-secret-token') && error.status === 403);
});
