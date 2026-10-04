// Hagency is the source of contribution authority. Catalog visibility alone
// never creates one, and a missing page row never releases a reservation.
import { ApiError } from './service.mjs';
import { canonical } from './outbound.mjs';
const fail = (code, message) => { throw new ApiError(409, code, message); };
const closed = (v, keys) => v && typeof v === 'object' && !Array.isArray(v)
  && Object.keys(v).length === keys.length && keys.every(k => Object.hasOwn(v, k));
const id = v => typeof v === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(v);
const positive = v => Number.isSafeInteger(v) && v > 0;
const count = v => Number.isSafeInteger(v) && v >= 0;
const dimensions = ['tokens', 'maxAgents', 'maxRatePerDay'];
const limits = (v, zero = false) => closed(v, dimensions) && dimensions.every(k => (zero ? count : positive)(v[k])) && v.maxAgents <= 10000;
export const contributionKey = (fleetId, generation, id) => `${fleetId}:${generation}:${id}`;

export function validateContributionPage(fleet, page, { issuer, records, now, maxRecords }) {
  if (!closed(page, ['v', 'registrationGeneration', 'observedAtMs', 'after', 'contributions', 'nextAfter'])
    || page.v !== 1 || !positive(page.registrationGeneration) || !positive(page.observedAtMs) || page.observedAtMs > now + 5000
    || !(page.after === '' || id(page.after)) || !(page.nextAfter === null || id(page.nextAfter))
    || !Array.isArray(page.contributions) || page.contributions.length > 16) fail('invalid_contribution_page', 'Use a bounded contribution page from the registered Hagency.');
  const previous = fleet.contributionPublication;
  if (previous && page.registrationGeneration < previous.registrationGeneration) fail('contribution_generation_conflict', 'An older registration cannot restore contribution authority.');
  let after = page.after, added = 0;
  for (const row of page.contributions) {
    const g = row?.grant;
    if (!closed(row, ['grant', 'state', 'reserved'])
      || !closed(g, ['v', 'id', 'revision', 'fleetId', 'registrationGeneration', 'issuer', 'resourceId', 'limits', 'expiresAtMs'])
      || g.v !== 1 || !id(g.id) || g.id <= after || !positive(g.revision) || g.fleetId !== fleet.id
      || g.registrationGeneration !== page.registrationGeneration || g.issuer !== issuer || !/^resource_[a-f0-9]{24}$/.test(g.resourceId)
      || !limits(g.limits) || !limits(row.reserved, true) || !dimensions.every(k => row.reserved[k] <= g.limits[k]) || !positive(g.expiresAtMs)
      || !['active', 'expired', 'revoked'].includes(row.state)
      || (row.state === 'active' && g.expiresAtMs <= page.observedAtMs)
      || (row.state === 'expired' && g.expiresAtMs > page.observedAtMs)) fail('invalid_contribution_page', 'Contribution identity, finite limits, or state is invalid.');
    const old = records[contributionKey(fleet.id, page.registrationGeneration, g.id)];
    if (old) {
      if (canonical(old.grant) !== canonical(g) || page.observedAtMs < old.observedAtMs
        || !dimensions.every(k => row.reserved[k] >= old.reserved[k])
        || (old.state === 'revoked' && row.state !== 'revoked')
        || (old.state === 'expired' && row.state === 'active')) fail('contribution_conflict', 'A contribution cannot change its original budget, refund reservations, or restore retired authority.');
    } else added++;
    after = g.id;
  }
  if (page.nextAfter !== null && (page.contributions.length !== 16 || page.nextAfter !== after)) fail('invalid_contribution_page', 'The page cursor must identify its last contribution.');
  if (Object.keys(records).length + added > maxRecords) fail('contribution_store_full', 'The durable contribution store is full.');
  return structuredClone(page);
}

// Only called inside the outbound update's SQLite transaction.
export function applyContributionPage(fleet, page, records, now) {
  if (!page) return;
  for (const row of page.contributions) records[contributionKey(fleet.id, page.registrationGeneration, row.grant.id)] = {
    ...row, observedAtMs: page.observedAtMs, receivedAtMs: now, transportGeneration: fleet.transport.generation,
  };
  fleet.contributionPublication = { registrationGeneration: page.registrationGeneration, transportGeneration: fleet.transport.generation,
    observedAtMs: page.observedAtMs, receivedAtMs: now, nextAfter: page.nextAfter };
}
