import test from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer as reserveServer } from 'node:net';
import { mkdtempSync, rmSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createApp } from '../server.mjs';
import { Accounts } from '../lib/accounts.mjs';
import { Store } from '../lib/store.mjs';
import { accountFixture, applicant } from './accounts.fixture.mjs';

async function prepared(t, options) {
  const f = accountFixture(options); t.after(async () => { await f.accounts.stop(); f.store.close(); });
  await f.accounts.tick(); assert.equal(f.accounts.ready, true, f.accounts.lastError); f.joinAdmin();
  return f;
}
async function pending(f, input = applicant()) {
  f.accounts.submit(input); await f.accounts.tick();
  const row = f.accounts.state.requests[input.id]; assert.equal(row.status, 'pending'); return row;
}

test('request waits for real administrator verdict, then creates an ordinary account and erases password', async t => {
  const f = await prepared(t), input = applicant(), row = await pending(f, input);
  assert.equal(f.credentials.size, 0);
  const card = f.events.get(row.sourceEventId).content;
  assert.deepEqual(card['org.octos.actions'].map(a => a.label), ['Approve', 'Reject']);
  assert.equal(card['org.octos.approval_request'].tool_args_digest, row.digest);
  for (const serialized of [JSON.stringify(f.store.state), JSON.stringify(card), JSON.stringify(f.accounts.adminView()), JSON.stringify(f.accounts.status(input.id, input.receipt))]) {
    for (const secret of [input.password, input.receipt, f.config.passwordKey, f.config.registrationToken, f.config.adminToken]) assert.ok(!serialized.includes(secret));
  }
  const event = f.decision(row); f.events.set(event.event_id, event);
  await f.accounts.tick(); assert.equal(row.status, 'approved'); assert.equal(f.credentials.size, 0);
  await f.accounts.tick(); assert.equal(row.status, 'registered'); assert.equal(row.password, undefined);
  assert.equal(f.credentials.get(row.userId), input.password); assert.equal(f.users.get(row.userId).admin, false);
  assert.equal(f.accounts.status(input.id, input.receipt).status, 'registered');
  await f.accounts.decide(f.decision(row)); await f.accounts.tick();
  assert.equal(f.registrationCalls.length, 2); assert.equal(f.credentials.size, 1);
});

test('reject and expiry never register an account and erase pending password', async t => {
  let time = Date.now(); const f = await prepared(t, { accountOptions: { clock: () => time } });
  const rejected = await pending(f), event = f.decision(rejected); event.content['org.octos.approval_response'].decision = 'deny';
  await f.accounts.decide(event); assert.equal(rejected.status, 'rejected'); assert.equal(rejected.password, undefined);
  const expired = await pending(f, applicant('bob')); time = expired.expiresAt; await f.accounts.tick();
  assert.equal(expired.status, 'expired'); assert.equal(expired.password, undefined);
  await f.accounts.decide(f.decision(expired)); assert.equal(f.registrationCalls.length, 0);
});

test('forged, wrong-room, mismatched, nonadmin and revoked-admin decisions are refused', async t => {
  const f = await prepared(t), row = await pending(f);
  const mutate = [
    event => { event.sender = '@other:example.test'; },
    event => { event.room_id = '!other:example.test'; },
    event => { event.content['org.octos.approval_response'].tool_args_digest = 'bad'; },
    event => { event.content['org.octos.approval_response'].source_event_id = '$other'; },
    event => { event.content['m.relates_to']['m.in_reply_to'].event_id = '$other'; },
    event => { event.content = { msgtype: 'm.text', body: 'approve' }; },
    event => { event.content['org.octos.approval_response'].decision = 'allow'; },
  ];
  for (const change of mutate) { const event = f.decision(row); change(event); await f.accounts.decide(event); assert.equal(row.status, 'pending'); }
  f.users.get('@admin:example.test').admin = false;
  await f.accounts.decide(f.decision(row)); assert.equal(row.status, 'pending'); assert.equal(f.credentials.size, 0);
});

test('changed room privacy blocks approval and notification without losing pending request', async t => {
  const f = await prepared(t), row = await pending(f);
  f.putState(f.rooms.get(f.accounts.state.roomId), 'm.room.member', '@other:example.test', { membership: 'invite' }, '@admin:example.test');
  await assert.rejects(f.accounts.decide(f.decision(row)), { code: 'account_room_not_private' });
  await f.accounts.tick(); assert.equal(f.accounts.lastError, 'account_room_not_private'); assert.equal(row.status, 'pending');
  f.putState(f.rooms.get(f.accounts.state.roomId), 'm.room.member', '@other:example.test', { membership: 'leave' }, '@admin:example.test');
  await f.accounts.tick(); assert.equal(f.accounts.ready, true);
});

test('first-use room history starts at invitation and legacy hidden cards are reissued once', async t => {
  const f = await prepared(t), row = await pending(f);
  const room = f.rooms.get(f.accounts.state.roomId);
  assert.equal(room.state.find(e => e.type === 'm.room.history_visibility').content.history_visibility, 'invited');
  f.putState(room, 'm.room.history_visibility', '', { history_visibility: 'joined' }, f.config.botMxid);
  const oldId = row.sourceEventId, oldDecision = f.decision(row);
  const recovered = new Accounts(f.service, f.config); await recovered.tick();
  assert.notEqual(row.sourceEventId, oldId); assert.equal(row.status, 'pending');
  assert.deepEqual(row.supersededSourceEventIds, [oldId]);
  await recovered.decide(oldDecision); assert.equal(row.status, 'pending');
  const newId = row.sourceEventId;
  const again = new Accounts(f.service, f.config); await again.tick(); assert.equal(row.sourceEventId, newId);
  await again.decide(f.decision(row)); assert.equal(row.status, 'approved');
});

test('request receipt, field-bound retry, username reservation and existing-account conflicts are enforced', async t => {
  const f = await prepared(t), input = applicant(), row = await pending(f, input);
  assert.throws(() => f.accounts.status(input.id, 'b'.repeat(64)), { status: 404 });
  assert.equal(f.accounts.submit(input).id, input.id);
  assert.throws(() => f.accounts.submit({ ...input, password: 'another-long-password' }), { code: 'request_changed' });
  assert.throws(() => f.accounts.submit(applicant()), { code: 'username_pending' });
  await f.accounts.decide(f.decision(row));
  f.users.set(row.userId, { admin: false, deactivated: false, displayname: 'Existing owner' });
  await f.accounts.tick(); assert.equal(row.status, 'name_unavailable'); assert.equal(f.registrationCalls.length, 0);
  assert.equal(f.users.get(row.userId).displayname, 'Existing owner');
});

test('restart reuses encrypted request, decision and exact registration device after a lost response', async t => {
  let time = Date.now();
  const dir = mkdtempSync(join(tmpdir(), 'palpo-account-')); t.after(() => rmSync(dir, { recursive: true, force: true }));
  const path = join(dir, 'state.sqlite'), f = await prepared(t, { path, accountOptions: { clock: () => time } }), input = applicant(), row = await pending(f, input);
  await f.accounts.decide(f.decision(row)); f.loseRegistration(); await f.accounts.tick();
  assert.equal(row.status, 'registering'); assert.equal(f.credentials.size, 1);
  assert.ok(!readFileSync(path).includes(Buffer.from(input.password)));
  time += 3000;
  const recovered = new Accounts(f.service, f.config, { clock: () => time }); await recovered.tick();
  assert.equal(recovered.state.requests[input.id].status, 'registered'); assert.equal(f.registrationCalls.length, 2);
  assert.equal(recovered.state.requests[input.id].password, undefined);
  const reopened = new Store(path); t.after(() => reopened.close());
  assert.equal(reopened.state.accountAccess.requests[input.id].status, 'registered');
  assert.throws(() => new Accounts(f.service, { ...f.config, passwordKey: 'b'.repeat(64) }), /binding changed/);
});

test('public HTTP flow enforces origin, hides admin data and permits login only after approval', async t => {
  const f = accountFixture(); t.after(() => f.store.close());
  const reservation = reserveServer().listen(0, '127.0.0.1'); await once(reservation, 'listening');
  const port = reservation.address().port; await new Promise(resolve => reservation.close(resolve));
  const base = `http://127.0.0.1:${port}`;
  const server = createApp({ service: f.service, publicOrigin: base, accountConfig: f.config, startAccountWorker: false });
  server.listen(port, '127.0.0.1'); await once(server, 'listening'); t.after(() => new Promise(resolve => server.close(resolve)));
  await server.accounts.tick();
  f.putState(f.rooms.get(server.accounts.state.roomId), 'm.room.member', '@admin:example.test', { membership: 'join' }, '@admin:example.test');
  const input = applicant();
  const call = async (path, body, headers = {}) => { const response = await fetch(base + path, { method: body ? 'POST' : 'GET', headers: { Origin: base, 'Content-Type': 'application/json', ...headers }, ...(body ? { body: JSON.stringify(body) } : {}) }); return { status: response.status, data: await response.json(), cookie: response.headers.get('set-cookie') }; };
  assert.equal((await call('/api/account-requests', input, { Origin: 'https://evil.test' })).status, 403);
  assert.equal((await call('/api/account-requests')).status, 401);
  assert.equal((await call('/api/account-requests', input)).status, 202);
  assert.equal((await call('/api/login', { username: '@alice:example.test', password: input.password })).status, 403);
  await server.accounts.tick(); const row = server.accounts.state.requests[input.id];
  await server.accounts.decide(f.decision(row)); await server.accounts.tick();
  const login = await call('/api/login', { username: row.userId, password: input.password });
  assert.equal(login.status, 200); assert.equal(login.data.isAdmin, false);
  assert.equal((await call('/api/account-requests', null, { Cookie: login.cookie })).status, 403);
  assert.equal((await call('/api/account-requests/status', { id: input.id, receipt: input.receipt })).data.request.status, 'registered');
  assert.equal((await call('/api/account-requests/status', { id: input.id, receipt: 'a'.repeat(64) })).status, 404);
});

test('shutdown cancels a stalled registration while preserving its approved request for recovery', async t => {
  const f = await prepared(t), row = await pending(f); await f.accounts.decide(f.decision(row));
  const original = f.palpo.fetch; let entered;
  const started = new Promise(resolve => { entered = resolve; });
  f.palpo.fetch = (url, options) => {
    if (url.pathname !== '/_matrix/client/v3/register') return original(url, options);
    entered(); return new Promise((_resolve, reject) => {
      if (options.signal.aborted) reject(options.signal.reason);
      else options.signal.addEventListener('abort', () => reject(options.signal.reason), { once: true });
    });
  };
  const tick = f.accounts.tick(); await started;
  const time = Date.now(); await f.accounts.stop(); await tick;
  assert.ok(Date.now() - time < 1000); assert.equal(row.status, 'registering');
  assert.ok(row.password); assert.equal(f.credentials.size, 0);
});
