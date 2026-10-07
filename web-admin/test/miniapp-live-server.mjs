// Operator-invoked validation sidecar. Uses real Matrix authentication and an
// isolated workflow database. Optional operator-provisioned test identities let
// the native driver exercise real admin decisions and private Matrix notices.
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { Store } from '../lib/store.mjs';
import { Palpo, Service } from '../lib/service.mjs';
import { createApp } from '../server.mjs';

const directory = resolve(process.argv[2] ?? '');
const port = Number(process.argv[3]);
const matrix = new URL(process.argv[4]);
const serverName = process.argv[5];
const accountsFile = process.argv[6];
if (!process.argv[2] || !Number.isSafeInteger(port) || port < 1024 || port > 65535
  || matrix.hostname !== '127.0.0.1' || matrix.protocol !== 'http:' || !serverName) throw new Error('Explicit isolated directory, loopback port and Matrix upstream required.');
mkdirSync(directory, { recursive: true, mode: 0o700 });
const store = new Store(resolve(directory, 'validation.sqlite'));
const origin = `http://127.0.0.1:${port}`;
const accounts = accountsFile ? JSON.parse(readFileSync(accountsFile, 'utf8')).accounts : null;
if (accounts && ['owner', 'admin', 'bot'].some(role => !accounts[role]?.user_id?.startsWith(`@rinx_validation_${role}_`)
  || !accounts[role].user_id.endsWith(':' + serverName) || !accounts[role].access_token)) throw new Error('Only dedicated validation accounts are allowed.');
const service = new Service({ store, palpo: new Palpo(matrix.href), serverName,
  ...(accounts ? { transportOrigin: origin, relayOrigin: origin } : {}) });
const server = createApp({ service, publicOrigin: `http://127.0.0.1:${port}`,
  startAccountWorker: false, startActionWorker: false,
  actionConfig: accounts ? { botMxid: accounts.bot.user_id, botToken: accounts.bot.access_token,
    adminToken: accounts.admin.access_token, approvers: [accounts.admin.user_id],
    homeserverOrigin: 'https://crew.ominix.io:19443' } : undefined,
  inboxOptions: { requireProjectApproval: true } });
let running;
const report = () => {
  const state = server.inbox.state;
  writeFileSync(resolve(directory, 'backend.json'), JSON.stringify({
    actions: Object.values(state.records).map(({ id, state, revision, execution, result, lastError }) => ({ id, state, revision, execution, result, lastError })),
    notices: Object.values(state.notices), rooms: Object.values(state.rooms), fleets: Object.keys(store.state.fleets),
  }), { mode: 0o600 });
  if (accountsFile) {
    const journal = JSON.parse(readFileSync(accountsFile, 'utf8'));
    journal.fleets = [...new Set([...journal.fleets, ...Object.keys(store.state.fleets)])];
    journal.rooms = [...new Set([...journal.rooms, ...Object.values(state.rooms).map(r => r.roomId)])];
    writeFileSync(accountsFile, JSON.stringify(journal), { mode: 0o600 });
  }
};
// Accelerated reminder intervals belong only to this explicit validation runner.
// Production uses the standard one-hour/day/two-day schedule.
server.actionNotifications.intervals = [1500, 3000];
const timer = setInterval(() => {
  if (!running) running = service.serial(() => server.actionNotifications.tick())
    .catch(error => console.log(JSON.stringify({ notificationError: error.code ?? 'delivery_failed' })))
    .finally(() => { report(); running = null; });
}, 500);
timer.unref();
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({ ready: true, port, serverName, pid: process.pid })));
const close = async () => { clearInterval(timer); await running; report(); server.closeAllConnections(); server.close(() => { store.close(); process.exit(0); }); };
process.on('SIGTERM', close); process.on('SIGINT', close);
