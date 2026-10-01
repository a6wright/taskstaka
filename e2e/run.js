// End-to-end: start a daemon on a scratch dir, drive the web UI in headless
// Chromium, assert what a user would see, and leave screenshots in shots/.
//
//   cargo build && (cd e2e && npm test)
//
// Exits non-zero on the first failed check. No test framework: the script
// *is* the scenario, top to bottom, so it reads like the manual pass it
// replaces.

const { chromium } = require('playwright');
const { spawn, execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const BIN = path.resolve(__dirname, '..', 'target', 'debug', 'taskstaka');
const SHOTS = path.join(__dirname, 'shots');
fs.mkdirSync(SHOTS, { recursive: true });

const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'taskstaka-e2e-'));
execFileSync(BIN, ['init'], { env: { ...process.env, TASKSTAKA_DIR: dir } });
const daemon = spawn(BIN, ['serve', '--port', '0', '--no-notify'], { env: { ...process.env, TASKSTAKA_DIR: dir }, stdio: ['ignore', 'ignore', 'pipe'] });
let stderr = '';
daemon.stderr.on('data', d => { stderr += d; });

async function waitForDaemon() {
  const f = path.join(dir, '.taskstaka', 'daemon.json');
  for (let i = 0; i < 100; i++) {
    if (fs.existsSync(f)) { try { return JSON.parse(fs.readFileSync(f, 'utf8')).port; } catch (_) {} }
    await new Promise(r => setTimeout(r, 50));
  }
  throw new Error('daemon did not start:\n' + stderr);
}

let failures = 0;
function check(cond, what) {
  if (cond) console.log('  ok   ' + what);
  else { failures++; console.log('  FAIL ' + what); }
}

(async () => {
  const port = await waitForDaemon();
  const base = `http://127.0.0.1:${port}`;
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: 'dark' });
  const errors = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  const shot = (name) => page.screenshot({ path: path.join(SHOTS, name + '.png') });
  const settle = () => page.waitForTimeout(350);

  console.log('outline: type, Tab, Enter');
  await page.goto(base + '/');
  await page.waitForSelector('.newline .title');
  const draft = page.locator('.newline .title');
  await draft.click();
  await page.keyboard.type('Get DNS address');
  await page.keyboard.press('Enter'); await settle();
  await page.keyboard.type("Figure out what's possible");
  await page.keyboard.press('Enter'); await settle();
  await page.keyboard.type('Write the change request');
  await page.keyboard.press('Enter'); await settle();
  check(await page.locator('li.row').count() === 3, 'three rows after three Enters');

  // Tab on the second row makes it a subtask of the first.
  await page.locator('li.row .title').nth(1).click();
  await page.keyboard.press('Tab'); await settle();
  let depths = await page.locator('li.row').evaluateAll(els => els.map(e => e.dataset.depth));
  check(depths.join(',') === '0,1,0', `Tab indents the second row (depths ${depths})`);

  // Tab on the third row too, then Enter after it inserts a sibling *there*.
  await page.locator('li.row .title').nth(2).click();
  await page.keyboard.press('Tab'); await settle();
  await page.locator('li.row .title').nth(1).click();
  await page.keyboard.press('End');
  await page.keyboard.press('Enter'); await settle();
  // "@B today": waiting on B with the follow-up due today, so it is actionable.
  await page.keyboard.type('Ask B about wildcard certs @B today'); await page.keyboard.press('Enter'); await settle();
  let titles = await page.locator('li.row .title').allTextContents();
  check(titles[2] === 'Ask B about wildcard certs', `Enter inserts right after the row (${titles.join(' | ')})`);
  check(await page.locator('li.row').nth(2).locator('.chip.wait').count() === 1, '@B made it waiting on B');
  // Make it a sub-subtask of "Figure out" to get the blocked chain.
  await page.locator('li.row .title').nth(2).click();
  await page.keyboard.press('Tab'); await settle();
  depths = await page.locator('li.row').evaluateAll(els => els.map(e => e.dataset.depth));
  check(depths.join(',') === '0,1,2,1', `tree is story › figure › ask, story › write (depths ${depths})`);
  const storyChip = await page.locator('li.row').nth(0).locator('.chip.wait').textContent().catch(() => '');
  check(/⌛ B/.test(storyChip || ''), `blocked story says who it waits on: "${storyChip}"`);
  await shot('01-outline');

  console.log('start a task from the row');
  const write = page.locator('li.row').nth(3);
  await write.hover();
  await write.locator('button.rowbtn.start').click(); await settle();
  check((await page.locator('li.row').nth(3).locator('.chip.doing').count()) === 1, 'hover ▶ start marks it doing');

  console.log('side pane');
  await write.hover();
  await write.locator('button.openbtn').click(); await settle();
  check(await page.locator('#app.pane-open').count() === 1, 'open slides the pane in');
  check((await page.locator('.pane-title').textContent()) === 'Write the change request', 'pane shows the task');
  await shot('02-pane');
  await page.locator('#pane button[data-status="done"]').click(); await settle();
  check(await page.locator('li.row').nth(3).locator('.cb').isChecked(), 'done from the pane ticks the row');
  await page.keyboard.press('Escape'); await settle();
  check(await page.locator('#app.pane-open').count() === 0, 'Esc closes the pane');

  console.log('sidebar chevron');
  await page.locator('#side-toggle').click({ force: true }); await settle();
  check(await page.locator('#app.side-collapsed').count() === 1, 'chevron collapses the sidebar');
  await shot('03-collapsed');
  await page.keyboard.press('\\'); await settle();
  check(await page.locator('#app.side-collapsed').count() === 0, 'backslash restores it');

  console.log('views');
  await page.locator('a[data-view="waiting"]').click(); await settle();
  check(/B/.test(await page.locator('.person .who').first().textContent()), 'Waiting on groups by person');
  await page.locator('a[data-view="next"]').click(); await settle();
  const nextRows = await page.locator('#main li.row').count();
  check(nextRows >= 1, `Next lists what is actionable as rows (${nextRows})`);
  check(/Ask B about wildcard certs/.test(await page.locator('#main li.row .title').allTextContents().then(a => a.join('|'))), 'the due follow-up is one of them');
  await shot('04-next');

  console.log('bottom box: Tab nests under the row above, Shift+Tab steps out');
  await page.locator('a[data-view="outline"]').click(); await settle();
  await page.locator('.newline .title').click();
  await page.keyboard.press('Tab'); await settle();
  const depthOf = () => page.locator('.newline').evaluate(e => e.style.getPropertyValue('--depth').trim());
  check((await depthOf()) === '1', 'Tab indents the capture box');
  await page.keyboard.type('Nested from the bottom'); await page.keyboard.press('Enter'); await settle();
  const nested = await page.locator('li.row[data-depth="1"] .title').allTextContents();
  check(nested.includes('Nested from the bottom'), `created as a subtask of the row above (${nested.join(' | ')})`);
  check((await depthOf()) === '1', 'the box stays indented for the next line');
  await page.keyboard.press('Shift+Tab'); await settle();
  check((await depthOf()) === '0', 'Shift+Tab steps back out');

  console.log('completed toggle');
  check(!(await page.locator('#toggle-done').evaluate(e => e.classList.contains('on'))), 'completed are hidden by default');
  await page.locator('#toggle-done').click(); await settle();
  check(await page.locator('#toggle-done').evaluate(e => e.classList.contains('on')), 'toggle turns on');
  check((await page.evaluate(() => localStorage.getItem('taskstaka.showDone'))) === '1', 'and is remembered');
  await page.locator('#toggle-done').click(); await settle();

  console.log('pane chevron on the boundary');
  const row0 = page.locator('li.row').nth(0);
  await row0.hover(); await row0.locator('button.openbtn').click(); await settle();
  check(await page.locator('#pane-toggle').isVisible(), 'chevron appears on the list/pane boundary');
  await page.locator('#pane-toggle').click(); await settle();
  check(await page.locator('#app.pane-open').count() === 0, 'and closes the pane');

  console.log('checkbox on a parent completes the subtree');
  await page.locator('li.row').nth(0).locator('.cb').check(); await settle();
  const checked = await page.locator('li.row .cb').evaluateAll(els => els.map(e => e.checked));
  check(checked.every(Boolean), `all rows ticked (${checked})`);

  check(errors.length === 0, 'no page errors' + (errors.length ? ':\n    ' + errors.join('\n    ') : ''));
  await browser.close();
  daemon.kill('SIGTERM');
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed');
  process.exit(failures ? 1 : 0);
})().catch(e => { console.error(e); daemon.kill('SIGTERM'); process.exit(1); });
