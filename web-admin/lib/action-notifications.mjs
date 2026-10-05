import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { isQuietAt } from './notification-preferences.mjs';

const hash = value => createHash('sha256').update(value).digest('hex');
const enc = encodeURIComponent;
const fail = (message) => { throw new ApiError(409, 'action_room_not_private', message); };

// Delivery is a projection of Inbox. No event handler can approve an action.
// A lost Matrix reply repeats the same transaction; reminders use new ones.
export class ActionNotifications {
  constructor(inbox, config, { now = Date.now, intervals = [3600000, 86400000, 172800000] } = {}) {
    Object.assign(this, { inbox, config, now, intervals });
    this.service = inbox.service; this.palpo = this.service.palpo;
    if (config) {
      this.service.owner(config.botMxid);
      const origin = new URL(config.homeserverOrigin);
      if (origin.protocol !== 'https:' || origin.username || origin.password || origin.pathname !== '/' || origin.search || origin.hash) throw new Error('Action notifications require the public HTTPS homeserver origin.');
      if (!config.botToken || !config.adminToken || !Array.isArray(config.approvers) || !config.approvers.length
        || config.approvers.includes(config.botMxid)) throw new Error('Invalid action notification configuration.');
      config.approvers.forEach(user => this.service.owner(user));
    }
  }
  async isAdmin(actor) {
    const user = await this.palpo.user(actor, this.config.adminToken);
    return !!user?.admin && !user.deactivated && !user.locked && !user.appservice_id;
  }
  async room(actor) {
    const { botMxid, botToken } = this.config;
    const identity = await this.palpo.call('/_matrix/client/v3/account/whoami', botToken);
    if (identity.user_id !== botMxid || identity.is_guest) fail('The notification bot identity could not be verified.');
    let binding = this.inbox.state.rooms[actor];
    const expected = { v: 1, purpose: 'my_actions', ownerMxid: actor, botMxid, serverName: this.service.serverName };
    if (binding && binding.botMxid !== botMxid) fail('The saved My Actions room belongs to a different bot.');
    if (!binding) {
      const alias = `palpo_actions_${hash(JSON.stringify(expected)).slice(0, 24)}`;
      let roomId;
      try { roomId = (await this.palpo.call(`/_matrix/client/v3/directory/room/${enc('#' + alias + ':' + this.service.serverName)}`, botToken)).room_id; }
      catch (cause) { if (cause.status !== 404) throw cause; }
      if (!roomId) {
        const result = await this.palpo.call('/_matrix/client/v3/createRoom', botToken, { method: 'POST', body: {
          name: 'My Actions', room_alias_name: alias, visibility: 'private', preset: 'private_chat',
          creation_content: { 'm.federate': false }, invite: [actor],
          power_level_content_override: { users: { [botMxid]: 100 }, users_default: 0, invite: 100, state_default: 100, events_default: 0 },
          initial_state: [
            { type: 'im.palpo.actions.v1', state_key: '', content: expected },
            { type: 'm.room.history_visibility', state_key: '', content: { history_visibility: 'invited' } },
          ],
        } });
        roomId = result.room_id;
      }
      if (typeof roomId !== 'string') fail('Matrix did not return an action room.');
      binding = { ...expected, roomId };
      // The alias allows recovery after a crash between createRoom and this save.
      this.inbox.store.atomic(() => { this.inbox.state.rooms[actor] = binding; });
    }
    const state = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(binding.roomId)}/state`, botToken);
    if (!Array.isArray(state)) fail('Action room state is unavailable.');
    const get = (type, key = '') => state.find(e => e.type === type && e.state_key === key)?.content;
    const marker = get('im.palpo.actions.v1');
    const powers = get('m.room.power_levels');
    if (Object.keys(expected).some(key => marker?.[key] !== expected[key])
      || get('m.room.join_rules')?.join_rule !== 'invite' || get('m.room.history_visibility')?.history_visibility !== 'invited'
      || get('m.room.encryption') || get('m.room.create')?.['m.federate'] !== false
      || powers?.invite !== 100 || powers?.state_default !== 100 || powers?.users?.[botMxid] !== 100
      || (powers?.users_default ?? 0) !== 0
      || Object.entries(powers?.users ?? {}).some(([user, level]) => user !== botMxid && level !== 0)
      || ['m.room.power_levels', 'm.room.join_rules', 'm.room.history_visibility', 'm.room.encryption', 'im.palpo.actions.v1']
        .some(type => (powers?.events?.[type] ?? powers?.state_default) !== 100)
      || get('m.room.member', botMxid)?.membership !== 'join'
      || !['join', 'invite'].includes(get('m.room.member', actor)?.membership)
      || state.some(e => e.type === 'm.room.member' && ['join', 'invite'].includes(e.content?.membership) && ![actor, botMxid].includes(e.state_key))) fail('Restore the private My Actions room settings and membership. Pending actions remain in the Inbox.');
    return binding.roomId;
  }
  async tick() {
    if (!this.config) return;
    await this.service.palpo.requireAdmin(this.config.adminToken);
    const at = this.now(), settings = new Map();
    const due = Object.values(this.inbox.state.notices).filter(n => {
      if (n.cancelled || n.finished || n.dueAt > at || (n.snoozedUntil ?? 0) > at) return false;
      if (!settings.has(n.recipient)) {
        const prefs = this.inbox.preferences.get(n.recipient);
        settings.set(n.recipient, { ...prefs, quiet: isQuietAt(prefs.quietHours, at) });
      }
      const prefs = settings.get(n.recipient);
      return prefs.enabled && !prefs.quiet && (n.delivered === 0 || prefs.remindersEnabled);
    }).sort((a, b) => a.dueAt - b.dueAt || a.id.localeCompare(b.id)).slice(0, 20);
    for (const notice of due) {
      try {
        const row = this.inbox.state.records[notice.actionId];
        const admin = this.inbox.canApproveProjects(notice.recipient, true) && await this.isAdmin(notice.recipient);
        // Recheck canonical state and recipient authority just before sending.
        if (!row || row.revision !== notice.revision || !this.inbox.canRead(row, notice.recipient, admin)
          || (notice.delivered > 0 && !this.inbox.pending(row, notice.recipient, admin))) {
          this.inbox.store.atomic(() => { notice.cancelled = true; }); continue;
        }
        if (this.inbox.agents.manages(row)) {
          const person = await this.palpo.user(notice.recipient, this.config.adminToken);
          if (!person || person.deactivated || person.locked || person.appservice_id) {
            this.inbox.store.atomic(() => { notice.cancelled = true; }); continue;
          }
        }
        const roomId = await this.room(notice.recipient);
        // Room/authority checks await Matrix; a quiet-hours boundary may have
        // passed since this tick selected its batch.
        const prefs = this.inbox.preferences.get(notice.recipient);
        if (!prefs.enabled || (notice.delivered > 0 && !prefs.remindersEnabled) || isQuietAt(prefs.quietHours, this.now())) continue;
        const pending = this.inbox.pending(row, notice.recipient, admin);
        const intervals = this.inbox.preferences.intervals(notice.recipient, this.intervals);
        // Persist the exact envelope before attempting delivery. An ambiguous
        // response retry cannot change its body or transaction across a restart.
        if (!notice.delivery) {
          const label = pending ? 'A Palpo action needs your attention.' : 'A Palpo action has an update.';
          const route = `${new URL(this.config.homeserverOrigin).origin}/_palpo/miniapp/action/${row.id}`;
          this.inbox.store.atomic(() => {
            notice.delivery = { roomId, transactionId: notice.id + '_' + notice.delivered, content: {
              msgtype: pending ? 'm.text' : 'm.notice', body: `${label}\nOpen in Rinx: ${route}`,
              format: 'org.matrix.custom.html', formatted_body: `${label} <a href="${route}">Open action</a>`,
              'm.mentions': { user_ids: pending ? [notice.recipient] : [] },
              'im.palpo.action.v1': { v: 1, id: row.id, revision: row.revision, appId: 'im.palpo.operations', ownerMxid: notice.recipient, serverName: this.service.serverName },
            } };
          });
        }
        if (notice.delivery.roomId !== roomId) fail('The action room changed during a pending delivery.');
        const result = await this.palpo.call(`/_matrix/client/v3/rooms/${enc(roomId)}/send/m.room.message/${enc(notice.delivery.transactionId)}`,
          this.config.botToken, { method: 'PUT', body: notice.delivery.content });
        if (typeof result.event_id !== 'string') throw new Error('No Matrix receipt');
        this.inbox.store.atomic(() => {
          notice.eventId = result.event_id; notice.roomId = roomId; notice.delivered++; notice.attempt = 0; notice.lastError = null;
          notice.lastDeliveredAt = this.now(); notice.snoozedUntil = null; notice.delivery = null;
          // One delivery covers every cadence point missed during downtime,
          // retries, snooze or quiet hours. Never replay an overdue burst.
          notice.reminderCursor = Math.max(notice.reminderCursor ?? 0,
            intervals.filter(delay => notice.createdAt + delay <= this.now()).length);
          notice.finished = !pending || notice.reminderCursor >= intervals.length;
          notice.dueAt = notice.createdAt + (intervals[notice.reminderCursor] ?? 0);
        });
      } catch (cause) {
        this.inbox.store.atomic(() => {
          notice.attempt++; notice.lastError = cause.code ?? 'delivery_failed';
          notice.dueAt = this.now() + Math.min(3600000, 1000 * 2 ** Math.min(notice.attempt, 12));
        });
      }
    }
  }
  start() {
    if (!this.config || this.timer) return;
    const run = () => {
      if (!this.running) this.running = this.service.serial(() => this.tick()).catch(() => {}).finally(() => { this.running = null; });
    };
    run(); this.timer = setInterval(run, 30000); this.timer.unref();
  }
  async stop() { clearInterval(this.timer); this.timer = null; await this.running; }
}
