import assert from 'node:assert/strict';
import { createServer } from 'node:net';
import { request as httpRequest } from 'node:http';
import { once } from 'node:events';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fixture } from './fixture.mjs';
import { createApp } from '../server.mjs';

const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright-core');
const reservation = createServer().listen(0, '127.0.0.1'); await once(reservation, 'listening');
const port = reservation.address().port; await new Promise(r => reservation.close(r));
const origin = `http://127.0.0.1:${port}`, relayOrigin = 'http://relay.fixture:8090';
const f = fixture({ transportOrigin: origin, relayOrigin });
f.palpo.fetch = (url, options) => { assert.ok(!new URL(url).pathname.startsWith('/api/fleet/v1'), 'Outbound UI must never call HAFleet'); return f.fetch(url, options); };
const app = createApp({ service: f.service, publicOrigin: origin }).listen(port, '127.0.0.1'); await once(app, 'listening');
const browser = await chromium.launch({ headless: true, ...(process.env.CHROME_EXECUTABLE ? { executablePath: process.env.CHROME_EXECUTABLE } : { channel: 'chrome' }) });
const page = await browser.newPage({ viewport: { width: 1440, height: 1100 }, acceptDownloads: true });
const errors = []; page.on('pageerror', error => errors.push(error.message));
const http = (path, { body, method = body ? 'POST' : 'GET', headers = {} } = {}) => new Promise((resolve, reject) => {
  const req = httpRequest(`${origin}${path}`, { method, headers: { 'Content-Type': 'application/json', ...headers } }, res => {
    const chunks = []; res.on('data', chunk => chunks.push(chunk)); res.on('end', () => resolve({ status: res.statusCode, data: JSON.parse(Buffer.concat(chunks)) }));
  }); req.on('error', reject); req.end(body ? JSON.stringify(body) : undefined);
});
const signIn = async mxid => {
  await page.getByLabel('Matrix ID', { exact: true }).fill(mxid); await page.getByLabel('Password', { exact: true }).fill('correct-password');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click(); await page.locator('#login-panel').waitFor({ state: 'hidden' });
};
const refresh = async () => { await page.getByRole('button', { name: 'Refresh status', exact: true }).click(); await page.waitForFunction(() => !document.querySelector('#member-refresh').disabled); };
try {
  await page.goto(origin); await signIn('@admin:example.test');
  const form = page.locator('#fleet-form');
  assert.equal(await form.getByLabel('Connection mode').inputValue(), 'outbound');
  assert.equal(await form.locator('[name=callbackUrl]').isDisabled(), true);
  await form.getByLabel('Name', { exact: true }).fill('Outbound browser fleet'); await form.getByLabel('Owner Matrix ID').fill('@owner:example.test');
  await form.getByRole('button', { name: 'Authorize and install' }).click(); await page.locator('#fleets').getByText('Identity: verified', { exact: true }).waitFor();
  const fleet = Object.values(f.store.state.fleets)[0];
  await page.getByRole('button', { name: 'Sign out', exact: true }).click(); await signIn('@owner:example.test');
  const downloadPromise = page.waitForEvent('download'); await page.getByRole('button', { name: 'Download HAFleet configuration', exact: true }).click();
  const download = await downloadPromise, credentials = JSON.parse(await readFile(await download.path(), 'utf8'));
  assert.equal(credentials.transport.mode, 'outbound'); assert.equal(credentials.transport.url, `${origin}/api/fleet/v2/${fleet.id}`);
  const machine = (path, body) => http(`/api/fleet/v2/${fleet.id}${path}`, { body, headers: { Authorization: `Bearer ${credentials.transport.token}`, 'X-HAFleet-Generation': '1' } });
  const caps = { v: 1, fleetId: fleet.id, serverName: 'example.test', representativeMxid: fleet.representativeMxid, approvalBotMxid: '@approvalbot:example.test',
    offers: [{ role: 'coding', resources: [{ id: `resource_${'a'.repeat(24)}`, name: 'Published outbound resource', framework: 'codex', model: 'fixture-model' }] }] };
  assert.equal((await machine('/updates', { v: 2, generation: 1, sequence: 1, heartbeat: true, capabilities: caps })).status, 200);
  await page.getByRole('button', { name: 'Verify connection & create reception', exact: true }).click();
  await page.getByText('Verification event queued for HAFleet. Receipt will be confirmed over its outbound connection.', { exact: true }).waitFor();
  assert.equal(fleet.state, 'pending_connection');
  const event = f.events.get(fleet.probe.eventId);
  assert.equal((await http(`/api/relay/v2/${fleet.id}/_matrix/app/v1/transactions/browser-proof`, { method: 'PUT', body: { events: [event] }, headers: { Host: new URL(relayOrigin).host, Authorization: `Bearer ${fleet.registration.hs_token}` } })).status, 200);
  const matrix = (await machine(`/poll?lane=matrix&consumer=${randomUUID()}&wait=0`)).data.delivery;
  const work = (await machine(`/poll?lane=work&consumer=${randomUUID()}&wait=0`)).data.delivery;
  for (const delivery of [matrix, work]) assert.equal((await machine('/ack', { id: delivery.id, lane: delivery.lane, token: delivery.token })).status, 200);
  assert.equal((await machine('/updates', { v: 2, generation: 1, sequence: 2, heartbeat: true, probeReceipts: [{ v: 1, received: true, ...work.payload, mode: 'edge' }] })).status, 200);
  await refresh(); await page.locator('#my-fleets').getByText('Ready to receive requests', { exact: true }).waitFor();
  await page.locator('#project-form').getByLabel('Project name').fill('Outbound project');
  await page.getByRole('button', { name: 'Create project and approval room', exact: true }).click();
  await page.locator('#projects').getByRole('heading', { name: 'Outbound project', exact: true }).waitFor();
  fleet.transport.lastSeenAt = new Date(Date.now() - 100000).toISOString(); f.store.save(); await refresh();
  await page.locator('#request-connection').getByText(/HAFleet is offline/).waitFor();
  await page.getByRole('button', { name: 'Define Agent on this resource', exact: true }).click();
  await page.locator('#request-form').getByLabel('Agent name', { exact: true }).fill('offline-browser-agent');
  await page.locator('#request-form').getByLabel('Request ID', { exact: true }).fill('offline-browser-request');
  assert.equal(await page.getByRole('button', { name: 'Send agent request', exact: true }).isEnabled(), true);
  await page.getByRole('button', { name: 'Send agent request', exact: true }).click();
  await page.locator('#requests').getByText('Stored in Palpo. Waiting for HAFleet to receive this request; delivery does not approve or allocate an Agent.', { exact: true }).waitFor();
  assert.equal(Object.keys(f.store.state.requests).length, 1); assert.equal(Object.keys(fleet.agents).length, 0);
  const delivery = (await machine(`/poll?lane=work&consumer=${randomUUID()}&wait=0`)).data.delivery;
  assert.equal(delivery.payload.requestId, 'offline-browser-request');
  assert.equal((await machine('/ack', { id: delivery.id, lane: delivery.lane, token: delivery.token })).status, 200);
  assert.equal((await machine(`/poll?lane=work&consumer=${randomUUID()}&wait=0`)).data.delivery, null);
  assert.deepEqual(errors, []);
  const evidence = resolve('test-results'); await mkdir(evidence, { recursive: true });
  await page.screenshot({ path: resolve(evidence, 'outbound-offline-queued.png'), fullPage: true });
  const summary = { result: 'passed', realPalpo: false, pageErrors: errors, flows: ['outbound default authorization', 'owner-only machine configuration download', 'actual Matrix relay receipt before readiness', 'offline resources retained', 'offline request durably queued once', 'machine poll and receipt ACK without reverse callbacks'] };
  await writeFile(resolve(evidence, 'outbound-summary.json'), JSON.stringify(summary, null, 2)); console.log(JSON.stringify(summary));
} finally { await browser.close(); app.closeAllConnections(); await new Promise(r => app.close(r)); f.store.close(); }
