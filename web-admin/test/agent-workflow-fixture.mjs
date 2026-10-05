import { createHash } from 'node:crypto';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';
import { APP_ID, SERVICES } from '../lib/miniapp.mjs';
import { contributedFleet, projectBudget, projectAdministrators, acceptProjectReservations, projectResource } from './project-workflow-fixture.mjs';

export async function agentWorkflow(t, policy = projectAdministrators) {
  const f = fixture({ transportOrigin: 'https://transport.example.test', relayOrigin: 'http://relay.example.test' });
  const server = createApp({ service: f.service, publicOrigin: 'http://admin.example.test', startAccountWorker: false,
    startActionWorker: false, inboxOptions: { projectApprover: '@admin:example.test', requireProjectApproval: true } });
  t.after(() => f.store.close());
  const inbox = server.inbox, workflow = inbox.workflow, commands = server.projectCommands;
  commands.adminToken = 'admin-secret';
  const fleet = await contributedFleet(f, workflow, inbox);
  const login = async (token, services = Object.keys(SERVICES)) => (await server.miniapp.open(`Bearer ${token}`, { appId: APP_ID, bundleDigest: 'a'.repeat(64), services })).sessionToken;
  const owner = await login('owner-secret'), admin = await login('admin-secret'), assigned = await login('other-secret');
  const call = (session, service, args) => server.miniapp.call(`Bearer ${session}`, { service, args });
  const projectRequest = (await call(owner, 'palpo.inbox.submit', { requestId: 'project_for_agent', kind: 'project', name: 'Agent project', reason: 'Research', fleetId: fleet.id,
    resourceIds: [projectResource], allocations: projectBudget() })).action;
  await call(admin, 'palpo.inbox.decide', { id: projectRequest.id, expectedRevision: projectRequest.revision, commandId: 'project_decision', decision: 'approve', reason: 'Budget approved', ...policy });
  await acceptProjectReservations(f, workflow, inbox);
  const project = f.store.state.projects[projectRequest.result.projectId];
  const input = { requestId: 'agent_one', projectId: project.id, role: 'coding', requestedTokens: 100000, ratePerDay: 10000,
    agentDefinition: { name: 'ResearchBot', resourceId: projectResource } };
  const submit = (body = input) => call(owner, 'palpo.requests.create', body);
  const decideInput = row => ({ id: row.id, expectedRevision: row.revision, commandId: 'agent_decision', decision: 'approve', reason: 'Reviewed', allocatedTokens: 80000 });
  const decide = (row, extra = {}, session = assigned) => call(session, 'palpo.inbox.decide', { ...decideInput(row), ...extra });
  const entry = () => Object.values(commands.state.commands).find(e => e.command.operation.kind === 'approve_agent' || e.command.operation.kind === 'reject_agent');
  const receipt = e => ({ v: 1, fleetId: fleet.id, registrationGeneration: 1, commandId: e.command.commandId, commandDigest: e.digest,
    completedAtMs: inbox.now(), outcome: { status: 'applied', result: { kind: 'agent',
      engagementId: `en_${createHash('sha256').update(JSON.stringify([fleet.id, e.command.operation.request.requestId])).digest('hex').slice(0, 32)}`,
      state: e.command.operation.kind === 'approve_agent' ? 'reserved' : 'rejected', allocatedTokens: e.command.operation.allocatedTokens ?? input.requestedTokens, cleanup: 'not_required' } } });
  const accept = async e => f.service.outbound.updates(fleet, { v: 2, generation: fleet.transport.generation, sequence: fleet.transport.sequence + 1,
    heartbeat: true, commandReceipts: [receipt(e)] }, workflow);
  return { ...f, inbox, workflow, commands, fleet, project, input, owner, admin, assigned, call, login, submit, decide, decideInput, entry, receipt, accept };
}
