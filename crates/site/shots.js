// Screenshots of a demo tree for the site. Run from the repo root after
// `cargo build --release -p taskstaka`:  node crates/site/shots.js
const { chromium } = require('../../e2e/node_modules/playwright');
const { spawn, execFileSync } = require('child_process');
const fs = require('fs'), os = require('os'), path = require('path');

const BIN = path.resolve(__dirname, '..', '..', 'target', 'release', 'taskstaka');
const OUT = path.join(__dirname, 'assets', 'shots');
fs.mkdirSync(OUT, { recursive: true });
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'taskstaka-site-'));
const env = { ...process.env, TASKSTAKA_DIR: dir, TASKSTAKA_NO_NOTIFY: 'true' };
delete env.TASKSTAKA_PORT; delete env.TASKSTAKA_DIRECT;
execFileSync(BIN, ['init'], { env });
const daemon = spawn(BIN, ['serve', '--port', '0', '--no-notify'], { env, stdio: ['ignore', 'ignore', 'inherit'] });

(async () => {
  const f = path.join(dir, '.taskstaka', 'daemon.json');
  let port;
  for (let i = 0; i < 100 && !port; i++) { try { port = JSON.parse(fs.readFileSync(f, 'utf8')).port; } catch (_) { await new Promise(r => setTimeout(r, 50)); } }
  const base = `http://127.0.0.1:${port}`;
  // The CLI finds the daemon through daemon.json, so these go through it.
  const ts = (...a) => execFileSync(BIN, a, { env }).toString().trim();
  const id = (s) => s.split(/\s+/)[1];
  const story = id(ts('add', 'Ship the new site', '--project', 'website'));
  const dns = id(ts('add', '-p', story, 'Point taskstaka.com at Cloud Run'));
  ts('add', '-p', dns, 'Ask Priya for the DNS record', '-w', 'Priya');
  const cert = id(ts('add', '-p', story, 'Renew the cert'));
  await fetch(`${base}/api/v1/tasks/${cert}`, { method: 'PATCH', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ body: 'certbot renew on web-1, then restart nginx.\nCheck the expiry in the browser before closing this.' }) });
  ts('start', cert);
  ts('add', '-p', story, 'Write the launch note');
  const inbox = id(ts('add', 'Reply to the accountant', '--project', 'admin'));
  ts('add', '-p', inbox, 'Find the Q3 receipts');
  ts('add', 'Learn the ukulele', '--someday');
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: 'dark' });
  for (const [name, p] of [['next', '/next'], ['outline', '/']]) {
    await page.goto(base + p, { waitUntil: 'load' });
    await page.waitForTimeout(800);
    await page.screenshot({ path: path.join(OUT, name + '.png') });
  }
  await browser.close();
  daemon.kill();
  console.log('wrote', fs.readdirSync(OUT).join(' '));
})().catch(e => { console.error(e); daemon.kill(); process.exit(1); });
