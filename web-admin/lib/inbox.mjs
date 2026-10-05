import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical } from './outbound.mjs';
import { fields } from './miniapp.mjs';
import { ProjectApprovals } from './project-approvals.mjs';
import { AgentApprovals } from './agent-approvals.mjs';
import { NotificationPreferences } from './notification-preferences.mjs';
import { AgentLifecycle } from './agent-lifecycle.mjs';

const hash = value => createHash('sha256').update(canonical(value)).digest('hex');
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const key = value => {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{1,80}$/.test(value)) fail(400, 'invalid_request_id', 'Use a stable request ID.');
  return value;
};
const text = (value, name, max) => {
  if (typeof value !== 'string' || !value.trim() || value.length > max || /[\x00-\x1f]/.test(value)) fail(400, 'invalid_input', `${name} is required (maximum ${max} characters).`);
  return value.trim();
};

// Persist workflow, audit and notification intent in the same SQLite transaction.
// No token, export content, or Matrix event is a workflow decision record.
export class Inbox {
  constructor(service, workflow, { now = Date.now, approvers = [], projectApprover, maxRecords = 10000, requireProjectApproval = false } = {}) {
    Object.assign(this, { service, workflow, now, approvers, maxRecords, requireProjectApproval });
    // One business-role holder; a notification list or Matrix admin bit alone
    // is not project approval authority. A single legacy recipient migrates
    // unambiguously; absent/ambiguous configuration fails closed.
    this.projectApprover = projectApprover ?? (approvers.length === 1 ? approvers[0] : null);
    if (this.projectApprover !== null) this.projectApprover = service.owner(this.projectApprover);
    this.approvers = this.projectApprover ? [this.projectApprover] : [];
    this.store = service.store;
    this.store.state.actionInbox ??= { records: {}, notices: {}, rooms: {} };
    this.preferences = new NotificationPreferences(this);
    this.projects = new ProjectApprovals(this);
    this.agents = new AgentApprovals(this);
    this.lifecycle = new AgentLifecycle(this);
    workflow.projectGrant = (input, actor, existing, options) => this.projectGrant(input, actor, existing, options);
    workflow.resourceGrant = (project, resource, existing) => this.resourceGrant(project, resource, existing);
    workflow.projectAllocation = project => this.projects.allocation(project);
    workflow.agentRequest = (request, project) => this.agents.submit(request, project);
    workflow.agentRequestPlan = (project, payload) => this.agents.select(project, payload);
    workflow.agentStatus = (request, status, receipts) => this.agents.validateStatus(request, status, receipts);
    workflow.agentLifecycle = request => this.lifecycle.observe(request);
  }
  get state() { return this.store.state.actionInbox; }
  canApproveProjects(actor, admin) { return !!admin && actor === this.projectApprover; }
  pending(row, actor, admin) {
    if (row.kind === 'agent_removal') return this.lifecycle.canRetry(row, actor) || (row.execution === 'inspection_required' && row.requesterMxid === actor);
    if (this.agents.manages(row)) return this.agents.canDecide(row, actor);
    admin = this.canApproveProjects(actor, admin);
    // Legacy contribution records remain readable, but contribution now starts
    // in Hagency, not as an action for a Rinx project manager.
    if (row.kind !== 'project') return false;
    const recovery = this.projects.recoveryOptions(row, actor, admin);
    if (recovery.canRetryReservation || recovery.canReleaseReservation) return true;
    if (row.workflowVersion === 1) return row.execution === 'preparing' ? row.ownerMxid === actor : row.state === 'requested' && row.execution === 'pending' && admin;
    if (row.state === 'requested') return admin;
    return row.state === 'approved' && row.ownerMxid === actor && row.execution !== 'done';
  }
  reminderStatus(row, actor, admin) {
    if (!this.pending(row, actor, admin)) return null;
    const notice = Object.values(this.state.notices).find(n => n.actionId === row.id && n.revision === row.revision && n.recipient === actor);
    if (!notice) return null;
    const prefs = this.preferences.get(actor);
    return { enabled: prefs.enabled && prefs.remindersEnabled,
      overdue: this.now() >= notice.createdAt + prefs.reminderMinutes.at(-1) * 60000,
      remindersSent: Math.max(0, notice.delivered - 1),
      snoozedUntil: (notice.snoozedUntil ?? 0) > this.now() ? new Date(notice.snoozedUntil).toISOString() : null };
  }
  view(row, actor, admin) {
    admin = this.canApproveProjects(actor, admin);
    const { fingerprint, command, commandRef, requestKey, reservations, releases, recoveryHistory, recovery, ...copy } = row;
    const resourceResult = ({ grantId, state, code }) => {
      const allocation = row.payload.allocations?.[reservations?.findIndex(r => r.grantId === grantId) ?? -1];
      return { grantId, state, code: code ?? null, resourceName: allocation?.resourceName ?? allocation?.resourceId ?? 'Project resource' };
    };
    return { ...copy, requesterMxid: row.requesterMxid ?? row.ownerMxid, workflowVersion: row.workflowVersion ?? null, needsMyAction: this.pending(row, actor, admin),
      payload: { ...row.payload, allocations: row.payload.allocations?.map(a => ({ ...a, resourceName: a.resourceName ?? a.resourceId, expiresAt: new Date(a.expiresAtMs).toISOString() })) ?? null },
      reservations: reservations?.map(resourceResult) ?? null,
      releases: releases?.map(resourceResult) ?? null,
      ...this.projects.recoveryOptions(row, actor, admin),
      nextAction: this.agents.manages(row) ? this.agents.canDecide(row, actor) ? 'review_agent' : null
        : row.workflowVersion === 1 ? row.execution === 'preparing' ? 'prepare_project' : row.state === 'requested' ? 'review' : null
        : row.kind !== 'project' ? null : row.state === 'requested' ? 'review'
        : row.state === 'approved' && row.execution !== 'done' ? 'activate_project' : null,
      reminderStatus: this.reminderStatus(row, actor, admin),
      canRetry: this.lifecycle.canRetry(row, actor),
      canDecide: this.agents.manages(row) ? this.agents.canDecide(row, actor) : admin && row.kind === 'project' && row.state === 'requested' && row.execution === 'pending',
      canContinue: !this.agents.manages(row) && this.pending(row, actor, admin) && (row.execution === 'preparing' || row.workflowVersion !== 1 && row.state === 'approved') };
  }
  record(id, actor, admin) {
    const row = this.state.records[id];
    if (!row || !this.canRead(row, actor, admin)) fail(404, 'action_not_found', 'Action not found.');
    return row;
  }
  canRead(row, actor, admin) {
    return this.agents.manages(row) ? this.agents.canRead(row, actor) : row.ownerMxid === actor || this.canApproveProjects(actor, admin);
  }
  get(id, actor, admin) {
    let row = this.record(id, actor, admin);
    const latest = row.kind === 'agent_removal' && this.store.state.requests[row.requestKey]?.removalActionId;
    if (latest && latest !== row.id) row = this.record(latest, actor, admin);
    return { action: this.view(row, actor, admin) };
  }
  list(actor, admin, { view = 'needs_action', offset = 0, limit = 50 } = {}) {
    if (!['needs_action', 'waiting', 'history', 'all'].includes(view) || !Number.isSafeInteger(offset) || offset < 0
      || !Number.isSafeInteger(limit) || limit < 1 || limit > 100) fail(400, 'invalid_page', 'Invalid inbox page.');
    admin = this.canApproveProjects(actor, admin);
    const allowed = Object.values(this.state.records).filter(row => this.canRead(row, actor, admin));
    const complete = row => row.state === 'cancelled' || row.execution === 'superseded' || (row.state === 'rejected' ? !this.agents.manages(row) || row.execution === 'done' : row.state === 'approved' && row.execution === 'done');
    const selected = allowed.filter(row => row.kind === 'contribution' ? ['history', 'all'].includes(view) : view === 'all' || (view === 'needs_action' ? this.pending(row, actor, admin)
      : view === 'waiting' ? !this.pending(row, actor, admin) && !complete(row)
        : complete(row) && !this.pending(row, actor, admin)));
    selected.sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id));
    return { actions: selected.slice(offset, offset + limit).map(row => this.view(row, actor, admin)), total: selected.length,
      pendingCount: allowed.filter(row => this.pending(row, actor, admin)).length,
      room: this.state.rooms[actor] ? { roomId: this.state.rooms[actor].roomId, botMxid: this.state.rooms[actor].botMxid, serverName: this.service.serverName } : null };
  }
  notify(row) {
    // Discard obsolete queued deliveries; already delivered cards remain links to
    // the current record. Reminder state is per recipient, independent of read.
    for (const notice of Object.values(this.state.notices)) if (notice.actionId === row.id && notice.revision !== row.revision) notice.cancelled = true;
    const recipients = this.agents.manages(row) ? [row.ownerMxid, row.requesterMxid, ...this.agents.administrators(row)] : [row.ownerMxid, ...this.approvers];
    for (const recipient of new Set(recipients)) {
      const id = `${row.id}_${row.revision}_${hash(recipient).slice(0, 16)}`;
      this.state.notices[id] ??= { id, actionId: row.id, revision: row.revision, recipient, createdAt: this.now(), dueAt: this.now(), attempt: 0, delivered: 0, seenAt: null, cancelled: false };
    }
  }
  async submit(input, actor, token) {
    if (input?.kind === 'agent_removal') return this.lifecycle.submit(input, actor, token);
    if (input?.kind === 'top_up') return this.agents.submitTopUp(input, actor, token);
    fields(input, ['requestId', 'kind', 'name', 'reason', 'fleetId', 'roomId', 'resourceIds', 'allocations']);
    const requestId = key(input.requestId), kind = input.kind;
    if (kind === 'contribution') fail(403, 'hagency_contribution_required', 'Resource contribution starts in Hagency. Request a project using published resources in Rinx.');
    if (kind !== 'project') fail(400, 'invalid_kind', 'Choose a project request.');
    if (!this.projectApprover) fail(503, 'project_approver_unconfigured', 'The server must designate one project approval administrator.');
    const payload = { name: text(input.name, 'Name', 128), reason: text(input.reason, 'Reason', 1000) };
    payload.fleetId = text(input.fleetId, 'Fleet', 80);
    if (input.roomId) payload.roomId = text(input.roomId, 'Room ID', 255);
    const id = `action_${hash({ actor, requestId }).slice(0, 32)}`;
    const fingerprint = hash({ kind, payload, resourceIds: input.resourceIds, allocations: input.allocations });
    const existing = this.state.records[id];
    if (existing) {
      if (existing.fingerprint !== fingerprint) fail(409, 'idempotency_conflict', 'This request ID has different content.');
      await this.projects.prepare(existing, actor, token);
      return this.get(id, actor, false);
    }
    const createdAt = this.now();
    const fleet = this.workflow.fleet(payload.fleetId);
    payload.allocations = this.projects.allocations(input.allocations, fleet, createdAt);
    const capabilities = await this.workflow.capabilities(fleet);
    if (!Array.isArray(input.resourceIds) || !input.resourceIds.length || input.resourceIds.length > 32
      || new Set(input.resourceIds).size !== input.resourceIds.length
      || input.resourceIds.some(id => !/^resource_[a-f0-9]{24}$/.test(id) || !capabilities.offers.some(offer => offer.resources?.some(r => r.id === id)))) fail(400, 'invalid_resources', 'Select currently offered resources.');
    payload.resourceIds = [...input.resourceIds].sort();
    if (canonical(payload.resourceIds) !== canonical(payload.allocations.map(a => a.resourceId))) fail(400, 'invalid_resources', 'Every requested resource requires its own finite contribution budget.');
    for (const allocation of payload.allocations) {
      const resource = capabilities.offers.flatMap(o => o.resources ?? []).find(r => r.id === allocation.resourceId);
      allocation.resourceName = typeof resource?.name === 'string' && resource.name.trim() ? resource.name.trim() : allocation.resourceId;
    }
    if (Object.keys(this.state.records).length >= this.maxRecords || Object.values(this.state.records).filter(row => row.ownerMxid === actor && row.state === 'requested').length >= 100) fail(429, 'inbox_full', 'Too many workflow records. Contact the server administrator.');
    this.store.atomic(() => {
      this.state.records[id] = { id, requestId, kind, payload, fingerprint, workflowVersion: 1, ownerMxid: actor, state: 'requested', execution: 'preparing', revision: 1, createdAt, updatedAt: createdAt };
      this.store.audit(actor, 'inbox.submit', payload.fleetId ?? null, id, 'preparing');
    });
    await this.projects.prepare(this.state.records[id], actor, token);
    return this.get(id, actor, false);
  }
  async decide(input, actor, token) {
    if (this.agents.manages(this.state.records[input?.id])) return this.agents.decide(input, actor, token);
    fields(input, ['id', 'expectedRevision', 'commandId', 'decision', 'reason', 'administrators', 'allowSelfApproval']);
    await this.service.palpo.requireAdmin(token);
    const identity = await this.service.palpo.call('/_matrix/client/v3/account/whoami', token);
    if (identity.user_id !== actor || !this.canApproveProjects(actor, !identity.is_guest)) fail(403, 'project_approver_required', 'Only the designated Palpo administrator can approve or reject projects.');
    const row = this.record(input.id, actor, true), commandId = key(input.commandId);
    if (row.kind !== 'project') fail(403, 'hagency_contribution_required', 'Manage resource contribution in Hagency.');
    if (!['approve', 'reject'].includes(input.decision) || !Number.isSafeInteger(input.expectedRevision)) fail(400, 'invalid_decision', 'A decision and the reviewed revision are required.');
    const reason = text(input.reason, 'Decision reason', 1000);
    const command = { commandId, actor, expectedRevision: input.expectedRevision, decision: input.decision, reason,
      ...(input.decision === 'approve' ? { administrators: this.projects.administrators(input.administrators), allowSelfApproval: input.allowSelfApproval ?? null } : {}) };
    if (row.command) {
      if (canonical(row.command) !== canonical(command)) fail(409, 'decision_conflict', 'This action has already changed. Refresh its latest result.');
    } else {
      if (row.state !== 'requested' || row.revision !== input.expectedRevision) fail(409, 'decision_conflict', 'This action has already changed. Refresh its latest result.');
      const plan = input.decision === 'approve' ? await this.projects.plan(row, input, token) : null;
      this.store.atomic(() => {
        row.command = command; row.state = input.decision === 'approve' ? 'approved' : 'rejected'; row.revision++;
        row.updatedAt = this.now(); row.decision = { by: actor, at: this.now(), reason };
        if (plan) this.projects.enqueue(row, actor, plan);
        else if (row.result?.projectId && this.store.state.projects[row.result.projectId]) this.store.state.projects[row.result.projectId].state = 'rejected';
        this.notify(row); this.store.audit(actor, 'inbox.decide', row.payload.fleetId ?? null, row.id, row.state);
      });
    }
    return this.get(row.id, actor, true);
  }
  async recover(input, actor, token) {
    fields(input, ['id', 'expectedRevision', 'commandId', 'operation', 'reason']);
    await this.service.palpo.requireAdmin(token);
    const identity = await this.service.palpo.call('/_matrix/client/v3/account/whoami', token);
    if (identity.user_id !== actor || !this.canApproveProjects(actor, !identity.is_guest)) fail(403, 'project_approver_required', 'Only the designated Palpo administrator can recover a failed project allocation.');
    if (!['retry', 'release'].includes(input.operation) || !Number.isSafeInteger(input.expectedRevision)) fail(400, 'invalid_recovery', 'Review the current revision and choose retry or release.');
    const row = this.record(input.id, actor, true);
    await this.projects.recover(row, { commandId: key(input.commandId), actor, expectedRevision: input.expectedRevision,
      operation: input.operation, reason: text(input.reason, 'Recovery reason', 1000) }, token);
    return this.get(row.id, actor, true);
  }
  async activate(input, actor, token) {
    fields(input, ['id']);
    // Activation always uses the project owner's current Matrix authority.
    const row = this.record(input.id, actor, false);
    if (row.kind !== 'project') fail(403, 'hagency_contribution_required', 'Manage resource contribution in Hagency.');
    if (row.workflowVersion === 1) {
      await this.projects.prepare(row, actor, token);
      return this.get(row.id, actor, false);
    }
    if (row.state !== 'approved') fail(409, 'not_approved', 'This project has not been approved.');
    if (row.execution === 'done') return this.get(row.id, actor, false);
    try {
      const project = await this.workflow.createProject({ requestId: row.id, name: row.payload.name, fleetId: row.payload.fleetId, roomId: row.payload.roomId }, actor, token);
      this.store.atomic(() => {
        row.result = { projectId: project.id, roomId: project.roomId }; row.execution = 'done'; row.lastError = null; row.updatedAt = this.now(); row.revision++;
        this.notify(row); this.store.audit(actor, 'inbox.activate', row.payload.fleetId, row.id, 'done');
      });
      return this.get(row.id, actor, false);
    } catch (cause) {
      this.store.atomic(() => { row.execution = 'failed'; row.lastError = { code: cause.code ?? 'internal_error' }; row.updatedAt = this.now(); });
      throw cause;
    }
  }
  projectGrant(input, actor, existing, { proposal = false } = {}) {
    const row = this.state.records[input.requestId];
    if (!row) {
      if (this.requireProjectApproval && !existing) fail(403, 'project_approval_required', 'Request and approve a project in Palpo before creating it.');
      return null;
    }
    if (row.kind !== 'project' || (proposal ? row.workflowVersion !== 1 || row.state !== 'requested' || row.execution !== 'preparing' : row.state !== 'approved') || row.ownerMxid !== actor
      || row.payload.name !== input.name || row.payload.fleetId !== input.fleetId || (row.payload.roomId ?? null) !== (input.roomId || null)) fail(403, 'project_grant_mismatch', 'Project creation must match the approved owner and resources.');
    if (row.workflowVersion === 1) {
      if (!proposal && row.execution !== 'done') fail(409, 'project_reservation_pending', 'Hagency has not confirmed this project allocation.');
      return existing?.resourceGrant ?? { v: 1, actionId: row.id, resourceIds: [...row.payload.resourceIds], grantIds: [] };
    }
    return { actionId: row.id, resourceIds: [...row.payload.resourceIds] };
  }
  resourceGrant(project, resource, existing) {
    // Existing legacy operations may retry their exact source-bound payload;
    // this never allocates a new grant or changes their Hagency verdict.
    if (existing && project.resourceGrant?.v !== 1) return;
    if (!this.projects.allocation(project).ready) fail(409, 'project_allocation_required', 'This project needs a current, accepted finite allocation before requesting another agent.');
    const row = this.state.records[project.resourceGrant.actionId];
    if (!row || row.state !== 'approved' || !project.resourceGrant.resourceIds.includes(resource)) fail(403, 'resource_not_granted', 'Select a resource approved for this project.');
  }
  projectReceipt(entry) { this.projects.receipt(entry); this.agents.receipt(entry); this.lifecycle.receipt(entry); }
  seen(input, actor, admin) {
    fields(input, ['id']); const row = this.record(input.id, actor, admin);
    this.store.atomic(() => { for (const notice of Object.values(this.state.notices)) if (notice.actionId === row.id && notice.recipient === actor) notice.seenAt = this.now(); });
    return this.get(row.id, actor, admin);
  }
  snooze(input, actor, admin) {
    fields(input, ['id', 'minutes']); const row = this.record(input.id, actor, admin);
    if (!this.pending(row, actor, admin) || !Number.isSafeInteger(input.minutes) || input.minutes < 1 || input.minutes > 1440) fail(400, 'invalid_snooze', 'Snooze a pending action for 1 to 1440 minutes.');
    this.store.atomic(() => { for (const notice of Object.values(this.state.notices)) if (notice.actionId === row.id && notice.revision === row.revision && notice.recipient === actor) { notice.snoozedUntil = this.now() + input.minutes * 60000; notice.dueAt = notice.snoozedUntil; notice.finished = false; } });
    return { snoozedUntil: this.now() + input.minutes * 60000, action: this.view(row, actor, admin) };
  }
}
