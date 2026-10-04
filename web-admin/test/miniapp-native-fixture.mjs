// Explicit local-only Matrix fixture for the Rinx native instrument test.
// Runs the actual Palpo HTTP/session/workflow service. No deployed server calls.
import { fixture } from './fixture.mjs';
import { contributedFleet, acceptProjectReservations, refreshContributions, acceptAgentDecisions } from './project-workflow-fixture.mjs';
import { createApp } from '../server.mjs';
import { existsSync, writeFileSync } from 'node:fs';
const port = Number(process.argv[2]), directory = process.argv[3];
if (!Number.isSafeInteger(port) || port < 1024 || !directory) throw new Error('port and isolated evidence directory required');
const f = fixture({ path: directory + '/palpo.sqlite', transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
const server = createApp({ service: f.service, publicOrigin: `http://127.0.0.1:${port}`, startAccountWorker: false, startActionWorker: false,
  inboxOptions: { requireProjectApproval: true, approvers: ['@admin:example.test'] } });
const workflow = server.inbox.workflow;
server.projectCommands.adminToken = 'admin-secret'; // Explicit local fixture identity only.
await contributedFleet(f, workflow, server.inbox);
let consuming = false;
const consumer = setInterval(() => {
  if (consuming) return;
  consuming = true;
  f.service.serial(async () => {
    await refreshContributions(f, workflow, server.inbox);
    // The instrument releases a synthetic business receipt only after capturing
    // the pending state. UI/CI speed must not determine what this test proves.
    if (existsSync(directory + '/release-reservations')) await acceptProjectReservations(f, workflow, server.inbox);
    if (existsSync(directory + '/release-agent-decisions')) await acceptAgentDecisions(f, workflow, server.inbox);
  }).catch(error => console.error(error.code ?? error.name)).finally(() => { consuming = false; });
}, 1000);
consumer.unref();
const report = () => writeFileSync(directory + '/backend.json', JSON.stringify({
  actions: Object.values(server.inbox.state.records).map(({id, kind, state, execution, revision, result}) => ({id, kind, state, execution, revision, result})),
  fleets: Object.keys(f.store.state.fleets).length, projects: Object.keys(f.store.state.projects).length,
  requests: Object.keys(f.store.state.requests).length, logouts: f.calls.filter(c => c.path.endsWith('/logout')).length,
  requestStates: Object.values(f.store.state.requests).map(({state, usable}) => ({state, usable})),
  allocations: Object.values(f.store.state.requests).map(r => server.inbox.agents.allocation(r)),
  matrixMutations: f.calls.filter(c => c.method !== 'GET').length,
}));
const timer = setInterval(report, 100); timer.unref();
server.listen(port, '127.0.0.1', () => console.log('ready'));
process.on('SIGTERM', () => { report(); clearInterval(timer); clearInterval(consumer); server.close(() => { f.store.close(); process.exit(0); }); });
