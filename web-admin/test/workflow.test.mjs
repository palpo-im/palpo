import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fixture, fleetInput } from './fixture.mjs';
import { Workflow } from '../lib/workflow.mjs';
const admin = ['@admin:example.test', 'admin-secret'], owner = ['@owner:example.test', 'owner-secret'];

async function setup(t, options) {
  const f = fixture(options); t.after(() => f.store.close());
  const workflow = new Workflow(f.service);
  const fleet = await f.service.create(fleetInput, ...admin);
  return { ...f, workflow, fleet };
}
async function project(f) {
  await f.workflow.connect(f.fleet.id, ...owner);
  return f.workflow.createProject({ fleetId: f.fleet.id, requestId: 'project-op-1', name: 'Coding project' }, ...owner);
}
const input = project => ({ projectId: project.id, requestId: 'request-role-1', role: 'coding', requestedTokens: 100000, ratePerDay: 20000 });

test('projects define multiple Agents on one resource with durable source-bound requests', async t => {
  const f = await setup(t), p = await project(f), resourceId = `resource_${'a'.repeat(24)}`;
  for (const name of ['fast-one', 'fast-two']) {
    const data = { ...input(p), requestId: name, agentName: name, resourceId };
    const first = await f.workflow.request(data, ...owner);
    assert.equal(first.state, 'pending');
    assert.deepEqual(first.agentDefinition, { name, resourceId });
    assert.equal(first.resource.reasoning, 'medium');
    const event = f.events.get(first.sourceEventId);
    assert.deepEqual(event.content.agentDefinition, first.agentDefinition);
    assert.equal(event.content.ownerDmRoomId, undefined);
    assert.equal((await f.workflow.request(data, ...owner)).sourceEventId, first.sourceEventId);
    await assert.rejects(f.workflow.request({ ...data, agentName: 'changed' }, ...owner), { code: 'idempotency_conflict' });
  }
  assert.equal(f.requests.size, 2);
  assert.equal(Object.keys(f.service.fleet(f.fleet.id).agents).length, 0);
  await assert.rejects(f.workflow.request({ ...input(p), requestId: 'duplicate', agentName: 'fast-one', resourceId }, ...owner), { code: 'agent_name_conflict' });
  await assert.rejects(f.workflow.request({ ...input(p), agentName: 'private', resourceId: `resource_${'b'.repeat(24)}` }, ...owner), { code: 'resource_unavailable' });
  await assert.rejects(f.workflow.request({ ...input(p), agentName: '../escape', resourceId }, ...owner), { code: 'invalid_agent_definition' });
  f.publishedOffers.set(f.fleet.id, []);
  assert.equal((await f.workflow.request({ ...input(p), requestId: 'fast-one', agentName: 'fast-one', resourceId }, ...owner)).state, 'pending');
});

test('Chinese Agent names survive source binding and canonical duplicate checks', async t => {
  const f = await setup(t), p = await project(f), resourceId = `resource_${'a'.repeat(24)}`;
  for (const [i, name] of ['小白', '孙悟空-01', 'Edison', 'Édison'].entries()) {
    const data = { ...input(p), requestId: `unicode-${i}`, agentName: name.normalize('NFD'), resourceId };
    const result = await f.workflow.request(data, ...owner);
    assert.equal(result.agentDefinition.name, name);
    assert.equal(f.events.get(result.sourceEventId).content.agentDefinition.name, name);
    assert.equal((await f.workflow.request({ ...data, agentName: name }, ...owner)).sourceEventId, result.sourceEventId);
    await assert.rejects(f.workflow.request({ ...data, requestId: `duplicate-${i}`, agentName: name }, ...owner), { code: 'agent_name_conflict' });
  }
  for (const [i, name] of ['', '../小白', '小白\n命令', '小\u202E白', '小白/二', '小'.repeat(65)].entries()) {
    await assert.rejects(f.workflow.request({ ...input(p), requestId: `invalid-${i}`, agentName: name, resourceId }, ...owner), { code: 'invalid_agent_definition' });
  }
});

test('definition acknowledgement must match and a retry preserves the original definition', async t => {
  const f = await setup(t), p = await project(f);
  const data = { ...input(p), agentDefinition: { name: 'retry-one', resourceId: `resource_${'a'.repeat(24)}` } };
  f.palpo.fetch = async (target, options) => {
    const response = await f.fetch(target, options);
    if (new URL(target).pathname === '/api/fleet/v1/requests') {
      const body = await response.json(); delete body.agentDefinition;
      return new Response(JSON.stringify(body), { status: 200 });
    }
    return response;
  };
  await assert.rejects(f.workflow.request(data, ...owner), { code: 'request_binding_conflict' });
  const stored = Object.values(f.store.state.requests)[0], eventId = stored.sourceEventId;
  assert.equal(stored.state, 'submission_pending');
  f.palpo.fetch = f.fetch;
  const result = await f.workflow.request(data, ...owner);
  assert.equal(result.state, 'pending'); assert.equal(result.sourceEventId, eventId);
  assert.deepEqual(result.agentDefinition, data.agentDefinition);
  assert.equal(f.requests.size, 1);
});

test('readiness requires actual pushed event receipt and exact reception memberships', async t => {
  const f = await setup(t);
  const connected = await f.workflow.connect(f.fleet.id, ...owner);
  assert.equal(connected.readiness.ready, true);
  assert.equal(connected.reception.roomId, connected.connection.sourceRoomId);
  assert.equal(f.events.get(connected.connection.sourceEventId).type, 'com.hafleet.connection.probe.v1');
  const count = f.rooms.size;
  await f.workflow.connect(f.fleet.id, ...owner);
  assert.equal(f.rooms.size, count);
  await assert.rejects(f.workflow.connect(f.fleet.id, '@other:example.test', 'other-secret'), { status: 404 });
});

test('unreceived Matrix probes remain pending and retry preserves the same event', async t => {
  const f = await setup(t, { deliverProbe: false });
  await assert.rejects(f.workflow.connect(f.fleet.id, ...owner), { code: 'probe_pending' });
  const first = { ...f.service.fleet(f.fleet.id).probe };
  assert.equal(f.service.fleet(f.fleet.id).state, 'pending_connection');
  await assert.rejects(f.workflow.connect(f.fleet.id, ...owner), { code: 'probe_pending' });
  assert.equal(f.service.fleet(f.fleet.id).probe.eventId, first.eventId);
  assert.equal(f.events.size, 1);
});

test('connect waits for delayed push receipt using one event and refuses unrelated errors', async t => {
  const f = await setup(t);
  let attempts = 0; const bodies = [];
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname === '/api/fleet/v1/probe') {
      attempts++; bodies.push(JSON.parse(options.body));
      if (attempts <= 2) return new Response(JSON.stringify({ code: 'probe_pending' }), { status: 409 });
    }
    return f.fetch(target, options);
  };
  const result = await f.workflow.connect(f.fleet.id, ...owner);
  assert.equal(result.readiness.ready, true);
  assert.equal(attempts, 3); assert.equal(f.events.size, 1);
  assert.ok(bodies.every(body => JSON.stringify(body) === JSON.stringify(bodies[0])));
  assert.equal(f.requests.size, 0);
  attempts = 0;
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname === '/api/fleet/v1/probe') {
      attempts++; return new Response(JSON.stringify({ code: 'unauthorized' }), { status: 403 });
    }
    return f.fetch(target, options);
  };
  await assert.rejects(f.workflow.connect(f.fleet.id, ...owner), { code: 'unauthorized' });
  assert.equal(attempts, 1); assert.equal(f.service.fleet(f.fleet.id).state, 'pending_connection');
});

test('a fleet paused during receipt verification stays paused', async t => {
  const f = await setup(t);
  f.palpo.fetch = async (target, options) => {
    const response = await f.fetch(target, options);
    if (new URL(target).pathname === '/api/fleet/v1/probe') f.service.fleet(f.fleet.id).state = 'paused';
    return response;
  };
  await assert.rejects(f.workflow.connect(f.fleet.id, ...owner), { code: 'fleet_inactive' });
  assert.equal(f.service.fleet(f.fleet.id).state, 'paused');
  assert.equal(f.service.fleet(f.fleet.id).connection, undefined);
});

test('catalog shares concurrent refreshes, bounds callback concurrency and retains actor-specific ownership', async t => {
  const f = await setup(t);
  for (let index = 0; index < 4; index++) await f.service.create({ ...fleetInput, requestId: `parallel-${index}` }, ...admin);
  let active = 0, maxActive = 0, count = 0;
  const releases = [];
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname === '/api/fleet/v1/capabilities') {
      active++; count++; maxActive = Math.max(maxActive, active);
      await new Promise(resolve => releases.push(resolve)); active--;
    }
    return f.fetch(target, options);
  };
  const owned = f.workflow.catalog(owner[0]), other = f.workflow.catalog('@other:example.test');
  // Allow asynchronous response parsing and each bounded batch to progress.
  for (let turn = 0; turn < 20 && count < 5; turn++) {
    await new Promise(resolve => setImmediate(resolve));
    while (releases.length) releases.shift()();
  }
  const [ownerRows, otherRows] = await Promise.all([owned, other]);
  assert.equal(count, 5); assert.equal(maxActive, 3);
  assert.ok(ownerRows.every(row => row.owned)); assert.ok(otherRows.every(row => !row.owned));
});

test('project creation is resumable, publishes scoped target binding and creates separate encrypted owner DM', async t => {
  const f = await setup(t), result = await project(f);
  assert.equal(result.state, 'registered'); assert.equal(result.canRequest, true);
  assert.notEqual(result.roomId, result.ownerDmRoomId);
  const target = f.rooms.get(result.roomId);
  assert.deepEqual(target.state.find(event => event.type === 'com.hafleet.admin.binding.v1' && event.state_key === f.fleet.id).content, { v: 1, fleetId: f.fleet.id, purpose: 'project', projectId: result.id, ownerMxid: owner[0], authVersion: 1 });
  assert.equal(f.rooms.get(result.ownerDmRoomId).state.find(event => event.type === 'm.room.encryption').content.algorithm, 'm.megolm.v1.aes-sha2');
  const count = f.rooms.size;
  const retried = await f.workflow.createProject({ fleetId: f.fleet.id, requestId: 'project-op-1', name: 'Coding project' }, ...owner);
  assert.equal(retried.roomId, result.roomId); assert.equal(f.rooms.size, count);
});

test('reordered Matrix binding keys recover an existing reception without creating another room', async t => {
  const f = await setup(t);
  const connected = await f.workflow.connect(f.fleet.id, ...owner);
  const roomId = connected.reception.roomId, fleet = f.service.fleet(f.fleet.id);
  // Simulate a lost local room acknowledgement and unavailable alias. The
  // joined-room state (sorted by Palpo) must recover the original room.
  delete fleet.reception.roomId;
  f.palpo.fetch = async (target, options) => new URL(target).pathname.includes('/directory/room/')
    ? new Response(JSON.stringify({ errcode: 'M_NOT_FOUND' }), { status: 404 }) : f.fetch(target, options);
  const recovered = await f.workflow.connect(f.fleet.id, ...owner);
  assert.equal(recovered.reception.roomId, roomId);
  assert.equal(f.calls.filter(call => call.path === '/_matrix/client/v3/createRoom').length, 1);
  const binding = f.rooms.get(roomId).state.find(event => event.type === 'com.hafleet.admin.binding.v1');
  binding.content.purpose = 'another_operation';
  await assert.rejects(f.workflow.connect(f.fleet.id, ...owner), { code: 'room_binding_conflict' });
  assert.equal(f.rooms.size, 1);
});

test('canonical room bindings preserve existing project IDs and idempotency fingerprints', async t => {
  const f = await setup(t), result = await project(f);
  const legacyDigest = value => createHash('sha256').update(JSON.stringify(value)).digest('hex');
  assert.equal(result.id, `project_${legacyDigest({ actor: owner[0], requestId: 'project-op-1' }).slice(0, 24)}`);
  const stored = f.store.state.projects[result.id];
  assert.equal(stored.fingerprint, legacyDigest({ actor: owner[0], fleetId: f.fleet.id, name: 'Coding project', existingRoom: null }));
  const requested = await f.workflow.request(input(result), ...owner);
  const request = f.store.state.requests[`${f.fleet.id}:request-role-1`];
  assert.equal(request.fingerprint, legacyDigest(request.payload));
  const retry = await f.workflow.request(input(result), ...owner);
  assert.equal(retry.sourceEventId, requested.sourceEventId);
  assert.equal(f.requests.size, 1);
});

test('an unjoined approval bot or unrelated invited member blocks requests', async t => {
  const f = await setup(t, { autoJoinBot: false }), result = await project(f);
  assert.equal(result.canRequest, false); assert.equal(result.readinessError, 'owner_dm_join_pending');
  await assert.rejects(f.workflow.request(input(result), ...owner), { code: 'owner_dm_join_pending' });
  const dm = f.rooms.get(result.ownerDmRoomId);
  f.putState(dm, 'm.room.member', '@approvalbot:example.test', { membership: 'join' }, '@approvalbot:example.test');
  f.putState(dm, 'm.room.member', '@other:example.test', { membership: 'invite' }, owner[0]);
  await assert.rejects(f.workflow.request(input(result), ...owner), { code: 'owner_dm_not_private' });
  assert.equal(f.requests.size, 0);
});

test('request binds real sender/source event to registered target and retries create one pending engagement', async t => {
  const f = await setup(t), result = await project(f);
  const requested = await f.workflow.request(input(result), ...owner);
  assert.equal(requested.state, 'pending');
  const event = f.events.get(requested.sourceEventId);
  assert.equal(event.sender, owner[0]); assert.equal(event.room_id, f.service.fleet(f.fleet.id).reception.roomId);
  assert.equal(event.content.targetRoomId, result.roomId); assert.notEqual(event.room_id, result.roomId);
  assert.ok(!JSON.stringify(event).includes(result.ownerDmRoomId));
  const retried = await f.workflow.request(input(result), ...owner);
  assert.equal(retried.sourceEventId, requested.sourceEventId); assert.equal(f.requests.size, 1);
  await assert.rejects(f.workflow.request({ ...input(result), requestedTokens: 99 }, ...owner), { code: 'idempotency_conflict' });
  assert.equal(f.requests.size, 1);
});

test('foreign member and revoked owner authority cannot target a registered project', async t => {
  const f = await setup(t), result = await project(f);
  await assert.rejects(f.workflow.request(input(result), '@other:example.test', 'other-secret'), { status: 403 });
  const room = f.rooms.get(result.roomId);
  f.putState(room, 'm.room.power_levels', '', { users: { [owner[0]]: 50 }, invite: 0 }, owner[0]);
  await assert.rejects(f.workflow.request(input(result), ...owner), { code: 'project_owner_authority_required' });
  assert.equal(f.requests.size, 0);
});

test('status links a real admitted agent only after verified ready fulfillment, preserving unknown runtime health', async t => {
  const f = await setup(t), result = await project(f);
  await f.workflow.request(input(result), ...owner);
  let requests = await f.workflow.requests(...owner);
  assert.equal(requests[0].usable, false); assert.equal(Object.keys(f.service.fleet(f.fleet.id).agents).length, 0);
  const fulfilled = f.fulfill(f.fleet.id, 'request-role-1');
  fulfilled.ready = false;
  requests = await f.workflow.requests(...owner);
  assert.equal(requests[0].usable, false); assert.equal(Object.keys(f.service.fleet(f.fleet.id).agents).length, 0);
  fulfilled.ready = true;
  fulfilled.serving.private_key = 'must-never-appear';
  requests = await f.workflow.requests(...owner);
  assert.equal(requests[0].usable, true); assert.equal(requests[0].agentJoined, true);
  assert.equal(requests[0].provider.serving.framework, 'codex');
  assert.ok(!JSON.stringify(requests).includes('must-never-appear'));
  const agents = Object.values(f.service.fleet(f.fleet.id).agents);
  assert.equal(agents.length, 1); assert.equal(agents[0].authorization, 'verified_hafleet_fulfillment');
  assert.equal(agents[0].mxid, fulfilled.agentMxid);
});

test('provider status cannot substitute another target or disclose arbitrary private fields', async t => {
  const f = await setup(t), result = await project(f);
  await f.workflow.request(input(result), ...owner);
  const record = f.requests.get(`${f.fleet.id}:request-role-1`);
  record.arbitrary_secret = 'must-not-appear';
  let observed = await f.workflow.requests(...owner);
  assert.ok(!JSON.stringify(observed).includes('must-not-appear'));
  record.targetRoomId = '!substituted:example.test';
  observed = await f.workflow.requests(...owner);
  assert.equal(observed[0].usable, false); assert.equal(observed[0].lastError.code, 'request_binding_conflict');
});
