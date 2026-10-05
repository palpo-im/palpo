// Explicit local-only Matrix fixture for the Rinx native instrument test.
// Runs the actual Palpo HTTP/session/workflow service. No deployed server calls.
import { accountFixture, applicant } from './accounts.fixture.mjs';
import { contributedFleet, acceptProjectReservations, refreshContributions, acceptAgentDecisions, acceptAgentRemovals, publishAgentCleanup, publishReadyAgent, projectResource, projectAdministrators } from './project-workflow-fixture.mjs';
import { createApp } from '../server.mjs';
import { existsSync, writeFileSync } from 'node:fs';
const port = Number(process.argv[2]), directory = process.argv[3];
if (!Number.isSafeInteger(port) || port < 1024 || !directory) throw new Error('port and isolated evidence directory required');
const f = accountFixture({ path: directory + '/palpo.sqlite', transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
const server = createApp({ service: f.service, publicOrigin: `http://127.0.0.1:${port}`, startAccountWorker: false, startActionWorker: false,
  accountConfig: f.config,
  inboxOptions: { requireProjectApproval: true, approvers: ['@admin:example.test'] } });
const workflow = server.inbox.workflow;
await server.accounts.tick(); f.joinAdmin();
const signup = applicant('miniapp_signup');
server.accounts.submit(signup); await server.accounts.tick();
server.projectCommands.adminToken = 'admin-secret'; // Explicit local fixture identity only.
await contributedFleet(f, workflow, server.inbox);
const seededRecoveries = new Set();
async function seedRecovery(mode) {
  if (seededRecoveries.has(mode) || !existsSync(directory + '/seed-' + mode + '-project')) return;
  const fleet = Object.values(f.store.state.fleets)[0], resourceId = `resource_${'b'.repeat(24)}`;
  const original = Object.values(server.projectCommands.state.contributions).find(r => r.grant.resourceId === projectResource);
  if (!fleet.capabilities.offers[0].resources.some(r => r.id === resourceId)) fleet.capabilities.offers[0].resources.push({ ...fleet.capabilities.offers[0].resources[0], id: resourceId, name: 'Recovery analysis resource' });
  const second = { grant: { ...original.grant, id: 'contribution_recovery', resourceId }, state: 'active', reserved: { tokens: 0, maxAgents: 0, maxRatePerDay: 0 } };
  await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true,
    contributionPage: { v: 1, registrationGeneration: 1, observedAtMs: server.inbox.now(), after: '', nextAfter: null, contributions: [second] } }, workflow);
  const { action } = await server.inbox.submit({ requestId: `recovery_${mode}`, kind: 'project', name: `Recovery ${mode} project`, reason: 'Recover a partial reservation',
    fleetId: fleet.id, resourceIds: [projectResource, resourceId], allocations: [original.grant, second.grant].map(g => ({ contributionId: g.id, resourceId: g.resourceId,
      limits: { tokens: 10000, maxAgents: 1, maxRatePerDay: 1000 }, durationHours: 24 })) }, '@owner:example.test', 'owner-secret');
  await server.inbox.decide({ id: action.id, expectedRevision: action.revision, commandId: `approve_recovery_${mode}`, decision: 'approve', reason: 'Fixture seed approval', ...projectAdministrators }, '@admin:example.test', 'admin-secret');
  const entries = Object.values(server.projectCommands.state.commands).filter(e => e.actionId === action.id);
  const commandReceipts = entries.map((e, index) => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest,
    completedAtMs: server.inbox.now(), outcome: index === 0 ? { status: 'applied', result: { kind: 'grant', grant: e.command.operation.grant } } : { status: 'refused', code: 'insufficient_capacity' } }));
  await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true, commandReceipts }, workflow);
  seededRecoveries.add(mode);
}
async function releaseUnused() {
  if (!existsSync(directory + '/apply-unused-releases')) return;
  for (const fleet of Object.values(f.store.state.fleets)) {
    const entries = Object.values(server.projectCommands.state.commands).filter(e => !e.receipt && e.command.fleetId === fleet.id && e.command.operation.kind === 'release_unused_project');
    if (!entries.length) continue;
    const commandReceipts = entries.slice(0, 8).map(e => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest,
      completedAtMs: server.inbox.now(), outcome: { status: 'applied', result: { kind: 'released_unused_project', grantId: e.command.operation.grantId, revision: e.command.operation.expectedRevision } } }));
    await f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1, heartbeat: true, commandReceipts }, workflow);
  }
}
let consuming = false;
const consumer = setInterval(() => {
  if (consuming) return;
  consuming = true;
  f.service.serial(async () => {
    await refreshContributions(f, workflow, server.inbox);
    // The instrument releases a synthetic business receipt only after capturing
    // the pending state. UI/CI speed must not determine what this test proves.
    if (existsSync(directory + '/release-reservations') && (!existsSync(directory + '/seed-retry-project') || existsSync(directory + '/apply-project-recovery')))
      await acceptProjectReservations(f, workflow, server.inbox);
    if (existsSync(directory + '/release-agent-decisions')) await acceptAgentDecisions(f, workflow, server.inbox);
    if (existsSync(directory + '/release-agent-removals')) {
      await acceptAgentRemovals(f, workflow, server.inbox);
      await publishAgentCleanup(f, workflow, server.inbox, existsSync(directory + '/release-agent-cleanup') ? 'complete'
        : existsSync(directory + '/fail-agent-cleanup') ? 'failed' : 'pending');
    }
    if (existsSync(directory + '/publish-ready-agent')) {
      for (const request of Object.values(f.store.state.requests)) await publishReadyAgent(f, workflow, server.inbox, request);
    }
    await seedRecovery('retry'); await seedRecovery('release'); await releaseUnused();
  }).catch(error => console.error(error.code ?? error.name)).finally(() => { consuming = false; });
}, 1000);
consumer.unref();
const report = () => writeFileSync(directory + '/backend.json', JSON.stringify({
  actions: Object.values(server.inbox.state.records).map(({id, kind, state, execution, revision, result}) => ({id, kind, state, execution, revision, result})),
  fleets: Object.keys(f.store.state.fleets).length, projects: Object.keys(f.store.state.projects).length,
  requests: Object.keys(f.store.state.requests).length, logouts: f.calls.filter(c => c.path.endsWith('/logout')).length,
  requestStates: Object.values(f.store.state.requests).map(({state, usable}) => ({state, usable})),
  agentChats: Object.values(f.store.state.requests).filter(r => r.provider?.agentMxid).map(r => ({
    account: r.requesterMxid, roomId: r.payload.targetRoomId, agentMxid: r.provider.agentMxid })),
  allocations: Object.values(f.store.state.requests).map(r => server.inbox.agents.allocation(r)),
  removalCommands: Object.values(server.projectCommands.state.commands).filter(e => e.command.operation.kind === 'revoke_agent').length,
  recoveries: Object.values(server.inbox.state.records).filter(r => r.requestId?.startsWith('recovery_')).map(r => ({ name: r.payload.name, state: r.state, execution: r.execution,
    owner: r.ownerMxid, allocated: r.result?.projectId ? server.inbox.projects.allocation(f.store.state.projects[r.result.projectId]).ready : false,
    commands: Object.values(server.projectCommands.state.commands).filter(e => e.actionId === r.id).map(e => e.command.operation.kind) })),
  signup: { status: server.accounts.state.requests[signup.id].status, roomId: server.accounts.state.roomId,
    eventId: server.accounts.state.requests[signup.id].sourceEventId, registrations: f.credentials.size },
  notificationPreferences: Object.fromEntries(['@owner:example.test', '@admin:example.test', '@other:example.test'].map(actor => [actor, server.inbox.preferences.get(actor)])),
  matrixMutations: f.calls.filter(c => c.method !== 'GET').length,
}));
const timer = setInterval(report, 100); timer.unref();
server.listen(port, '127.0.0.1', () => console.log('ready'));
process.on('SIGTERM', () => { report(); clearInterval(timer); clearInterval(consumer); server.close(() => { f.store.close(); process.exit(0); }); });
