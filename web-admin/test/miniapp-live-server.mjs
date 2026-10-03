// Operator-invoked validation sidecar. Uses real Matrix authentication and an
// isolated workflow database; no notification worker or production database.
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { Store } from '../lib/store.mjs';
import { Palpo, Service } from '../lib/service.mjs';
import { createApp } from '../server.mjs';

const directory = resolve(process.argv[2] ?? '');
const port = Number(process.argv[3]);
const matrix = new URL(process.argv[4]);
const serverName = process.argv[5];
if (!process.argv[2] || !Number.isSafeInteger(port) || port < 1024 || port > 65535
  || matrix.hostname !== '127.0.0.1' || matrix.protocol !== 'http:' || !serverName) throw new Error('Explicit isolated directory, loopback port and Matrix upstream required.');
mkdirSync(directory, { recursive: true, mode: 0o700 });
const store = new Store(resolve(directory, 'validation.sqlite'));
const service = new Service({ store, palpo: new Palpo(matrix.href), serverName });
const server = createApp({ service, publicOrigin: `http://127.0.0.1:${port}`,
  startAccountWorker: false, startActionWorker: false,
  inboxOptions: { requireProjectApproval: true } });
server.listen(port, '127.0.0.1', () => console.log(JSON.stringify({ ready: true, port, serverName, pid: process.pid })));
const close = () => { server.closeAllConnections(); server.close(() => { store.close(); process.exit(0); }); };
process.on('SIGTERM', close); process.on('SIGINT', close);
