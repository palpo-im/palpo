// Explicit local-only Matrix fixture for the Rinx native instrument test.
// Runs the actual Palpo HTTP/session/workflow service. No deployed server calls.
import { fixture, fleetInput } from './fixture.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { createApp } from '../server.mjs';
import { writeFileSync } from 'node:fs';
const port = Number(process.argv[2]), directory = process.argv[3];
if (!Number.isSafeInteger(port) || port < 1024 || !directory) throw new Error('port and isolated evidence directory required');
const f = fixture({ path: directory + '/palpo.sqlite', transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
const workflow = new Workflow(f.service);
const fleet = await f.service.create({ ...fleetInput, transportMode: 'callback' }, '@admin:example.test', 'admin-secret');
await workflow.connect(fleet.id, '@owner:example.test', 'owner-secret');
const server = createApp({ service: f.service, publicOrigin: `http://127.0.0.1:${port}`, startAccountWorker: false, startActionWorker: false,
  inboxOptions: { requireProjectApproval: true, approvers: ['@admin:example.test'] } });
const report = () => writeFileSync(directory + '/backend.json', JSON.stringify({
  actions: Object.values(server.inbox.state.records).map(({id, kind, state, execution, revision, result}) => ({id, kind, state, execution, revision, result})),
  fleets: Object.keys(f.store.state.fleets).length, projects: Object.keys(f.store.state.projects).length,
  requests: Object.keys(f.store.state.requests).length, logouts: f.calls.filter(c => c.path.endsWith('/logout')).length,
  matrixMutations: f.calls.filter(c => c.method !== 'GET').length,
}));
const timer = setInterval(report, 100); timer.unref();
server.listen(port, '127.0.0.1', () => console.log('ready'));
process.on('SIGTERM', () => { report(); clearInterval(timer); server.close(() => { f.store.close(); process.exit(0); }); });
