import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fixture, fleetInput } from './fixture.mjs';
import { Workflow } from '../lib/workflow.mjs';
import { createApp } from '../server.mjs';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright-core');
const f = fixture(), workflow = new Workflow(f.service);
const fleet = await f.service.create(fleetInput, '@admin:example.test', 'admin-secret');
await workflow.connect(fleet.id, '@owner:example.test', 'owner-secret');
const ownerProject = await workflow.createProject({ fleetId: fleet.id, requestId: 'owner-project', name: 'Owner project' }, '@owner:example.test', 'owner-secret');
await workflow.createProject({ fleetId: fleet.id, requestId: 'borrower-project', name: 'Borrower project' }, '@other:example.test', 'other-secret');
const server = createServer();
// Reserve a dynamic port before creating the origin-bound app.
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const port = server.address().port; await new Promise(resolve => server.close(resolve));
const origin = `http://127.0.0.1:${port}`;
const app = createApp({ service: f.service, publicOrigin: origin });
app.listen(port, '127.0.0.1'); await once(app, 'listening');
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_EXECUTABLE ? { executablePath: process.env.CHROME_EXECUTABLE } : { channel: 'chrome' }) });
const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
const pageErrors = []; page.on('pageerror', error => pageErrors.push(error.message));
const stored = f.service.fleet(fleet.id);
const evidence = resolve('test-results'); await mkdir(evidence, { recursive: true });
const refresh = async () => {
  const response = page.waitForResponse(r => r.url().endsWith('/api/catalog'));
  await page.getByRole('button', { name: 'Refresh status', exact: true }).click(); await response;
  await page.waitForFunction(() => !document.querySelector('#member-refresh').disabled);
};
const signIn = async mxid => {
  await page.getByLabel('Matrix ID', { exact: true }).fill(mxid);
  await page.getByLabel('Password', { exact: true }).fill('correct-password');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await page.locator('#projects article').first().waitFor();
};
const proofEvents = () => [...f.events.values()].filter(event => event.type === 'com.hagency.connection.probe.v1').length;
const connectResponses = [];
page.on('response', response => { if (response.url().endsWith('/connect')) connectResponses.push(response.status()); });
const waitReady = () => page.waitForFunction(() => document.querySelector('#request-connection').hidden && !document.querySelector('#request-form [type=submit]').disabled);
try {
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  await page.goto(origin); await signIn('@other:example.test');
  const connection = page.locator('#request-connection'), form = page.locator('#request-form');
  const send = form.getByRole('button', { name: 'Send agent request', exact: true });
  await connection.getByText(/Connection verification has expired/).waitFor();
  assert.match(await connection.innerText(), /Ask the Hagency owner \(@owner:example.test\)/);
  assert.equal(await connection.getByRole('button').count(), 0);
  assert.equal(await send.isEnabled(), false);
  assert.deepEqual(connectResponses, []); assert.equal(f.requests.size, 0);

  // Signing in again now renews the owner's separate proof through the real
  // connect handler, with a fresh source event and no agent request.
  const initialProof = stored.connection.sourceEventId, roomCount = f.rooms.size;
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  const loginRenewal = page.waitForResponse(r => r.url().endsWith('/connect'));
  await signIn('@owner:example.test'); assert.equal((await loginRenewal).status(), 200);
  await form.getByLabel('Request ID', { exact: true }).fill('retained-request-id');
  await form.getByLabel('Agent name', { exact: true }).fill('retry-one');
  await form.getByLabel('Resource', { exact: true }).selectOption(`resource_${'a'.repeat(24)}`);
  await form.getByLabel('Requested tokens', { exact: true }).fill('120000');
  await form.getByLabel('Daily rate', { exact: true }).fill('15000');
  await waitReady();
  assert.notEqual(stored.connection.sourceEventId, initialProof);
  assert.equal(stored.probe.completedAt !== undefined, true);
  assert.equal(f.rooms.size, roomCount); assert.equal(f.requests.size, 0);
  const draft = await form.evaluate(el => Object.fromEntries(new FormData(el)));

  // Renew before expiry. Repeated reads coalesce into one proof, and a still
  // valid connection keeps its draft usable while the new receipt is in flight.
  let releaseProbe, announceProbe;
  const probeStarted = new Promise(resolve => { announceProbe = resolve; });
  const heldProbe = new Promise(resolve => { releaseProbe = resolve; });
  let probeCalls = 0;
  f.palpo.fetch = async (target, options) => {
    if (new URL(target).pathname === '/api/fleet/v1/probe') { probeCalls++; announceProbe(); await heldProbe; }
    return f.fetch(target, options);
  };
  const periodicRenewal = page.waitForRequest(r => r.url().endsWith('/connect'));
  stored.connection.expiresAt = new Date(Date.now() + 30000).toISOString();
  const beforeRenewal = proofEvents();
  await periodicRenewal; await probeStarted;
  assert.equal(await send.isEnabled(), true);
  // Focus/catalog reads run independently of the serialized connect mutation.
  await page.evaluate(() => { window.dispatchEvent(new Event('focus')); window.dispatchEvent(new Event('focus')); });
  await page.waitForResponse(r => r.url().endsWith('/api/catalog'));
  assert.equal(probeCalls, 1); assert.equal(proofEvents(), beforeRenewal + 1);
  const earlyRenewal = page.waitForResponse(r => r.url().endsWith('/connect'));
  releaseProbe(); assert.equal((await earlyRenewal).status(), 200); await waitReady();
  f.palpo.fetch = f.fetch;

  // A mismatched receipt cannot extend the proof. Failure remains actionable,
  // disables Send, and catalog refreshes respect the retry backoff.
  const confirmedProof = { ...stored.connection };
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  probeCalls = 0;
  f.palpo.fetch = async (target, options) => {
    const result = await f.fetch(target, options);
    if (new URL(target).pathname === '/api/fleet/v1/probe') {
      probeCalls++; return new Response(JSON.stringify({ ...await result.json(), challenge: 'wrong-challenge' }), { status: 200 });
    }
    return result;
  };
  await refresh();
  await connection.getByText(/Connection could not be verified/).waitFor();
  assert.equal(await send.isEnabled(), false);
  assert.equal(stored.connection.sourceEventId, confirmedProof.sourceEventId);
  assert.ok(Date.parse(stored.connection.expiresAt) < Date.now());
  await refresh(); await refresh(); assert.equal(probeCalls, 1);
  assert.deepEqual(await form.evaluate(el => Object.fromEntries(new FormData(el))), draft);
  assert.equal(f.requests.size, 0); assert.equal(Object.keys(f.store.state.requests).length, 0);
  await page.screenshot({ path: resolve(evidence, 'request-renewal-failed.png'), fullPage: true });

  // Manual retry remains available immediately and reuses the pending probe.
  const failedProbe = stored.probe.eventId;
  f.palpo.fetch = f.fetch;
  await connection.getByRole('button', { name: 'Verify connection', exact: true }).click();
  await page.locator('#request-status').getByText(/Connection verified. Review your request/).waitFor();
  await waitReady(); assert.equal(stored.connection.sourceEventId, failedProbe);
  assert.equal(await page.locator('#notice').isVisible(), false);

  // A server-side expiry after render still rejects the submission. Automatic
  // renewal repairs readiness, but never retries the agent submission itself.
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  const rejected = page.waitForResponse(r => r.url().endsWith('/api/requests') && r.request().method() === 'POST');
  const lateRenewal = page.waitForResponse(r => r.url().endsWith('/connect'));
  await send.click(); assert.equal((await rejected).status(), 409);
  assert.equal((await lateRenewal).status(), 200); await waitReady();
  await page.locator('#request-status').getByText(/Hagency has not confirmed this request/).waitFor();
  assert.equal(f.requests.size, 0); assert.equal(Object.keys(f.store.state.requests).length, 0);
  assert.deepEqual(await form.evaluate(el => Object.fromEntries(new FormData(el))), draft);

  // Hidden pages do not create background protocol traffic. Returning to the
  // page renews the expired connection without losing the saved draft.
  await page.evaluate(() => Object.defineProperty(document, 'hidden', { configurable: true, get: () => true }));
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  const beforeHidden = connectResponses.length;
  await refresh(); assert.equal(await send.isEnabled(), false);
  assert.equal(connectResponses.length, beforeHidden);
  const visibleRenewal = page.waitForResponse(r => r.url().endsWith('/connect'));
  await page.evaluate(() => { delete document.hidden; document.dispatchEvent(new Event('visibilitychange')); });
  assert.equal((await visibleRenewal).status(), 200); await waitReady();
  assert.deepEqual(await form.evaluate(el => Object.fromEntries(new FormData(el))), draft);

  // Failed status readback after a good probe is also fail-closed and backed
  // off; a successful POST alone must not manufacture current catalog state.
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  let failReadback = false;
  await page.route('**/api/catalog', async route => {
    if (failReadback) return route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Catalog unavailable' }) });
    return route.continue();
  });
  f.palpo.fetch = async (target, options) => {
    const result = await f.fetch(target, options);
    if (new URL(target).pathname === '/api/fleet/v1/probe') failReadback = true;
    return result;
  };
  await refresh();
  await connection.getByText(/Connection could not be verified: Catalog unavailable/).waitFor();
  assert.equal(await send.isEnabled(), false);
  const afterReadFailure = connectResponses.length;
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await page.waitForResponse(r => r.url().endsWith('/api/catalog'));
  assert.equal(connectResponses.length, afterReadFailure);
  failReadback = false; f.palpo.fetch = f.fetch; await page.unroute('**/api/catalog');
  // Expiring the backoff retries even if the server's previous successful POST
  // left a future-dated proof that the failed readback could not validate.
  await page.clock.setFixedTime(new Date(Date.now() + 61000));
  const recoveredReadback = page.waitForResponse(r => r.url().endsWith('/connect'));
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  assert.equal((await recoveredReadback).status(), 200); await waitReady();
  await page.clock.setSystemTime(new Date());
  assert.equal(f.requests.size, 0);

  await send.click();
  await page.locator('#request-status').getByText(/Request retained-request-id delivered to Hagency/).waitFor();
  await page.locator('#requests').getByText('Awaiting the Hagency owner’s resource decision.', { exact: true }).waitFor();
  assert.equal(f.requests.size, 1);
  const actual = Object.values(f.store.state.requests)[0];
  assert.equal(actual.projectId, ownerProject.id); assert.equal(actual.state, 'pending');
  assert.equal(actual.payload.requestedTokens, 120000); assert.equal(actual.payload.ratePerDay, 15000);
  assert.equal(f.rooms.size, roomCount); assert.deepEqual(pageErrors, []);
  // A previous owner's in-flight catalog must not be reused by another login;
  // its late authentication error must not sign the new account out either.
  let captureOldCatalog;
  const oldCatalog = new Promise(resolve => { captureOldCatalog = resolve; });
  await page.route('**/api/catalog', route => {
    if (captureOldCatalog) { const capture = captureOldCatalog; captureOldCatalog = null; capture(route); }
    else return route.continue();
  });
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  const oldRoute = await oldCatalog;
  const beforeLogout = connectResponses.length;
  await page.getByRole('button', { name: 'Sign out', exact: true }).click();
  stored.connection.expiresAt = new Date(Date.now() - 1000).toISOString();
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  const newCatalog = page.waitForResponse(r => r.url().endsWith('/api/catalog') && r.status() === 200);
  await signIn('@other:example.test');
  await newCatalog;
  await connection.getByText(/Connection verification has expired/).waitFor();
  const oldResponse = page.waitForResponse(r => r.url().endsWith('/api/catalog') && r.status() === 401);
  await oldRoute.fulfill({ status: 401, contentType: 'application/json', body: JSON.stringify({ code: 'sign_in_required', error: 'Old session expired' }) });
  await oldResponse;
  await refresh();
  assert.equal(await page.locator('#login-panel').isVisible(), false);
  assert.match(await page.locator('#account').innerText(), /@other:example.test/);
  assert.equal(connectResponses.length, beforeLogout);
  assert.deepEqual(pageErrors, []);
  const summary = { result: 'passed', realPalpo: false, flows: ['nonowner cannot renew another fleet', 'owner login renews an established expired proof', 'early renewal preserves valid readiness and coalesces requests', 'mismatched receipt fails closed with backoff', 'manual retry retains exact pending probe', 'late server rejection renews without resubmitting', 'hidden page waits until visible', 'failed catalog readback stays blocked without retry storm', 'draft and operation ID preserved', 'only explicit submission creates one pending request', 'logout and nonowner login stop owner renewal', 'old catalog and authentication responses stay in their original session'], pageErrors };
  await writeFile(resolve(evidence, 'request-readiness-summary.json'), JSON.stringify(summary, null, 2));
  console.log(JSON.stringify(summary));
} finally { await browser.close(); await new Promise(resolve => app.close(resolve)); f.store.close(); }
