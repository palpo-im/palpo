import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';
import { ActionNotifications } from '../lib/action-notifications.mjs';
import { isQuietAt } from '../lib/notification-preferences.mjs';
import { contributedFleet, projectBudget, projectResource } from './project-workflow-fixture.mjs';

const config = { homeserverOrigin: 'https://matrix.example.test', botMxid: '@other:example.test', botToken: 'other-secret', adminToken: 'admin-secret', approvers: ['@admin:example.test'] };
const preferences = { expectedRevision: 0, enabled: true, remindersEnabled: true, reminderMinutes: [60, 1440, 2880], quietHours: null };
function setup(t, { now = Date.now, path } = {}) {
  const f = fixture({ path, transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const app = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false, startActionWorker: false,
    inboxOptions: { now, approvers: ['@admin:example.test'], requireProjectApproval: true } });
  t.after(() => { try { f.store.close(); } catch {} });
  const inbox = app.inbox, workflow = inbox.workflow;
  const login = async (token = 'owner-secret', services = Object.keys(SERVICES)) => (await app.miniapp.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services })).sessionToken;
  const call = (session, service, args = {}) => app.miniapp.call(`Bearer ${session}`, { service, args });
  const submit = async () => {
    const fleet = await contributedFleet(f, workflow, inbox);
    return (await inbox.submit({ requestId: 'notification_project', kind: 'project', name: 'Private research project', reason: 'Private purpose', fleetId: fleet.id,
      resourceIds: [projectResource], allocations: projectBudget() }, '@owner:example.test', 'owner-secret')).action;
  };
  const messages = actor => [...f.events.values()].filter(e => e.content['im.palpo.action.v1']?.ownerMxid === actor);
  return { ...f, inbox, workflow, login, call, submit, messages, worker: () => new ActionNotifications(inbox, config, { now }) };
}

test('settings are current-account-only, explicitly granted, revision checked and durable across restart', async t => {
  const dir = mkdtempSync(join(tmpdir(), 'palpo-preferences-')); t.after(() => rmSync(dir, { recursive: true, force: true }));
  const path = join(dir, 'state.sqlite'), f = setup(t, { path });
  const owner = await f.login(), admin = await f.login('admin-secret'), readOnly = await f.login('owner-secret', ['palpo.notifications.get']);
  assert.equal((await f.call(owner, 'palpo.notifications.get')).revision, 0);
  await assert.rejects(f.call(readOnly, 'palpo.notifications.set', preferences), e => e.status === 403);
  await assert.rejects(f.call(owner, 'palpo.notifications.set', { ...preferences, actor: '@admin:example.test' }), e => e.status === 400);
  const update = { ...preferences, enabled: false, quietHours: { start: '22:00', end: '08:00', timeZone: 'America/Los_Angeles' } };
  const saved = await f.call(owner, 'palpo.notifications.set', update);
  assert.equal(saved.revision, 1); assert.equal(saved.enabled, false);
  assert.deepEqual(await f.call(owner, 'palpo.notifications.set', update), saved);
  await assert.rejects(f.call(owner, 'palpo.notifications.set', preferences), e => e.code === 'notification_preferences_changed');
  assert.equal((await f.call(admin, 'palpo.notifications.get')).enabled, true);
  f.store.close();
  const reopened = setup(t, { path });
  assert.deepEqual(await reopened.call(await reopened.login(), 'palpo.notifications.get'), saved);
});

test('malformed cadence and quiet hours cannot change saved preferences', async t => {
  const f = setup(t), owner = await f.login();
  for (const extra of [
    { enabled: 'false' }, { reminderMinutes: [1] }, { reminderMinutes: [60, 15] }, { reminderMinutes: [60, 60] },
    { reminderMinutes: [15, 30, 45, 60] }, { reminderMinutes: { map: 'bad' } },
    { quietHours: { start: '24:00', end: '08:00', timeZone: 'UTC' } },
    { quietHours: { start: '22:00', end: '22:00', timeZone: 'UTC' } },
    { quietHours: { start: '22:00', end: '08:00', timeZone: 'Invalid/Zone' } },
    { quietHours: { start: '22:00', end: '08:00', timeZone: 'UTC', enabled: true } },
  ]) await assert.rejects(f.call(owner, 'palpo.notifications.set', { ...preferences, ...extra }), e => e.status === 400);
  assert.equal((await f.call(owner, 'palpo.notifications.get')).revision, 0);
});

test('quiet hours follow local clock boundaries including spring gaps and repeated fall hours', () => {
  const quiet = { start: '22:00', end: '08:00', timeZone: 'America/Los_Angeles' };
  assert.equal(isQuietAt(quiet, Date.parse('2026-03-08T09:59:00Z')), true);
  assert.equal(isQuietAt(quiet, Date.parse('2026-03-08T10:01:00Z')), true);
  assert.equal(isQuietAt(quiet, Date.parse('2026-03-08T15:00:00Z')), false);
  assert.equal(isQuietAt(quiet, Date.parse('2026-11-01T08:30:00Z')), true);
  assert.equal(isQuietAt(quiet, Date.parse('2026-11-01T09:30:00Z')), true);
  assert.equal(isQuietAt(quiet, Date.parse('2026-11-01T16:00:00Z')), false);
  assert.equal(isQuietAt({ ...quiet, start: '09:00', end: '17:00' }, Date.parse('2026-11-01T20:00:00Z')), true);
});

test('disabled notifications keep canonical work and do not starve another recipient; enabling respects quiet hours', async t => {
  let now = Date.parse('2026-10-04T05:00:00Z');
  const f = setup(t, { now: () => now }), owner = await f.login(), admin = await f.login('admin-secret');
  const action = await f.submit();
  await f.call(admin, 'palpo.notifications.set', { ...preferences, enabled: false });
  const worker = f.worker(); await worker.tick();
  assert.equal(f.messages('@admin:example.test').length, 0);
  assert.equal(f.messages('@owner:example.test').length, 1);
  assert.equal(f.inbox.list('@admin:example.test', true).pendingCount, 1);
  await f.call(admin, 'palpo.notifications.set', { ...preferences, expectedRevision: 1, quietHours: { start: '22:00', end: '08:00', timeZone: 'UTC' } });
  await worker.tick(); assert.equal(f.messages('@admin:example.test').length, 0);
  now = Date.parse('2026-10-04T08:00:00Z'); await worker.tick();
  const message = f.messages('@admin:example.test')[0];
  assert.equal(message.content.msgtype, 'm.text'); assert.deepEqual(message.content['m.mentions'], { user_ids: ['@admin:example.test'] });
  assert.equal(f.messages('@owner:example.test')[0].content.msgtype, 'm.notice');
  assert.equal(f.inbox.get(action.id, '@owner:example.test', false).action.state, 'requested');
  assert.doesNotMatch(JSON.stringify(message.content), /Private research|Private purpose|as_token|hs_token/);
  assert.equal((await f.call(owner, 'palpo.notifications.get')).revision, 0);
});

test('read actions are reminded, downtime is coalesced, snooze defers and disabling reminders preserves the Inbox', async t => {
  let now = Date.parse('2026-10-04T10:00:00Z');
  const f = setup(t, { now: () => now }), action = await f.submit(), admin = await f.login('admin-secret'), worker = f.worker();
  await worker.tick(); const count = () => f.messages('@admin:example.test').length;
  assert.equal(count(), 1);
  await f.call(admin, 'palpo.inbox.seen', { id: action.id });
  now += 3600000; await worker.tick(); assert.equal(count(), 2);
  await f.call(admin, 'palpo.inbox.snooze', { id: action.id, minutes: 60 });
  now += 3599000; await worker.tick(); assert.equal(count(), 2);
  now += 1000; await worker.tick(); assert.equal(count(), 3);
  now += 4 * 86400000; await worker.tick(); assert.equal(count(), 4);
  now += 60000; await worker.tick(); assert.equal(count(), 4, 'overdue reminders do not burst');
  assert.equal(f.inbox.get(action.id, '@admin:example.test', true).action.reminderStatus.overdue, true);
  assert.equal(f.inbox.list('@admin:example.test', true).pendingCount, 1);
  await f.call(admin, 'palpo.inbox.snooze', { id: action.id, minutes: 60 });
  await f.call(admin, 'palpo.notifications.set', { ...preferences, remindersEnabled: false });
  now += 3600000; await worker.tick(); assert.equal(count(), 4);
  assert.equal(f.inbox.list('@admin:example.test', true).pendingCount, 1);
});

test('lost Matrix response replays a frozen transaction after quiet hours and worker recreation', async t => {
  let now = Date.parse('2026-10-04T21:59:00Z');
  const f = setup(t, { now: () => now }), action = await f.submit(), admin = await f.login('admin-secret');
  const fetch = f.palpo.fetch; let failOnce = true;
  f.palpo.fetch = async (url, options) => {
    const result = await fetch(url, options);
    if (failOnce && options.body?.includes('"ownerMxid":"@admin:example.test"') && new URL(url).pathname.includes('/send/m.room.message/')) { failOnce = false; throw new Error('lost response'); }
    return result;
  };
  await f.worker().tick();
  const notice = Object.values(f.inbox.state.notices).find(n => n.recipient === '@admin:example.test');
  const envelope = structuredClone(notice.delivery); assert.ok(envelope); assert.equal(notice.delivered, 0);
  await f.call(admin, 'palpo.notifications.set', { ...preferences, quietHours: { start: '22:00', end: '08:00', timeZone: 'UTC' } });
  now += 120000; await f.worker().tick(); assert.deepEqual(notice.delivery, envelope);
  now = Date.parse('2026-10-05T08:00:00Z'); await f.worker().tick();
  assert.equal(f.messages('@admin:example.test').length, 1);
  const sends = f.calls.filter(c => c.path.endsWith('/' + envelope.transactionId));
  assert.equal(sends.length, 2); assert.deepEqual(sends[0].body, sends[1].body);
  assert.equal(notice.delivered, 1); assert.equal(notice.reminderCursor, 1);
  assert.equal(f.inbox.get(action.id, '@admin:example.test', true).action.needsMyAction, true);
});

test('quiet hours are rechecked after asynchronous room verification and toggling settings cannot restart an exhausted cadence', async t => {
  let now = Date.parse('2026-10-04T21:59:00Z');
  const f = setup(t, { now: () => now }), action = await f.submit(), admin = await f.login('admin-secret');
  const quietHours = { start: '22:00', end: '08:00', timeZone: 'UTC' };
  await f.call(admin, 'palpo.notifications.set', { ...preferences, quietHours });
  const worker = f.worker(), room = worker.room.bind(worker);
  worker.room = async actor => {
    const id = await room(actor);
    if (actor === '@admin:example.test') now = Date.parse('2026-10-04T22:00:00Z');
    return id;
  };
  await worker.tick(); assert.equal(f.messages('@admin:example.test').length, 0);
  now += 4 * 86400000 + 10 * 3600000;
  await f.worker().tick(); assert.equal(f.messages('@admin:example.test').length, 1);
  const notice = Object.values(f.inbox.state.notices).find(n => n.recipient === '@admin:example.test');
  assert.equal(notice.finished, true); assert.equal(notice.reminderCursor, 3);
  await f.call(admin, 'palpo.notifications.set', { ...preferences, expectedRevision: 1, enabled: false, quietHours });
  await f.call(admin, 'palpo.notifications.set', { ...preferences, expectedRevision: 2, quietHours });
  await f.worker().tick(); assert.equal(f.messages('@admin:example.test').length, 1);
  assert.equal(f.inbox.get(action.id, '@admin:example.test', true).action.reminderStatus.overdue, true);
});
