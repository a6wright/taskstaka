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
  check(/✉ B/.test(storyChip || ''), `blocked story says who it waits on: "${storyChip}"`);
  await shot('01-outline');

  console.log('start a task from the row');
  const write = page.locator('li.row').nth(3);
  await write.hover();
  await write.locator('button.rowbtn.start').click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Write the change request', 'hover ▶ start lands on Next with that task in hand');
  await page.locator('a[data-view="outline"]').click(); await settle();
  check((await page.locator('li.row').nth(3).locator('.chip.doing').count()) === 1, 'and the row is doing');

  console.log('side pane');
  const write2 = page.locator('li.row').nth(3);
  await write2.hover();
  await write2.locator('button.openbtn').click(); await settle();
  check(await page.locator('#app.pane-open').count() === 1, 'open slides the pane in');
  check((await page.locator('.pane-title').textContent()) === 'Write the change request', 'pane shows the task');
  await shot('02-pane');
  await page.locator('#pane button[data-status="done"]').click(); await settle();
  check(await page.locator('li.row').nth(3).locator('.cb').isChecked(), 'done from the pane ticks the row');
  await page.keyboard.press('Escape'); await settle();
  check(await page.locator('#app.pane-open').count() === 0, 'Esc closes the pane');

  console.log('nudges is a page');
  await page.locator('a[data-view="nudges"]').click(); await settle();
  check(/Nudges/.test(await page.locator('.head h1').textContent()), 'nudges opens its own view');
  await shot('03-nudges');

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

  console.log('notes toggle');
  const dnsId = await page.locator('li.row').nth(0).getAttribute('data-id');
  await page.request.patch(base + '/api/v1/tasks/' + dnsId, { data: { body: 'Zone: corp.internal\nAsk about TTLs.\n- 2026-10-01 09:00 journal line stays in the pane' } });
  await settle();
  check((await page.locator('li.row .note').count()) === 0, 'notes are hidden by default');
  await page.locator('#toggle-notes').click(); await settle();
  const note = await page.locator('li.row').nth(0).locator('.note').textContent();
  check(/Zone: corp\.internal/.test(note || '') && !/journal line/.test(note || ''), `notes show under the row, journal lines do not ("${(note || '').replace(/\n/g, ' / ')}")`);
  await page.locator('#toggle-notes').click(); await settle();

  console.log('pane chevron on the boundary');
  const row0 = page.locator('li.row').nth(0);
  await row0.hover(); await row0.locator('button.openbtn').click(); await settle();
  check(await page.locator('#pane-toggle').isVisible(), 'chevron appears on the list/pane boundary');
  await page.locator('#pane-toggle').click(); await settle();
  check(await page.locator('#app.pane-open').count() === 0, 'and closes the pane');

  console.log('unblock a waiting task: straight to doing');
  const askRow = page.locator('li.row', { hasText: 'Ask B about wildcard certs' }).first();
  await askRow.hover();
  await askRow.locator('button.rowbtn', { hasText: 'unblock' }).click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Ask B about wildcard certs', 'unblock lands on Next with that task in hand');
  await page.locator('a[data-view="outline"]').click(); await settle();
  check((await page.locator('li.row', { hasText: 'Ask B about wildcard certs' }).first().locator('.chip.doing').count()) === 1, 'unblock marks it doing');
  await page.locator('a[data-view="next"]').click(); await settle();
  const firstNext = await page.locator('#main li.row .title').first().textContent();
  check(firstNext.startsWith('Ask B about wildcard certs'), `and it is at the top of Next ("${firstNext}")`);
  const wrapped = await page.locator('#main li.row').first().evaluate(li => li.getBoundingClientRect().height);
  check(wrapped < 40, `next rows stay on one line (${Math.round(wrapped)}px)`);
  await page.locator('a[data-view="outline"]').click(); await settle();

  console.log('drag to reorder');
  const before = await page.locator('li.row .title').allTextContents();
  const src = page.locator('li.row', { hasText: 'Nested from the bottom' }).first().locator('.grip');
  const dst = page.locator('li.row', { hasText: "Figure out what's possible" }).first();
  await src.hover({ force: true });
  await src.dragTo(dst, { targetPosition: { x: 200, y: 3 }, force: true }); await settle();
  const after = await page.locator('li.row .title').allTextContents();
  check(after.indexOf('Nested from the bottom') < after.indexOf("Figure out what's possible") && before.indexOf('Nested from the bottom') > before.indexOf("Figure out what's possible"), `dragged above its sibling (${after.join(' | ')})`);

  console.log('chain capture: Goal > intermediate > first thing');
  await page.locator('.newline .title').click();
  await page.keyboard.type('Bake an apple pie > Make the filling > Buy apples'); await page.keyboard.press('Enter'); await settle();
  const pie = page.locator('li.row', { hasText: 'Bake an apple pie' }).first();
  const pieId = await pie.getAttribute('data-id');
  const applesLi = page.locator('li.row', { hasText: 'Buy apples' }).first();
  check((await applesLi.getAttribute('data-depth')) === '2', 'the last segment is the deepest leaf');
  check((await page.locator('li.row', { hasText: 'Make the filling' }).first().getAttribute('data-parent')) === pieId, 'intermediate nests under the goal');

  console.log('first I need to…');
  await applesLi.hover(); await applesLi.locator('button.rowbtn', { hasText: 'first' }).click(); await settle();
  await page.keyboard.type('Drive to the orchard'); await page.keyboard.press('Enter'); await settle();
  check((await page.locator('li.row', { hasText: 'Drive to the orchard' }).first().getAttribute('data-depth')) === '3', '↳ first nests a prerequisite under the row');

  console.log('next: start → in hand → tick shifts up; stories fold their later list');
  const orchard = page.locator('li.row', { hasText: 'Drive to the orchard' }).first();
  await orchard.hover(); await orchard.locator('button.rowbtn.start').click(); await settle();
  check(await page.locator('.plate.top .plate-title').textContent() === 'Drive to the orchard', 'the started task is the top plate');
  const unlocks = await page.locator('.plate.top .unlocks').textContent();
  check(/Buy apples → Make the filling → Bake an apple pie/.test(unlocks || ''), `the plate says what it unlocks ("${unlocks}")`);
  const pieStory = page.locator('.story', { hasText: 'Bake an apple pie' }).first();
  check(/later/.test(await pieStory.locator('.story-meta').textContent()), 'the story header counts what is left');
  check((await pieStory.locator('.rows.later').count()) === 0, 'later is folded by default');
  await pieStory.locator('.later-toggle').click(); await settle();
  const laterTitles = await page.locator('.story', { hasText: 'Bake an apple pie' }).first().locator('.rows.later .title').allTextContents();
  check(laterTitles.some(x => /Buy apples/.test(x)) && laterTitles.some(x => /Bake an apple pie/.test(x)), `expanding shows everything still required (${laterTitles.map(x => x.split('\n')[0]).join(' | ')})`);
  // Push a prerequisite from the plate: it becomes in hand, the parent pauses.
  await page.locator('.plate.top #first-title').fill('Find the car keys');
  await page.locator('.plate.top button[data-first="prompt"]').click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Find the car keys', 'pushing a prerequisite makes it the task in hand');
  await page.locator('a[data-view="outline"]').click(); await settle();
  const orchardRow = page.locator('li.row', { hasText: 'Drive to the orchard' }).first();
  check((await orchardRow.locator('.chip.doing').count()) === 0, 'the task above it is paused (no longer doing)');
  await page.locator('a[data-view="next"]').click(); await settle();
  await page.locator('.plate.top button[data-status="done"]').click(); await settle();
  const handAfterDone = await page.locator('.plate.top .plate-title').textContent().catch(() => '');
  check(handAfterDone !== 'Find the car keys' && handAfterDone !== 'Drive to the orchard', `ticking leaves the hand to the next started task, not the parent ("${handAfterDone || 'empty'}")`);
  const nowTitles = await page.locator('.plate.now .rows .title').allTextContents();
  check(nowTitles.some(x => x.startsWith('Drive to the orchard')), `the unblocked parent is back among the now rows (${nowTitles.map(x => x.split('→')[0].trim()).join(' | ')})`);
  const driveRow = page.locator('.plate.now li.row', { hasText: 'Drive to the orchard' }).first();
  await driveRow.hover(); await driveRow.locator('button.rowbtn.start').click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Drive to the orchard', 'start puts it in hand');
  await page.locator('.plate.top button[data-status="todo"]').click(); await settle();
  const handAfterPutBack = await page.locator('.plate.top .plate-title').textContent().catch(() => '');
  check(handAfterPutBack !== 'Drive to the orchard', `put back takes it out of the hand ("${handAfterPutBack || 'empty'}")`);
  await page.locator('a[data-view="outline"]').click(); await settle();

  console.log('someday is an outline you can refine');
  await page.request.post(base + '/api/v1/tasks/' + pieId + '/status', { data: { status: 'someday' } }); await settle();
  await page.locator('a[data-view="someday"]').click(); await settle();
  check((await page.locator('li.row', { hasText: 'Bake an apple pie' }).first().locator('.title[contenteditable]').count()) === 1, 'parked story rows are editable');
  check((await page.locator('li.row', { hasText: 'Buy apples' }).count()) >= 1, 'its subtasks show nested');
  await page.locator('.newline .title').click(); await page.keyboard.type('Learn to make croissants'); await page.keyboard.press('Enter'); await settle();
  const parkedTitles = await page.request.get(base + '/api/v1/tasks?status=someday').then(r => r.json()).then(a => a.map(t => t.title));
  check(parkedTitles.includes('Learn to make croissants'), `capture in Someday parks the new story (${parkedTitles.join(' | ')})`);
  await page.locator('li.row', { hasText: 'Bake an apple pie' }).first().locator('button', { hasText: 'revive' }).click(); await settle();
  await page.locator('a[data-view="outline"]').click(); await settle();
  check((await page.locator('li.row', { hasText: 'Bake an apple pie' }).count()) >= 1, 'revive puts the story back in the outline');

  console.log('checkbox on a parent completes the subtree');
  const roots = await page.locator('li.row[data-depth="0"]').count();
  for (let i = 0; i < roots; i++) {
    const cb = page.locator('li.row[data-depth="0"]').nth(i).locator('.cb');
    if (!(await cb.isChecked())) { await cb.check(); await settle(); }
  }
  const checked = await page.locator('li.row .cb').evaluateAll(els => els.map(e => e.checked));
  check(checked.every(Boolean), `ticking each story ticks everything under it (${checked})`);

  check(errors.length === 0, 'no page errors' + (errors.length ? ':\n    ' + errors.join('\n    ') : ''));
  await browser.close();
  daemon.kill('SIGTERM');
  fs.rmSync(dir, { recursive: true, force: true });
  console.log(failures ? `\n${failures} check(s) failed` : '\nall checks passed');
  process.exit(failures ? 1 : 0);
})().catch(e => { console.error(e); daemon.kill('SIGTERM'); process.exit(1); });
