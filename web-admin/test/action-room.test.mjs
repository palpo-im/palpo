import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';

const actor = '@owner:example.test';
function setup(t) {
  const f = fixture();
  const app = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false, startActionWorker: false,
    actionConfig: { homeserverOrigin: 'https://matrix.example.test', botMxid: '@other:example.test', botToken: 'other-secret', adminToken: 'admin-secret', approvers: ['@admin:example.test'] } });
  t.after(() => f.store.close());
  const login = async (token = 'owner-secret', services = Object.keys(SERVICES)) => (await app.miniapp.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services })).sessionToken;
  const call = (session, service, args = {}) => app.miniapp.call(`Bearer ${session}`, { service, args });
  return { ...f, app, login, call };
}

test('room lookup is read-only; explicit setup joins only the current account and retry reuses the binding', async t => {
  const f = setup(t), owner = await f.login(), other = await f.login('admin-secret');
  const readOnly = await f.login('owner-secret', ['palpo.actions.room.get']);
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.get'), { room: null });
  assert.equal(f.rooms.size, 0);
  await assert.rejects(f.call(readOnly, 'palpo.actions.room.ensure'), e => e.status === 403);
  await assert.rejects(f.call(owner, 'palpo.actions.room.ensure', { actor: '@admin:example.test' }), e => e.status === 400);
  const room = await f.call(owner, 'palpo.actions.room.ensure');
  assert.deepEqual(room, { v: 1, purpose: 'my_actions', revision: 1, account: actor, roomId: '!room1:example.test', botMxid: '@other:example.test', serverName: 'example.test' });
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.ensure'), room);
  assert.equal(f.rooms.size, 1);
  const before = f.calls.filter(c => c.method !== 'GET').length;
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.get', { roomId: room.roomId }), { room });
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.get', { roomId: '!unrelated:example.test' }), { room: null });
  assert.deepEqual(await f.call(other, 'palpo.actions.room.get', { roomId: room.roomId }), { room: null });
  assert.equal(f.calls.filter(c => c.method !== 'GET').length, before);
});

test('every lookup revalidates membership, privacy, marker and bot; a name cannot authenticate a board', async t => {
  const f = setup(t), owner = await f.login(), binding = await f.call(owner, 'palpo.actions.room.ensure');
  const room = f.rooms.get(binding.roomId), saved = structuredClone(room.state);
  const corruptions = [
    ['m.room.member', actor, { membership: 'leave' }],
    ['m.room.member', actor, { membership: 'invite' }],
    ['m.room.member', '@other:example.test', { membership: 'leave' }],
    ['m.room.member', '@admin:example.test', { membership: 'invite' }],
    ['m.room.join_rules', '', { join_rule: 'public' }],
    ['m.room.encryption', '', { algorithm: 'm.megolm.v1.aes-sha2' }],
    ['im.palpo.actions.v1', '', { v: 1, purpose: 'my_actions', ownerMxid: '@admin:example.test' }],
  ];
  for (const [type, key, content] of corruptions) {
    room.state = structuredClone(saved); f.putState(room, type, key, content, '@other:example.test');
    await assert.rejects(f.call(owner, 'palpo.actions.room.get', { roomId: room.id }));
  }
  room.state = structuredClone(saved);
  f.putState(room, 'm.room.name', '', { name: 'Renamed by user' }, actor);
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.get'), { room: binding });
  f.app.inbox.state.rooms[actor].botMxid = '@admin:example.test';
  await assert.rejects(f.call(owner, 'palpo.actions.room.get'), e => e.code === 'action_room_not_private');
});

test('notification invitations cannot mount a board before explicit joining', async t => {
  const f = setup(t), owner = await f.login();
  const roomId = await f.app.actionNotifications.room(actor);
  await assert.rejects(f.call(owner, 'palpo.actions.room.get'), e => e.code === 'action_room_not_private');
  assert.equal((await f.call(owner, 'palpo.actions.room.ensure')).roomId, roomId);
  assert.equal(f.rooms.size, 1);
});

test('pending board pagination retains every action, prioritizes deadlines then age and isolates recipients', async t => {
  const f = setup(t), inbox = f.app.inbox;
  for (let i = 0; i < 123; i++) inbox.state.records[`action_${i}`] = {
    id: `action_${i}`, kind: 'project', ownerMxid: actor, state: 'requested', execution: 'pending', payload: { name: `Project ${i}` },
    revision: 1, createdAt: i, updatedAt: 1000 - i, ...(i === 100 ? { deadlineAt: 20 } : {}),
  };
  const pages = [0, 50, 100].map(offset => inbox.list('@admin:example.test', true, { offset, limit: 50 }));
  assert.equal(pages[0].pendingCount, 123); assert.equal(pages[0].actions[0].id, 'action_100');
  assert.equal(pages[0].actions[1].id, 'action_0');
  assert.equal(new Set(pages.flatMap(p => p.actions.map(r => r.id))).size, 123);
  assert.equal(inbox.list('@other:example.test', false).total, 0);
  assert.equal(inbox.list(actor, false, { view: 'waiting' }).total, 123);
});


test('leaving stops delivery until explicit setup replaces the room; lost setup replies reuse it', async t => {
  const f = setup(t), owner = await f.login(), old = await f.call(owner, 'palpo.actions.room.ensure');
  f.putState(f.rooms.get(old.roomId), 'm.room.member', actor, { membership: 'leave' }, actor);
  await assert.rejects(f.app.actionNotifications.room(actor));
  assert.equal(f.rooms.size, 1);
  const next = await f.call(owner, 'palpo.actions.room.ensure');
  assert.equal(next.revision, 2); assert.notEqual(next.roomId, old.roomId);
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.ensure'), next);
  assert.deepEqual(await f.call(owner, 'palpo.actions.room.get', { roomId: old.roomId }), { room: null });
  assert.equal(f.rooms.size, 2);
});

test('repair recovers an ambiguous create response without making a third room or losing canonical work', async t => {
  const f = setup(t), owner = await f.login(), old = await f.call(owner, 'palpo.actions.room.ensure');
  f.app.inbox.state.records.pending = { id: 'pending', kind: 'project', ownerMxid: actor, state: 'requested', execution: 'pending', payload: {}, revision: 1, createdAt: 1, updatedAt: 1 };
  f.putState(f.rooms.get(old.roomId), 'm.room.member', actor, { membership: 'leave' }, actor);
  const call = f.palpo.call.bind(f.palpo); let lost = false;
  f.palpo.call = async (...args) => {
    const result = await call(...args);
    if (!lost && args[0] === '/_matrix/client/v3/createRoom') { lost = true; throw new Error('Fixture dropped committed create response'); }
    return result;
  };
  await assert.rejects(f.call(owner, 'palpo.actions.room.ensure'));
  assert.equal(f.rooms.size, 2);
  const repaired = await f.call(owner, 'palpo.actions.room.ensure');
  assert.equal(repaired.revision, 2); assert.equal(f.rooms.size, 2);
  assert.equal(f.app.inbox.list('@admin:example.test', true).pendingCount, 1);
});

test('a room binding changed during an asynchronous read is refused', async t => {
  const f = setup(t), owner = await f.login(); await f.call(owner, 'palpo.actions.room.ensure');
  const call = f.palpo.call.bind(f.palpo);
  f.palpo.call = async (...args) => {
    const result = await call(...args);
    if (args[0].endsWith('/state')) f.app.inbox.state.rooms[actor] = { ...f.app.inbox.state.rooms[actor], revision: 2 };
    return result;
  };
  await assert.rejects(f.call(owner, 'palpo.actions.room.get'), e => e.code === 'action_room_not_private');
});
