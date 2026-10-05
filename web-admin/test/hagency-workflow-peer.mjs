// Explicit Matrix fixture for the cross-repository Rust worker test. Palpo's
// HTTP, sessions, command queue, workflow and SQLite are the production modules.
// No Hagency contribution or business receipt is synthesized here.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { fixture, fleetInput } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';

const directory = resolve(process.argv[2]);
const controlToken = randomBytes(32).toString('hex');
const portProbe = createServer(); portProbe.listen(0, '127.0.0.1'); await once(portProbe, 'listening');
const port = portProbe.address().port; await new Promise(resolve => portProbe.close(resolve));
const endpoint = `http://127.0.0.1:${port}`;
const f = fixture({ path: `${directory}/palpo.sqlite3`, transportOrigin: 'http://127.0.0.1', relayOrigin: 'http://127.0.0.1' });
const server = createApp({ service: f.service, publicOrigin: endpoint, startAccountWorker: false,
  startActionWorker: false, inboxOptions: { projectApprover: '@admin:example.test', requireProjectApproval: true } });
server.projectCommands.adminToken = 'admin-secret';
let dropUpdate = false;
const updates = [];
// Lose the response only after the real handler commits an accepted update.
const handle = server.listeners('request')[0]; server.removeAllListeners('request');
server.on('request', (req, res) => {
  if (req.url.endsWith('/updates')) {
    const chunks = []; req.on('data', b => chunks.push(b));
    const end = res.end.bind(res);
    res.end = (...args) => {
      updates.push({ status: res.statusCode, digest: createHash('sha256').update(Buffer.concat(chunks)).digest('hex') });
      if (dropUpdate && res.statusCode === 200) { dropUpdate = false; res.destroy(); return res; }
      return end(...args);
    };
  }
  handle(req, res);
});
server.listen(port, '127.0.0.1'); await once(server, 'listening');
f.service.transportOrigin = endpoint;
const created = await f.service.create(fleetInput, '@admin:example.test', 'admin-secret');
await server.inbox.workflow.connect(created.id, '@owner:example.test', 'owner-secret');
const fleet = f.service.fleet(created.id);
fleet.transport = f.service.outbound.transport(fleet.id);
// The connection is a Matrix fixture; actual connection acceptance is separate.
fleet.connection.generation = fleet.transport.generation;
f.store.save();

const send = (res, status, value) => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify(value)); };
const peer = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    const chunks = []; let length = 0;
    for await (const chunk of req) { length += chunk.length; assert.ok(length <= 65536); chunks.push(chunk); }
    const body = length ? JSON.parse(Buffer.concat(chunks)) : {};
    if (url.pathname.startsWith('/fixture/')) {
      if (req.headers.authorization !== `Bearer ${controlToken}`) return send(res, 401, {});
      switch (url.pathname) {
        case '/fixture/support':
          // Explicit opt-in for this test only. Production publication still
          // withholds this capability pending the full ADR acceptance gates.
          fleet.projectWorkflow = { v: 1, registrationGeneration: 1, unusedRelease: true, transportGeneration: fleet.transport.generation };
          f.store.save(); return send(res, 200, {});
        case '/fixture/lock-admin':
          f.users.get('@admin:example.test').locked = body.locked === true; return send(res, 200, {});
        case '/fixture/drop-update': dropUpdate = true; return send(res, 200, {});
        case '/fixture/redeliver': {
          const entry = Object.values(server.projectCommands.state.commands).find(e => e.command.commandId === body.commandId); assert.ok(entry);
          f.service.outbound.enqueue(fleet, 'work', 'workflow', `replay_${body.commandId}`, entry.command);
          return send(res, 200, {});
        }
        case '/fixture/state': return send(res, 200, {
          sequence: fleet.transport.sequence, updates,
          commands: Object.values(server.projectCommands.state.commands).map(e => ({ command: e.command, receipt: e.receipt ?? null })),
          actions: Object.values(server.inbox.state.records).map(r => ({ id: r.id, kind: r.kind, state: r.state, execution: r.execution, revision: r.revision, result: r.result ?? null })),
          notices: Object.keys(server.inbox.state.notices).length,
          requests: Object.values(f.store.state.requests).map(r => ({ id: r.id, state: r.state, usable: r.usable ?? false,
            allocation: server.inbox.agents.allocation(r), lifecycle: r.provider?.lifecycle ?? null })),
          legacyDeliveries: f.store.db.prepare("SELECT count(*) n FROM fleet_delivery WHERE kind='request'").get().n,
          pendingDeliveries: f.service.outbound.usage(fleet).pending,
          contributions: Object.values(server.projectCommands.state.contributions).map(r => ({ reserved: r.reserved, released: r.released })),
        });
        default: return send(res, 404, {});
      }
    }
    // Serve the existing Matrix fixture through real HTTP for Hagency's Reader.
    // Authenticate and check membership using its full state route first.
    const parts = url.pathname.split('/').map(decodeURIComponent);
    const headers = { Authorization: req.headers.authorization };
    if (parts[1] === '_matrix' && parts[4] === 'rooms' && (parts[6] === 'joined_members' || parts[6] === 'state')) {
      const stateUrl = new URL(url); stateUrl.pathname = `/_matrix/client/v3/rooms/${encodeURIComponent(parts[5])}/state`;
      const answer = await f.fetch(stateUrl.href, { method: 'GET', headers });
      const state = await answer.json();
      if (answer.status !== 200) return send(res, answer.status, state);
      if (parts[6] === 'joined_members') return send(res, 200, { joined: Object.fromEntries(state.filter(e => e.type === 'm.room.member' && e.content.membership === 'join').map(e => [e.state_key, {}])) });
      const event = state.find(e => e.type === parts[7] && e.state_key === (parts[8] ?? ''));
      return send(res, event ? 200 : 404, event?.content ?? { errcode: 'M_NOT_FOUND' });
    }
    const answer = await f.fetch(url.href, { method: req.method, headers, ...(length ? { body: JSON.stringify(body) } : {}) });
    send(res, answer.status, await answer.json());
  } catch (error) { send(res, 500, { fixtureError: error.message }); }
});
peer.listen(0, '127.0.0.1'); await once(peer, 'listening');
const sessions = {};
for (const [role, token] of [['owner', 'owner-secret'], ['admin', 'admin-secret'], ['assigned', 'other-secret']]) {
  const reply = await fetch(`${endpoint}/_palpo/miniapp/v1/session`, { method: 'POST', headers: { Authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ appId: APP_ID, bundleDigest: 'a'.repeat(64), services: Object.keys(SERVICES) }) });
  const result = await reply.json(); assert.equal(reply.status, 200, result.code); sessions[role] = result.sessionToken;
}
console.log(JSON.stringify({ endpoint: `${endpoint}/api/fleet/v2/${fleet.id}`, miniapp: `${endpoint}/_palpo/miniapp/v1/call`,
  matrix: `http://127.0.0.1:${peer.address().port}`, controlToken, sessions,
  machineToken: fleet.transport.token, machineGeneration: fleet.transport.generation,
  appserviceToken: fleet.registration.as_token,
  registration: { fleetId: fleet.id, generation: 1, serverName: 'example.test', representativeMxid: fleet.representativeMxid,
    approvalBotMxid: fleet.capabilities.approvalBotMxid, receptionRoomId: fleet.reception.roomId } }));
let closing = false;
async function close() {
  if (closing) return; closing = true;
  server.closeAllConnections(); peer.closeAllConnections();
  await Promise.all([new Promise(resolve => server.close(resolve)), new Promise(resolve => peer.close(resolve))]);
  f.store.close();
}
process.on('SIGTERM', () => close().then(() => process.exit(0)));
