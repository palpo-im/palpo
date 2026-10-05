import { createHash } from 'node:crypto';
import { ApiError } from './service.mjs';
import { canonical } from './outbound.mjs';
import { fields } from './miniapp.mjs';

const defaults = { enabled: true, remindersEnabled: true, reminderMinutes: [60, 1440, 2880], quietHours: null };
const fail = (code, message, status = 400) => { throw new ApiError(status, code, message); };
const time = value => typeof value === 'string' && /^([01][0-9]|2[0-3]):[0-5][0-9]$/.test(value);
const minutes = value => Number(value.slice(0, 2)) * 60 + Number(value.slice(3));
const formatters = new Map();
function formatter(zone) {
  if (!formatters.has(zone)) {
    if (formatters.size >= 64) formatters.delete(formatters.keys().next().value);
    formatters.set(zone, new Intl.DateTimeFormat('en-GB', { timeZone: zone, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }));
  }
  return formatters.get(zone);
}
export function isQuietAt(quiet, at) {
  if (!quiet) return false;
  const parts = formatter(quiet.timeZone).formatToParts(at);
  const local = Number(parts.find(p => p.type === 'hour').value) * 60 + Number(parts.find(p => p.type === 'minute').value);
  const start = minutes(quiet.start), end = minutes(quiet.end);
  return start < end ? local >= start && local < end : local >= start || local < end;
}

// Preferences belong to the authenticated account, independently of each app
// session and action. Quiet hours use local wall time, including DST transitions.
export class NotificationPreferences {
  constructor(inbox) { this.inbox = inbox; inbox.state.preferences ??= {}; }
  get(actor) {
    const row = this.inbox.state.preferences[actor];
    return { revision: row?.revision ?? 0, ...structuredClone(row?.value ?? defaults) };
  }
  intervals(actor, fallback) {
    return this.inbox.state.preferences[actor]?.value.reminderMinutes.map(m => m * 60000) ?? fallback;
  }
  set(input, actor) {
    fields(input, ['expectedRevision', 'enabled', 'remindersEnabled', 'reminderMinutes', 'quietHours']);
    if (!Number.isSafeInteger(input.expectedRevision) || input.expectedRevision < 0 || typeof input.enabled !== 'boolean'
      || typeof input.remindersEnabled !== 'boolean') fail('invalid_notification_preferences', 'Review the notification settings.');
    const reminderMinutes = Array.isArray(input.reminderMinutes) ? input.reminderMinutes.map(v => typeof v === 'string' && /^[0-9]+$/.test(v) ? Number(v) : v) : null;
    if (!Array.isArray(reminderMinutes) || reminderMinutes.length < 1 || reminderMinutes.length > 3
      || reminderMinutes.some((v, i) => !Number.isSafeInteger(v) || v < 15 || v > 10080 || i > 0 && v <= reminderMinutes[i - 1]))
      fail('invalid_notification_preferences', 'Use one to three increasing reminder times, from 15 to 10080 minutes after the action starts.');
    let quietHours = null;
    if (input.quietHours !== null) {
      fields(input.quietHours, ['start', 'end', 'timeZone']);
      const { start, end, timeZone } = input.quietHours;
      if (!time(start) || !time(end) || start === end || typeof timeZone !== 'string' || timeZone.length > 100)
        fail('invalid_notification_preferences', 'Use different quiet-hour start/end times (HH:MM) and a time zone.');
      let zone;
      try { zone = formatter(timeZone).resolvedOptions().timeZone; }
      catch { fail('invalid_notification_preferences', 'Use a recognized time zone, such as America/Los_Angeles.'); }
      quietHours = { start, end, timeZone: zone };
    }
    const value = { enabled: input.enabled, remindersEnabled: input.remindersEnabled, reminderMinutes, quietHours };
    const fingerprint = createHash('sha256').update(canonical({ expectedRevision: input.expectedRevision, value })).digest('hex');
    const previous = this.inbox.state.preferences[actor];
    if (previous?.fingerprint === fingerprint) return this.get(actor); // Lost-response replay.
    if ((previous?.revision ?? 0) !== input.expectedRevision) fail('notification_preferences_changed', 'Settings changed on another device. Refresh before saving.', 409);
    if (!previous && Object.keys(this.inbox.state.preferences).length >= this.inbox.maxRecords)
      fail('notification_preferences_full', 'Notification settings storage is full.', 429);
    const cadenceChanged = canonical(previous?.value.reminderMinutes ?? defaults.reminderMinutes) !== canonical(reminderMinutes);
    this.inbox.store.atomic(() => {
      this.inbox.state.preferences[actor] = { revision: input.expectedRevision + 1, fingerprint, value };
      for (const notice of Object.values(this.inbox.state.notices)) {
        if (!cadenceChanged || notice.recipient !== actor || notice.cancelled || notice.delivery) continue;
        // A deliberate cadence change can schedule remaining reminders, but
        // cannot reset the count of messages already accepted by Matrix.
        notice.reminderCursor = Math.max(0, notice.delivered - 1);
        notice.finished = !notice.snoozedUntil && notice.delivered > 0 && notice.reminderCursor >= reminderMinutes.length;
        notice.dueAt = Math.max(notice.snoozedUntil ?? 0,
          notice.createdAt + (notice.delivered ? (reminderMinutes[notice.reminderCursor] ?? 0) * 60000 : 0));
      }
      this.inbox.store.audit(actor, 'notifications.preferences', null, actor, 'saved');
    });
    return this.get(actor);
  }
}
