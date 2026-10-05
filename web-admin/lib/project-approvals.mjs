import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical, outboundProven } from './outbound.mjs';
import { contributionKey } from './project-contributions.mjs';
import { validateLimits } from './project-commands.mjs';

const hash = v => createHash('sha256').update(canonical(v)).digest('hex');
const fail = (status, code, message) => { throw new ApiError(status, code, message); };
const closed = (v, keys) => v && typeof v === 'object' && !Array.isArray(v) && Object.keys(v).length === keys.length && keys.every(k => Object.hasOwn(v, k));
const dimensions = ['tokens', 'maxAgents', 'maxRatePerDay'];
const positive = v => Number.isSafeInteger(v) && v > 0;
const numericInput = v => typeof v === 'number' ? v : typeof v === 'string' && /^[1-9][0-9]*$/.test(v) ? Number(v) : NaN;

// Business authority is separate from Matrix room creation. Preparing the
// owner's room confers no resource grant and cannot admit an agent.
export class ProjectApprovals {
  constructor(inbox) { this.inbox = inbox; this.service = inbox.service; this.store = inbox.store; this.workflow = inbox.workflow; }
  get commands() {
    if (!this.inbox.projectCommands) fail(503, 'project_workflow_unavailable', 'Project reservations are not configured.');
    return this.inbox.projectCommands;
  }
  contribution(fleet, id) {
    this.commands.requireSupport(fleet);
    const row = this.commands.state.contributions[contributionKey(fleet.id, fleet.projectWorkflow.registrationGeneration, id)];
    const now = this.inbox.now(), received = row?.receivedAtMs, observed = row?.observedAtMs;
    if (!outboundProven(fleet) || !row || row.transportGeneration !== fleet.transport.generation
      || fleet.contributionPublication?.registrationGeneration !== fleet.projectWorkflow.registrationGeneration
      || row.state !== 'active' || row.grant.expiresAtMs <= now
      || !Number.isSafeInteger(observed) || !Number.isSafeInteger(received) || observed > now + 5000
      || now - Math.min(observed, received) >= 90000) fail(409, 'contribution_unavailable', 'Waiting for a current, active contribution from this Hagency.');
    return row;
  }
  available(fleet, row, excludeAction = null) {
    const accepted = Object.values(this.commands.state.grants).filter(r => r.fleetId === fleet.id
      && r.state !== 'released' && r.registrationGeneration === fleet.projectWorkflow.registrationGeneration && r.grant.delegationId === row.grant.id);
    const pending = Object.values(this.commands.state.commands).filter(e => !e.receipt && !e.cancelled && e.actionId !== excludeAction
      && e.command.fleetId === fleet.id && e.command.registrationGeneration === fleet.projectWorkflow.registrationGeneration
      && e.command.operation.kind === 'reserve_project' && e.command.operation.grant.delegationId === row.grant.id);
    return Object.fromEntries(dimensions.map(k => {
      const used = Math.max(row.reserved[k] - (row.released?.[k] ?? 0), accepted.reduce((n, r) => n + r.grant.limits[k], 0));
      const queued = pending.reduce((n, e) => n + e.command.operation.grant.limits[k], 0);
      return [k, Math.max(0, row.grant.limits[k] - used - queued)];
    }));
  }
  catalog(fleet, resourceId) {
    if (!this.inbox.projectCommands || fleet.projectWorkflow?.v !== 1) return [];
    const rows = [];
    for (const record of Object.values(this.commands.state.contributions)) {
      if (record.grant.fleetId !== fleet.id || record.grant.resourceId !== resourceId || record.grant.registrationGeneration !== fleet.projectWorkflow.registrationGeneration) continue;
      try {
        const row = this.contribution(fleet, record.grant.id), remaining = this.available(fleet, row);
        if (dimensions.every(k => remaining[k] > 0)) rows.push({ id: row.grant.id, revision: row.grant.revision,
          remaining, expiresAtMs: row.grant.expiresAtMs, observedAtMs: row.observedAtMs });
      } catch (error) { if (!(error instanceof ApiError)) throw error; }
    }
    return rows.slice(0, 32);
  }
  allocations(input, fleet, createdAt) {
    if (!Array.isArray(input) || !input.length || input.length > 32) fail(400, 'project_budget_required', 'Select finite project budgets.');
    const seen = new Set();
    return input.map(value => {
      if (!closed(value, ['contributionId', 'resourceId', 'limits', 'durationHours']) || seen.has(value.resourceId)) fail(400, 'invalid_project_budget', 'Select each resource once with its contribution and limits.');
      const limits = Object.fromEntries(dimensions.map(k => [k, numericInput(value.limits?.[k])]));
      const hours = numericInput(value.durationHours), expiresAtMs = createdAt + hours * 3600000;
      if (!closed(value.limits, dimensions) || !validateLimits(limits) || !positive(hours) || hours > 8760 || !positive(expiresAtMs)) fail(400, 'invalid_project_budget', 'Use positive whole-number limits and a duration of 1–8760 hours.');
      const row = this.contribution(fleet, value.contributionId);
      if (row.grant.resourceId !== value.resourceId || expiresAtMs > row.grant.expiresAtMs) fail(409, 'contribution_scope_conflict', 'The project must fit the selected contribution and its expiry.');
      if (!dimensions.every(k => limits[k] <= this.available(fleet, row)[k])) fail(409, 'project_capacity_unavailable', 'The requested limits exceed this contribution’s available capacity.');
      seen.add(value.resourceId);
      return { contributionId: row.grant.id, contributionRevision: row.grant.revision, resourceId: value.resourceId,
        limits, durationHours: hours, expiresAtMs, registrationGeneration: row.grant.registrationGeneration };
    }).sort((a, b) => a.resourceId.localeCompare(b.resourceId));
  }
  async prepare(row, actor, token) {
    if (row.execution !== 'preparing') return;
    try {
      const project = await this.workflow.createProject({ requestId: row.id, name: row.payload.name, fleetId: row.payload.fleetId, roomId: row.payload.roomId }, actor, token, { proposal: true });
      this.store.atomic(() => {
        row.result = { projectId: project.id, roomId: project.roomId }; row.execution = 'pending'; row.lastError = null;
        row.updatedAt = this.inbox.now(); row.revision++; this.inbox.notify(row);
        this.store.audit(actor, 'inbox.prepare_project', row.payload.fleetId, row.id, 'awaiting_approval');
      });
    } catch (cause) {
      this.store.atomic(() => { row.lastError = { code: cause.code ?? 'internal_error' }; row.updatedAt = this.inbox.now(); });
      throw cause;
    }
  }
  administrators(value) {
    const list = typeof value === 'string' ? value.trim().split(/[\s,]+/) : value;
    if (!Array.isArray(list) || !list.length || list.length > 16 || new Set(list).size !== list.length) fail(400, 'project_administrators_required', 'Assign one to sixteen distinct Matrix users.');
    return list.map(mxid => this.service.owner(mxid)).sort();
  }
  async plan(row, input, token) {
    if (row.workflowVersion !== 1 || row.execution !== 'pending' || !row.result?.roomId) fail(409, 'project_preparation_required', 'The owner must finish preparing a budgeted project request.');
    if (typeof input.allowSelfApproval !== 'boolean') fail(400, 'project_administrators_required', 'Choose an explicit self-approval policy.');
    const administrators = this.administrators(input.administrators);
    for (const mxid of administrators) {
      const account = await this.service.palpo.user(mxid, token);
      if (!account || account.deactivated || account.locked || account.appservice_id) fail(409, 'project_administrator_unavailable', 'Each assigned administrator must be an active person on this Matrix server.');
    }
    const fleet = this.workflow.fleet(row.payload.fleetId), project = this.store.state.projects[row.result.projectId];
    if (!project || project.ownerMxid !== row.ownerMxid || project.roomId !== row.result.roomId || project.fleetId !== fleet.id) fail(409, 'project_binding_conflict', 'The prepared project binding changed.');
    await this.validateBinding(project, fleet);
    const grants = row.payload.allocations.map(selection => {
      const parent = this.contribution(fleet, selection.contributionId), available = this.available(fleet, parent, row.id);
      if (parent.grant.revision !== selection.contributionRevision || parent.grant.registrationGeneration !== selection.registrationGeneration
        || selection.expiresAtMs <= this.inbox.now() || selection.expiresAtMs > parent.grant.expiresAtMs
        || !dimensions.every(k => selection.limits[k] <= available[k])) fail(409, 'project_capacity_unavailable', 'The reviewed budget is no longer available. Refresh and request a revised allocation.');
      return { v: 1, id: `grant_${hash({ action: row.id, resource: selection.resourceId }).slice(0, 32)}`, revision: 1,
        delegationId: selection.contributionId, delegationRevision: selection.contributionRevision,
        projectId: project.id, roomId: project.roomId, ownerMxid: project.ownerMxid,
        administratorMxids: administrators, allowSelfApproval: input.allowSelfApproval,
        limits: selection.limits, expiresAtMs: selection.expiresAtMs };
    });
    await this.service.palpo.requireAdmin(token);
    return { fleet, project, grants, administrators };
  }
  async validateBinding(project, fleet) {
    const state = await this.workflow.roomState(project.roomId, fleet.registration.as_token, fleet.representativeMxid);
    const content = (type, key = '') => state.find(e => e.type === type && e.state_key === key)?.content;
    const binding = content('com.hagency.admin.binding.v1', fleet.id), powers = content('m.room.power_levels');
    if (content('m.room.join_rules')?.join_rule !== 'invite' || content('m.room.encryption')
      || content('m.room.member', project.ownerMxid)?.membership !== 'join'
      || Number(powers?.users?.[project.ownerMxid] ?? powers?.users_default ?? 0) < Math.max(100, Number(powers?.state_default ?? 50), Number(powers?.invite ?? 0))
      || binding?.v !== 1 || binding.fleetId !== fleet.id || binding.projectId !== project.id
      || binding.ownerMxid !== project.ownerMxid || binding.purpose !== 'project' || binding.authVersion !== project.authVersion) fail(409, 'project_binding_conflict', 'The owner, privacy, or project room binding changed after submission.');
  }
  enqueue(row, actor, plan) {
    row.execution = 'awaiting_reservation'; row.reservations = [];
    for (const grant of plan.grants) {
      const commandId = `reserve_${hash({ action: row.id, grant: grant.id }).slice(0, 40)}`;
      this.commands.enqueue(plan.fleet, actor, { kind: 'reserve_project', grant }, {
        commandId, projectId: plan.project.id, actionId: row.id, expiresAtMs: Math.min(grant.expiresAtMs, this.inbox.now() + 7 * 86400000),
      });
      row.reservations.push({ grantId: grant.id, commandId, state: 'queued' });
    }
    plan.project.state = 'awaiting_reservation';
    plan.project.resourceGrant = { v: 1, actionId: row.id, resourceIds: [...row.payload.resourceIds], grantIds: plan.grants.map(g => g.id) };
  }
  recoveryOptions(row, actor, admin) {
    const permitted = row.kind === 'project' && row.workflowVersion === 1 && row.state === 'approved' && this.inbox.canApproveProjects(actor, admin);
    const failed = permitted && row.execution === 'reservation_refused' && row.reservations?.length > 0
      && row.reservations.every(r => ['applied', 'refused'].includes(r.state));
    const releaseFailed = permitted && row.execution === 'release_refused' && row.releases?.length > 0
      && row.releases.every(r => ['applied', 'refused'].includes(r.state));
    const fleet = this.store.state.fleets[row.payload?.fleetId];
    return { canRetryReservation: !!failed && row.payload.allocations.every(a => a.expiresAtMs > this.inbox.now()),
      canReleaseReservation: !!(failed || releaseFailed) && (fleet?.projectWorkflow?.unusedRelease === true || row.reservations.every(r => r.state === 'refused')) };
  }
  authorizesRecovery(entry) {
    const row = this.inbox.state.records[entry.actionId], recovery = row?.recovery;
    if (entry.actionKind !== 'project_recovery' || !recovery || row.state !== 'approved'
      || recovery.commandId !== entry.recoveryId || recovery.actor !== entry.command.actorMxid
      || row.result?.projectId !== entry.projectId) return false;
    return recovery.operation === 'retry' ? ['awaiting_reservation', 'reservation_refused'].includes(row.execution)
      && row.reservations.some(r => r.commandId === entry.command.commandId)
      : ['releasing_reservations', 'release_refused'].includes(row.execution)
        && row.releases.some(r => r.commandId === entry.command.commandId);
  }
  async recover(row, command, token) {
    const recoveryKey = hash(command.commandId), prior = row.recoveryHistory?.[recoveryKey];
    if (prior) {
      if (canonical(prior) !== canonical(command)) fail(409, 'recovery_conflict', 'This recovery command already has different content.');
      return;
    }
    const options = this.recoveryOptions(row, command.actor, true);
    if (row.revision !== command.expectedRevision || !(command.operation === 'retry' ? options.canRetryReservation : options.canReleaseReservation))
      fail(409, 'recovery_conflict', 'Wait for all reservation results, then review the latest failed allocation.');
    if (Object.keys(row.recoveryHistory ?? {}).length >= 64) fail(409, 'recovery_limit', 'This allocation needs operator review after repeated recovery attempts.');
    const fleet = this.workflow.fleet(row.payload.fleetId), project = this.store.state.projects[row.result.projectId];
    this.commands.requireSupport(fleet);
    const plan = [];
    if (command.operation === 'retry') {
      await this.validateBinding(project, fleet);
      for (const reservation of row.reservations.filter(r => r.state === 'refused')) {
        const original = this.commands.entry(fleet, reservation.commandId), grant = original?.command.operation.grant;
        if (!grant || original.state !== 'refused') fail(409, 'recovery_conflict', 'The original reservation result is unavailable in this registration.');
        const parent = this.contribution(fleet, grant.delegationId), available = this.available(fleet, parent);
        if (grant.delegationRevision !== parent.grant.revision || grant.expiresAtMs <= this.inbox.now()
          || grant.expiresAtMs > parent.grant.expiresAtMs || !dimensions.every(k => grant.limits[k] <= available[k]))
          fail(409, 'project_capacity_unavailable', 'The original approved budget cannot be retried. Release this failed allocation and submit a revised request.');
        for (const mxid of grant.administratorMxids) {
          const account = await this.service.palpo.user(mxid, token);
          if (!account || account.deactivated || account.locked || account.appservice_id) fail(409, 'project_administrator_unavailable', 'The original administrator assignment is no longer active.');
        }
        plan.push({ reservation, operation: { kind: 'reserve_project', grant }, expiresAtMs: Math.min(grant.expiresAtMs, this.inbox.now() + 7 * 86400000) });
      }
    } else {
      for (const reservation of row.reservations.filter(r => r.state === 'applied')) {
        const record = this.commands.state.grants[reservation.grantId];
        if (!record || record.fleetId !== fleet.id || record.registrationGeneration !== fleet.projectWorkflow.registrationGeneration
          || record.grant.projectId !== project.id) fail(409, 'recovery_conflict', 'The original accepted grant is unavailable in this registration.');
        if (record.state === 'released') continue;
        this.commands.requireUnusedRelease(fleet);
        plan.push({ reservation, operation: { kind: 'release_unused_project', grantId: record.grant.id, expectedRevision: record.grant.revision }, expiresAtMs: this.inbox.now() + 7 * 86400000 });
      }
    }
    await this.service.palpo.requireAdmin(token);
    const reviewer = await this.service.palpo.user(command.actor, token);
    if (!reviewer?.admin || reviewer.deactivated || reviewer.locked || reviewer.is_guest || reviewer.appservice_id
      || !this.inbox.canApproveProjects(command.actor, true)) fail(403, 'project_approver_required', 'The designated administrator is no longer active.');
    if (row.revision !== command.expectedRevision) fail(409, 'recovery_conflict', 'This action changed while preparing recovery.');
    for (const item of plan) if (item.operation.kind === 'reserve_project') {
      const grant = item.operation.grant, parent = this.contribution(fleet, grant.delegationId), available = this.available(fleet, parent);
      if (grant.expiresAtMs <= this.inbox.now() || grant.delegationRevision !== parent.grant.revision
        || !dimensions.every(k => grant.limits[k] <= available[k])) fail(409, 'project_capacity_unavailable', 'The original approved capacity changed during recovery.');
    }
    this.store.atomic(() => {
      row.recoveryHistory ??= {}; row.recoveryHistory[recoveryKey] = command; row.recovery = command;
      if (command.operation === 'release') row.releases = [];
      for (const item of plan) {
        const commandId = `recover_${hash({ action: row.id, command, grant: item.reservation.grantId }).slice(0, 40)}`;
        const entry = this.commands.enqueue(fleet, command.actor, item.operation, { commandId, actionId: row.id, projectId: project.id, expiresAtMs: item.expiresAtMs });
        entry.actionKind = 'project_recovery'; entry.recoveryId = command.commandId;
        if (command.operation === 'retry') Object.assign(item.reservation, { commandId, state: 'queued', code: null });
        else row.releases.push({ grantId: item.reservation.grantId, commandId, state: 'queued', code: null });
      }
      row.execution = command.operation === 'retry' ? 'awaiting_reservation' : plan.length ? 'releasing_reservations' : 'released';
      if (row.execution === 'released') row.state = 'cancelled';
      project.state = row.execution;
      row.updatedAt = this.inbox.now(); row.revision++;
      this.inbox.notify(row); this.store.audit(command.actor, 'inbox.recover_project', fleet.id, row.id, row.execution);
    });
  }
  receipt(entry) {
    if (entry.command.operation.kind === 'release_unused_project') {
      const row = this.inbox.state.records[entry.actionId];
      const release = row?.releases?.find(r => r.commandId === entry.command.commandId);
      if (!release || row.result?.projectId !== entry.projectId || row.state !== 'approved') return;
      release.state = entry.state; release.code = entry.receipt.outcome.code ?? null;
      row.execution = row.releases.some(r => r.state === 'refused') ? 'release_refused' : row.releases.every(r => r.state === 'applied') ? 'released' : 'releasing_reservations';
      if (row.execution === 'released') row.state = 'cancelled';
      const project = this.store.state.projects[entry.projectId]; if (project) project.state = row.execution;
      row.updatedAt = this.inbox.now(); row.revision++; this.inbox.notify(row);
      return;
    }
    if (entry.command.operation.kind !== 'reserve_project') return;
    const row = this.inbox.state.records[entry.actionId];
    if (!row || row.workflowVersion !== 1 || row.result?.projectId !== entry.projectId) return;
    const reservation = row.reservations?.find(r => r.commandId === entry.command.commandId);
    if (!reservation) return;
    reservation.state = entry.state; reservation.code = entry.receipt.outcome.code ?? null;
    const refused = row.reservations.some(r => r.state === 'refused');
    row.execution = refused ? 'reservation_refused' : row.reservations.every(r => r.state === 'applied') ? 'done' : 'awaiting_reservation';
    row.updatedAt = this.inbox.now(); row.revision++;
    const project = this.store.state.projects[entry.projectId];
    if (project) project.state = row.execution === 'done' ? 'registered' : row.execution;
    this.inbox.notify(row);
  }
  allocation(project) {
    const descriptor = project.resourceGrant, row = this.inbox.state.records[descriptor?.actionId];
    if (descriptor?.v !== 1 || !row || row.workflowVersion !== 1) return { state: 'migration_required', ready: false, grants: [] };
    if (row.execution !== 'done') return { state: row.execution, ready: false, grants: [] };
    const fleet = this.store.state.fleets[project.fleetId];
    const grants = descriptor.grantIds.map(id => this.commands.state.grants[id]);
    const valid = grants.length === row.payload.allocations.length && grants.length > 0 && grants.every(r => r?.state === 'accepted' && r.fleetId === project.fleetId
      && r.registrationGeneration === fleet?.projectWorkflow?.registrationGeneration && r.grant.expiresAtMs > this.inbox.now()
      && r.grant.projectId === project.id && r.grant.ownerMxid === project.ownerMxid && r.grant.roomId === project.roomId
      && this.commands.state.contributions[contributionKey(fleet.id, r.registrationGeneration, r.grant.delegationId)]?.state === 'active');
    return { state: valid ? 'allocated' : 'unavailable', ready: valid, grants: grants.map(r => r?.grant).filter(Boolean) };
  }
}
