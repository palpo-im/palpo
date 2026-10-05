import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical } from './outbound.mjs';
import { contributionKey } from './project-contributions.mjs';
import { fields } from './miniapp.mjs';

const hash = v => createHash('sha256').update(canonical(v)).digest('hex');
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const positive = v => Number.isSafeInteger(v) && v > 0;
const amount = v => typeof v === 'number' ? v : typeof v === 'string' && /^[1-9][0-9]*$/.test(v) ? Number(v) : NaN;

// Project roles come from accepted grants, never Matrix room powers or the
// server-admin flag. An agent request is not sent to the legacy console queue.
export class AgentApprovals {
  constructor(inbox) { this.inbox = inbox; this.store = inbox.store; this.service = inbox.service; this.workflow = inbox.workflow; }
  get commands() { return this.inbox.projects.commands; }
  manages(row) { return ['agent', 'top_up', 'agent_removal'].includes(row?.kind); }
  grant(id, cleanup = false) {
    const record = this.inbox.projectCommands?.state.grants[id], grant = record?.grant;
    const project = this.store.state.projects[grant?.projectId], fleet = this.store.state.fleets[record?.fleetId];
    if (!record || !(cleanup ? ['accepted', 'revoked'].includes(record.state) : record.state === 'accepted') || !project || !fleet || project.fleetId !== fleet.id
      || project.ownerMxid !== grant.ownerMxid || project.roomId !== grant.roomId
      || project.resourceGrant?.v !== 1 || !project.resourceGrant.grantIds.includes(grant.id)
      || record.registrationGeneration !== fleet.projectWorkflow?.registrationGeneration
      || (!cleanup && grant.expiresAtMs <= this.inbox.now())) return null;
    const root = this.commands.state.contributions[contributionKey(fleet.id, record.registrationGeneration, grant.delegationId)];
    if (!cleanup && (root?.state !== 'active' || root.grant.expiresAtMs <= this.inbox.now())) return null;
    return { record, grant, project, fleet, root };
  }
  administrators(row) {
    const value = this.grant(row.grantId, row.kind === 'agent_removal');
    if (!value) return [];
    const { record, grant } = value;
    return grant.administratorMxids.filter(id => !record.desiredAdministrators || record.desiredAdministrators.includes(id));
  }
  canRead(row, actor) {
    return row.ownerMxid === actor || row.requesterMxid === actor || this.administrators(row).includes(actor);
  }
  canDecide(row, actor) {
    if (row.kind === 'agent_removal') return false;
    const value = this.grant(row.grantId);
    return !!value && row.state === 'requested' && row.execution === 'pending'
      && value.grant.revision === row.grantRevision && (value.record.desiredRevision ?? value.grant.revision) === row.grantRevision
      && this.administrators(row).includes(actor) && (value.grant.allowSelfApproval || row.requesterMxid !== actor);
  }
  reviewer(actor) {
    return Object.keys(this.inbox.projectCommands?.state.grants ?? {}).some(grantId => this.administrators({ grantId }).includes(actor));
  }
  select(project, payload) {
    if (!payload.agentDefinition) fail(400, 'agent_definition_required', 'Name the agent and select an allocated resource.');
    const value = (project.resourceGrant?.grantIds ?? []).map(id => this.grant(id))
      .find(v => v?.root.grant.resourceId === payload.agentDefinition.resourceId);
    if (!value || !this.inbox.projects.allocation(project).ready) fail(409, 'project_allocation_required', 'Select a current accepted project allocation.');
    this.commands.requireSupport(value.fleet);
    if (payload.requestedTokens > value.grant.limits.tokens || payload.ratePerDay > value.grant.limits.maxRatePerDay)
      fail(409, 'project_budget_exceeded', 'This request exceeds the project token or daily-rate limit.');
    return value;
  }
  submit(request, project) {
    if (request.actionId) {
      const row = this.inbox.state.records[request.actionId];
      if (!row || row.requestKey !== request.id) fail(409, 'agent_action_conflict', 'The saved agent action is unavailable.');
      return this.workflow.requestView(request, request.requesterMxid);
    }
    if (request.workflowVersion !== 1) fail(409, 'agent_migration_required', 'This historical request retains its original approval workflow.');
    const { grant } = this.select(project, request.payload);
    const id = `action_${hash({ agent: request.id }).slice(0, 32)}`;
    if (Object.keys(this.inbox.state.records).length >= this.inbox.maxRecords
      || Object.values(this.inbox.state.records).filter(r => r.requesterMxid === request.requesterMxid && r.state === 'requested').length >= 100)
      fail(429, 'inbox_full', 'Too many workflow records. Contact the server administrator.');
    this.store.atomic(() => {
      const row = this.inbox.state.records[id] = { id, kind: 'agent', workflowVersion: 1, requestKey: request.id,
        grantId: grant.id, grantRevision: grant.revision, ownerMxid: project.ownerMxid, requesterMxid: request.requesterMxid,
        payload: { name: request.payload.agentDefinition.name, reason: `Request a ${request.payload.role} agent`,
          projectId: project.id, projectName: project.name, fleetId: project.fleetId, resourceId: request.payload.agentDefinition.resourceId,
          requestedTokens: request.payload.requestedTokens, ratePerDay: request.payload.ratePerDay },
        state: 'requested', execution: 'pending', revision: 1, createdAt: this.inbox.now(), updatedAt: this.inbox.now() };
      request.actionId = id; request.state = 'pending_project_admin'; request.usable = false; request.statusVerified = false; request.lastError = null;
      this.inbox.notify(row); this.store.audit(request.requesterMxid, 'agent.request', project.fleetId, id, 'pending_project_admin');
    });
    return this.workflow.requestView(request, request.requesterMxid);
  }
  canTopUp(request, actor) {
    const original = this.inbox.state.records[request.actionId];
    return original?.kind === 'agent' && original.ownerMxid === actor && original.state === 'approved'
      && original.execution === 'done' && !!this.grant(original.grantId) && !request.retirement && !request.removalActionId
      && !['ended', 'rejected', 'decision_refused'].includes(request.state);
  }
  allocation(request) {
    const original = this.inbox.state.records[request.actionId];
    if (!original || original.state !== 'approved') return null;
    const entries = Object.values(this.commands.state.commands).filter(e => e.actionId === original.id
      || (e.command.operation.kind === 'top_up_agent' && e.command.operation.engagementId === original.result?.engagementId
        && e.command.operation.grantId === original.grantId));
    const tokens = state => entries.filter(e => state === 'applied' ? e.receipt?.outcome.status === state : !e.receipt && !e.cancelled)
      .reduce((n, e) => n + (e.command.operation.allocatedTokens ?? e.command.operation.addTokens ?? 0), 0);
    return { tokens: tokens('applied'), pendingTokens: tokens('pending') };
  }
  async submitTopUp(input, actor, token) {
    fields(input, ['requestId', 'kind', 'agentRequestId', 'addTokens', 'reason']);
    if (typeof input.requestId !== 'string' || !/^[A-Za-z0-9_-]{1,80}$/.test(input.requestId)
      || typeof input.reason !== 'string' || !input.reason.trim() || input.reason.length > 1000 || /[\x00-\x1f]/.test(input.reason)
      || !positive(amount(input.addTokens))) fail(400, 'invalid_top_up', 'Request a positive token increase with a reason and stable request ID.');
    const id = `action_${hash({ actor, requestId: input.requestId }).slice(0, 32)}`;
    const fingerprint = hash({ kind: 'top_up', actor, agentRequestId: input.agentRequestId, addTokens: amount(input.addTokens), reason: input.reason.trim() });
    const old = this.inbox.state.records[id];
    if (old) {
      if (old.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This request ID has different content.');
      return this.inbox.get(id, actor, false);
    }
    const request = this.store.state.requests[input.agentRequestId];
    if (!request || !this.canTopUp(request, actor)) fail(404, 'agent_not_available', 'This agent is unavailable for a token increase.');
    const original = this.inbox.state.records[request.actionId], value = this.grant(original.grantId);
    await this.workflow.validateProjectRoom(value.project.roomId, actor, token, value.project.ownerMxid, true);
    this.commands.requireSupport(value.fleet);
    const requestedTokens = amount(input.addTokens);
    if (requestedTokens > this.remaining(value.grant).tokens) fail(409, 'project_capacity_unavailable', 'The increase exceeds the remaining project budget.');
    if (Object.keys(this.inbox.state.records).length >= this.inbox.maxRecords
      || Object.values(this.inbox.state.records).filter(r => r.requesterMxid === actor && r.state === 'requested').length >= 100)
      fail(429, 'inbox_full', 'Too many workflow records.');
    this.store.atomic(() => {
      const row = this.inbox.state.records[id] = { id, kind: 'top_up', workflowVersion: 1, requestKey: request.id, fingerprint,
        grantId: value.grant.id, grantRevision: value.grant.revision, engagementId: original.result.engagementId,
        ownerMxid: actor, requesterMxid: actor,
        payload: { name: original.payload.name, reason: input.reason.trim(), projectId: value.project.id,
          projectName: value.project.name, fleetId: value.fleet.id, requestedTokens, ratePerDay: original.payload.ratePerDay },
        state: 'requested', execution: 'pending', revision: 1, createdAt: this.inbox.now(), updatedAt: this.inbox.now() };
      this.inbox.notify(row); this.store.audit(actor, 'agent.request_top_up', value.fleet.id, row.id, 'pending_project_admin');
    });
    return this.inbox.get(id, actor, false);
  }
  async person(actor, token) {
    const identity = await this.service.palpo.call('/_matrix/client/v3/account/whoami', token);
    if (identity.user_id !== actor || identity.is_guest) fail(403, 'project_administrator_required', 'Use your current Matrix account.');
    if (!this.commands.adminToken) fail(503, 'workflow_authority_unavailable', 'Current account authority lookup is not configured.');
    const user = await this.service.palpo.user(actor, this.commands.adminToken);
    if (!user || user.locked || user.deactivated || user.appservice_id) fail(403, 'project_administrator_required', 'The decision maker must be an active person.');
  }
  remaining(grant) {
    const entries = Object.values(this.commands.state.commands).filter(e => e.command.operation.grantId === grant.id
      && !e.cancelled && e.receipt?.outcome.status !== 'refused');
    const agents = entries.filter(e => e.command.operation.kind === 'approve_agent');
    const tokens = agents.reduce((n, e) => n + e.command.operation.allocatedTokens, 0)
      + entries.filter(e => e.command.operation.kind === 'top_up_agent').reduce((n, e) => n + e.command.operation.addTokens, 0);
    const held = agents.filter(entry => {
      const request = this.store.state.requests[this.inbox.state.records[entry.actionId]?.requestKey];
      const removal = this.inbox.state.records[request?.removalActionId];
      return removal?.kind !== 'agent_removal' || removal.execution !== 'done'
        || removal.engagementId !== entry.receipt?.outcome.result?.engagementId;
    });
    // Verified retirement releases concurrency and daily rate. Allocated tokens
    // remain lifetime debits: a usage lower bound cannot justify a token refund.
    return { tokens: Math.max(0, grant.limits.tokens - tokens), maxAgents: Math.max(0, grant.limits.maxAgents - held.length),
      maxRatePerDay: Math.max(0, grant.limits.maxRatePerDay - held.reduce((n, e) => n + e.command.operation.request.ratePerDay, 0)) };
  }
  async decide(input, actor, token) {
    fields(input, ['id', 'expectedRevision', 'commandId', 'decision', 'reason', 'allocatedTokens']);
    const row = this.inbox.record(input.id, actor, false);
    if (!this.administrators(row).includes(actor)) fail(403, 'project_administrator_required', 'Only an explicitly assigned project administrator can decide this request.');
    if (!['approve', 'reject'].includes(input.decision) || !Number.isSafeInteger(input.expectedRevision)
      || typeof input.commandId !== 'string' || !/^[A-Za-z0-9_-]{1,80}$/.test(input.commandId)
      || typeof input.reason !== 'string' || !input.reason.trim() || input.reason.length > 1000 || /[\x00-\x1f]/.test(input.reason)) fail(400, 'invalid_decision', 'Review the current request and enter a decision reason.');
    const allocatedTokens = input.decision === 'approve' ? amount(input.allocatedTokens) : null;
    if (input.decision === 'approve' ? !positive(allocatedTokens) || allocatedTokens > row.payload.requestedTokens : input.allocatedTokens !== undefined)
      fail(400, 'invalid_allocation', 'Approve a positive whole number of tokens no greater than the request.');
    const command = { commandId: input.commandId, actor, expectedRevision: input.expectedRevision, decision: input.decision, reason: input.reason.trim(), allocatedTokens };
    await this.person(actor, token);
    if (row.command) {
      if (canonical(row.command) !== canonical(command)) fail(409, 'decision_conflict', 'This action already has a different decision.');
      return this.inbox.get(row.id, actor, false);
    }
    if (!this.canDecide(row, actor)) fail(403, 'project_administrator_required', 'The current assignment or self-approval policy does not permit this decision.');
    if (row.revision !== input.expectedRevision) fail(409, 'decision_conflict', 'Refresh the latest request before deciding.');
    const { project, fleet } = this.grant(row.grantId);
    await this.inbox.projects.validateBinding(project, fleet);
    // Recheck after Matrix I/O and immediately before the atomic decision.
    await this.person(actor, token);
    const current = this.grant(row.grantId);
    if (!current || !this.canDecide(row, actor)) fail(409, 'project_grant_changed', 'The project assignment changed. Refresh the request.');
    this.commands.requireSupport(fleet);
    this.inbox.projects.contribution(fleet, current.grant.delegationId);
    if (input.decision === 'approve') {
      const available = this.remaining(current.grant);
      if (allocatedTokens > available.tokens || (row.kind === 'agent' && (available.maxAgents < 1 || row.payload.ratePerDay > available.maxRatePerDay)))
        fail(409, 'project_capacity_unavailable', 'This decision exceeds the remaining project budget.');
    }
    const request = this.store.state.requests[row.requestKey];
    if (!request?.sourceEventId || (row.kind === 'agent' ? request.actionId !== row.id : !this.canTopUp(request, row.requesterMxid)))
      fail(409, 'agent_action_conflict', 'The original Matrix request is unavailable.');
    this.store.atomic(() => {
      const operation = row.kind === 'top_up' ? { kind: 'top_up_agent', grantId: row.grantId, grantRevision: row.grantRevision,
        engagementId: row.engagementId, requesterMxid: row.requesterMxid, addTokens: allocatedTokens }
        : { kind: input.decision === 'approve' ? 'approve_agent' : 'reject_agent',
        grantId: row.grantId, grantRevision: row.grantRevision, request: { ...request.payload, sourceEventId: request.sourceEventId },
        ...(input.decision === 'approve' ? { allocatedTokens } : {}) };
      const entry = row.kind === 'top_up' && input.decision === 'reject' ? null : this.commands.enqueue(fleet, actor, operation, {
        commandId: `agent_${hash({ action: row.id, command: input.commandId }).slice(0, 40)}`, projectId: project.id, actionId: row.id,
        expiresAtMs: Math.min(current.grant.expiresAtMs, this.inbox.now() + 7 * 86400000) });
      row.command = command; row.state = input.decision === 'approve' ? 'approved' : 'rejected'; row.execution = entry ? 'awaiting_hagency' : 'done';
      if (entry) entry.actionKind = row.kind;
      row.commandRef = entry?.command.commandId ?? null; row.decision = { by: actor, at: this.inbox.now(), reason: command.reason };
      row.updatedAt = this.inbox.now(); row.revision++;
      if (row.kind === 'agent') request.state = input.decision === 'approve' ? 'approval_queued' : 'rejection_queued';
      this.inbox.notify(row); this.store.audit(actor, 'agent.decide', fleet.id, row.id, row.state);
    });
    return this.inbox.get(row.id, actor, false);
  }
  receipt(entry) {
    const row = this.inbox.state.records[entry.actionId];
    if (!this.manages(row) || row.kind === 'agent_removal' || row.commandRef !== entry.command.commandId) return;
    const request = this.store.state.requests[row.requestKey];
    row.execution = entry.state === 'applied' ? 'done' : 'command_refused';
    row.result = entry.receipt.outcome.result ?? { code: entry.receipt.outcome.code };
    row.updatedAt = this.inbox.now(); row.revision++;
    if (request && row.kind === 'agent') {
      request.decisionReceipt = { state: entry.state, ...row.result }; request.usable = false;
      request.state = entry.state === 'refused' ? 'decision_refused' : row.state === 'approved' ? 'provisioning' : 'rejected';
    }
    this.inbox.notify(row);
  }
  validateStatus(request, status, incoming = []) {
    const row = this.inbox.state.records[request.actionId];
    const entry = Object.values(this.commands.state.commands).find(e => e.actionId === row?.id && e.command.commandId === row?.commandRef);
    const receipt = entry?.receipt ?? incoming.find(r => r.entry === entry)?.receipt;
    // Receipt publication is paginated independently from status publication.
    // A future status must not poison the immutable batch that carries earlier
    // receipts; ignore it until its own admission receipt is available.
    if (!row || !entry || receipt?.outcome.status !== 'applied') return false;
    if (status.engagementId !== receipt.outcome.result.engagementId
      || (row.state === 'approved' ? !['active', 'ended'].includes(status.state) : row.state !== 'rejected' || status.state !== 'rejected'))
      fail(409, 'agent_decision_pending', 'Agent status must follow the exact applied project-administrator decision.');
    return true;
  }
  authorizes(entry) {
    const row = this.inbox.state.records[entry.actionId], op = entry.command.operation;
    const request = this.store.state.requests[row?.requestKey];
    if (!this.manages(row) || !request) return false;
    const binding = row?.kind === 'top_up' ? request && this.canTopUp(request, row.requesterMxid)
      && op.engagementId === row.engagementId && op.requesterMxid === row.requesterMxid && op.addTokens === row.command?.allocatedTokens
      : request?.actionId === row?.id && canonical({ ...request?.payload, sourceEventId: request?.sourceEventId }) === canonical(op.request);
    return this.manages(row) && !!this.grant(row.grantId) && !!request && !!binding
      && row.commandRef === entry.command.commandId
      && row.decision?.by === entry.command.actorMxid && row.grantId === op.grantId && row.grantRevision === op.grantRevision
      && row.state === (op.kind === 'reject_agent' ? 'rejected' : 'approved');
  }
}
