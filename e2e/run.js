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

let page;   // hoisted so a failure can leave a screenshot and the rows it saw
(async () => {
  const port = await waitForDaemon();
  const base = `http://127.0.0.1:${port}`;
  const browser = await chromium.launch();
  page = await browser.newPage({ viewport: { width: 1280, height: 800 }, colorScheme: 'dark' });
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

  console.log('notes are Markdown');
  await page.request.patch(base + '/api/v1/tasks/' + dnsId, { data: { body: 'Zone: **corp.internal** — see [the runbook](https://example.com/runbook) and run `dig`.\n\n- [ ] TTLs\n- [x] owner\n\n<b>raw html stays text</b>\n- 2026-10-01 09:00 journal line stays in the pane' } });
  await settle();
  const rowNote = page.locator('li.row').nth(0).locator('.note.md');
  check((await rowNote.locator('strong').count()) === 1 && (await rowNote.locator('a[href="https://example.com/runbook"]').count()) === 1, 'the note under the row renders bold and the link');
  check(/<b>raw html stays text<\/b>/.test(await rowNote.textContent()), 'raw HTML in a note is shown as text, not interpreted');
  await page.locator('#toggle-notes').click(); await settle();
  await page.locator('li.row').nth(0).hover();
  await page.locator('li.row').nth(0).locator('button.openbtn').click(); await settle();
  const view = page.locator('#pane .notes-view');
  check((await view.locator('code').textContent()) === 'dig' && (await view.locator('input[type="checkbox"]').count()) === 2 && (await view.locator('input[type="checkbox"]:checked').count()) === 1, 'the pane renders code and the task list');
  check(!/journal line/.test(await view.textContent()), 'journal lines are not part of the rendered notes');
  await shot('10-markdown');
  await view.click(); await settle();
  const ta = page.locator('#pane textarea.notes');
  check((await ta.count()) === 1 && /Zone: \*\*corp\.internal\*\*/.test(await ta.inputValue()) && !/journal line/.test(await ta.inputValue()), 'clicking the notes opens the source in a textarea, without the journal');
  await ta.press('End'); await page.keyboard.type('\n\nAlso *italic*.');
  await page.locator('.pane-title').click(); await settle();
  check((await page.locator('#pane textarea.notes').count()) === 0 && (await page.locator('#pane .notes-view em').textContent()) === 'italic', 'leaving the textarea saves and renders again');
  const saved = await (await page.request.get(base + '/api/v1/tasks/' + dnsId)).json();
  check(/Also \*italic\*\.\n\n- 2026-10-01 09:00 journal line/.test(saved.body), `the journal line is kept after the notes (${JSON.stringify(saved.body)})`);
  await page.keyboard.press('Escape'); await settle();

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
  await page.locator('.plate.top .first-title').fill('Find the car keys');
  await page.locator('.plate.top button[data-first="prompt"]').click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Find the car keys', 'pushing a prerequisite makes it the task in hand');
  await page.locator('a[data-view="outline"]').click(); await settle();
  const orchardRow = page.locator('li.row', { hasText: 'Drive to the orchard' }).first();
  check((await orchardRow.locator('.chip.doing').count()) === 0, 'the task above it is paused (no longer doing)');
  await page.locator('a[data-view="next"]').click(); await settle();
  await page.locator('.plate.top button[data-status="done"]').click(); await settle();
  // ADR 0010: finishing the task in hand starts the parent it just unblocked.
  check((await page.locator('.plate.top .plate-title').textContent()) === 'Drive to the orchard', 'ticking the last prerequisite puts its parent in hand');
  check((await page.locator('.plate.top .rel').textContent()).startsWith('in hand'), 'the parent reads "in hand"');
  const nowTitles = await page.locator('.plate.now .rows .title').allTextContents();
  check(nowTitles.some(x => x.startsWith('Drive to the orchard')), `the unblocked parent is among the now rows (${nowTitles.map(x => x.split('→')[0].trim()).join(' | ')})`);
  await page.locator('.plate.top button[data-status="todo"]').click(); await settle();
  const handAfterPutBack = await page.locator('.plate.top .plate-title').textContent().catch(() => '');
  check(handAfterPutBack && handAfterPutBack !== 'Drive to the orchard', `put back takes it out of the hand and the plate stays filled ("${handAfterPutBack || 'empty'}")`);
  // ADR 0014: the plate is never empty while something is ready. Put back
  // everything started; the plate then shows what is up next and offers start.
  for (let i = 0; i < 6 && (await page.locator('.plate.top .rel').textContent()).startsWith('in hand'); i++) {
    await page.locator('.plate.top button[data-status="todo"]').click(); await settle();
  }
  const upNext = await page.locator('.plate.top .plate-title').textContent().catch(() => '');
  check((await page.locator('.plate.top .rel').textContent().catch(() => '')).startsWith('up next'), `with nothing in hand the plate shows what is up next ("${upNext || 'empty'}")`);
  check(upNext && upNext !== 'Drive to the orchard', 'up next prefers a task never picked up over one put back');
  check((await page.locator('.plate.top button[data-status="doing"]').count()) === 1 && (await page.locator('.plate.top button[data-status="todo"]').count()) === 0, 'the up-next plate offers start, not put back');
  await page.locator('.plate.top button[data-status="doing"]').click(); await settle();
  check((await page.locator('.plate.top .plate-title').textContent()) === upNext && (await page.locator('.plate.top .rel').textContent()).startsWith('in hand'), 'start from the plate takes it in hand');
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

  console.log('fold a row\'s subtasks');
  const dnsRow = page.locator('li.row', { hasText: 'Get DNS address' }).first();
  const dnsKids = await page.locator(`li.row[data-parent="${dnsId}"]`).count();
  check(dnsKids >= 2, `the DNS story shows its subtasks (${dnsKids})`);
  await dnsRow.hover(); await dnsRow.locator('button.fold').click({ force: true }); await settle();
  check((await page.locator(`li.row[data-parent="${dnsId}"]`).count()) === 0, 'the chevron folds them away');
  check(/more/.test(await dnsRow.locator('.chip.foldchip').textContent().catch(() => '')), 'the row says how many are folded');
  check(JSON.parse(await page.evaluate(() => localStorage.getItem('taskstaka.collapsed') || '[]')).includes(dnsId), 'the fold is remembered');
  await page.locator('li.row', { hasText: 'Get DNS address' }).first().locator('.chip.foldchip').click(); await settle();
  check((await page.locator(`li.row[data-parent="${dnsId}"]`).count()) === dnsKids, 'the chip unfolds them');
  await page.locator('#toggle-fold').click(); await settle();
  check((await page.locator('li.row[data-depth="1"]').count()) === 0 && (await page.locator('li.row[data-depth="0"]').count()) >= 2, 'fold in the head folds every story to one line');
  await shot('06-folded');
  await page.locator('#toggle-fold').click(); await settle();
  check((await page.locator('li.row[data-depth="1"]').count()) > 0, 'unfold brings them back');

  console.log('search');
  await page.locator('.head h1').click();   // nothing editable focused
  await page.keyboard.press('/');
  check(await page.locator('#search').evaluate(e => document.activeElement === e), '/ focuses the search box');
  await page.keyboard.type('apples'); await settle();
  let shownTitles = await page.locator('li.row .title').allTextContents();
  check(shownTitles.some(x => /Buy apples/.test(x)) && shownTitles.some(x => /Bake an apple pie/.test(x)) && !shownTitles.some(x => /Get DNS/.test(x)), `typing narrows the outline to matches and their ancestors (${shownTitles.join(' | ')})`);
  check((await page.locator('li.row.dim', { hasText: 'Bake an apple pie' }).count()) === 1, 'an ancestor that does not match itself is dimmed');
  check(/q=apples/.test(page.url()), 'the query is in the URL');
  check(await page.locator('#search').evaluate(e => document.activeElement === e), 'typing keeps the caret in the box');
  await shot('07-search');
  await page.fill('#search', 'nothing-like-this'); await settle();
  check((await page.locator('.content .empty').count()) === 1 && (await page.locator('li.row').count()) === 0, 'no match says so');
  await page.locator('.content .empty a[data-clear]').click(); await settle();
  check((await page.locator('li.row', { hasText: 'Get DNS' }).count()) >= 1 && !/q=/.test(page.url()), 'clear the filters restores the outline');

  console.log('filter by project and tag');
  await page.request.patch(base + '/api/v1/tasks/' + pieId, { data: { project: { to: 'kitchen' } } });
  const applesId = await page.locator('li.row', { hasText: 'Buy apples' }).first().getAttribute('data-id');
  await page.request.patch(base + '/api/v1/tasks/' + applesId, { data: { tags: ['agent'] } }); await settle(); await settle();
  check((await page.locator('#side .facets a[data-project="kitchen"]').count()) === 1, 'the sidebar lists the project');
  check((await page.locator('#side .facets a[data-tag="agent"]').count()) === 1, 'and the tag');
  await page.locator('#side .facets a[data-project="kitchen"]').click(); await settle();
  shownTitles = await page.locator('li.row .title').allTextContents();
  check(shownTitles.some(x => /Bake an apple pie/.test(x)) && !shownTitles.some(x => /Get DNS/.test(x)), `the project facet narrows the outline (${shownTitles.join(' | ')})`);
  check((await page.locator('.head .proj.filter', { hasText: 'kitchen' }).count()) === 1 && /project=kitchen/.test(page.url()), 'the head shows the project chip and the URL carries it');
  await shot('08-filter');
  await page.locator('a[data-view="next"]').click(); await settle();
  const storyTitles = await page.locator('.story .story-title').allTextContents();
  check(storyTitles.length === 1 && /Bake an apple pie/.test(storyTitles[0]), `Next keeps only that project's stories (${storyTitles.join(' | ')})`);
  await page.locator('.head .proj.filter').click(); await settle();
  check(!/project=/.test(page.url()) && (await page.locator('.story').count()) >= 2, 'the × on the chip clears it');
  await page.locator('a[data-view="outline"]').click(); await settle();
  await page.locator('li.row', { hasText: 'Buy apples' }).first().locator('.chip.agent').click(); await settle();
  shownTitles = await page.locator('li.row .title').allTextContents();
  check(shownTitles.length === 3 && shownTitles.some(x => /Buy apples/.test(x)) && /tag=agent/.test(page.url()), `clicking a tag chip filters to that tag (${shownTitles.join(' | ')})`);
  check((await page.locator('#side .facets a[data-tag="agent"].active').count()) === 1, 'the facet shows as active');
  await page.locator('#side .facets a[data-tag="agent"]').click(); await settle();
  check(!/tag=/.test(page.url()), 'clicking the active facet clears it');

  console.log('click to move: grip, then a slot');
  const orchardLi = page.locator('li.row', { hasText: 'Drive to the orchard' }).first();
  const fillingId = await page.locator('li.row', { hasText: 'Make the filling' }).first().getAttribute('data-id');
  await orchardLi.hover(); await orchardLi.locator('.grip').click({ force: true }); await settle();
  check((await page.locator('.movebar').count()) === 1 && (await page.locator('li.slot').count()) > 3, 'the grip picks the row up and slots appear');
  check((await page.locator('li.row.moving').count()) === 1, 'the picked row is marked');
  await page.locator(`li.slot[data-parent="${pieId}"][data-after="${fillingId}"]`).hover(); await shot('09-move');
  await page.locator(`li.slot[data-parent="${pieId}"][data-after="${fillingId}"]`).click(); await settle();
  const orchardAfter = page.locator('li.row', { hasText: 'Drive to the orchard' }).first();
  check((await orchardAfter.getAttribute('data-parent')) === pieId && (await orchardAfter.getAttribute('data-depth')) === '1', 'clicking a slot moves it there');
  check((await page.locator('.movebar').count()) === 0 && (await page.locator('li.slot').count()) === 0, 'and leaves move mode');
  await orchardAfter.hover(); await orchardAfter.locator('.grip').click({ force: true }); await settle();
  await page.keyboard.press('Escape'); await settle();
  check((await page.locator('.movebar').count()) === 0, 'Esc cancels a move');

  console.log('Alt+↑ swaps with the sibling above');
  const order1 = await page.locator(`li.row[data-parent="${dnsId}"] .title`).allTextContents();
  const figure = page.locator('li.row', { hasText: "Figure out what's possible" }).first();
  await figure.locator('.title').click(); await page.keyboard.press('Alt+ArrowUp'); await settle();
  const order2 = await page.locator(`li.row[data-parent="${dnsId}"] .title`).allTextContents();
  check(order1.indexOf("Figure out what's possible") === 1 && order2.indexOf("Figure out what's possible") === 0, `Alt+↑ moves the row up (${order2.join(' | ')})`);
  await page.keyboard.press('Alt+ArrowDown'); await settle();
  const order3 = await page.locator(`li.row[data-parent="${dnsId}"] .title`).allTextContents();
  check(order3.join('|') === order1.join('|'), 'Alt+↓ moves it back down');
  await page.keyboard.press('Escape');

  console.log('done, day by day');
  const mk = async (title, parent) => (await (await page.request.post(base + '/api/v1/tasks', { data: { title, parent: parent || null, context: 'work' } })).json()).primary;
  const finish = (id, status = 'done') => page.request.post(`${base}/api/v1/tasks/${id}/status`, { data: { status, cascade: true } });
  const standup = await mk('Standup story');
  const planId = await mk('Plan the sprint', standup);
  await mk('Pick the stories', planId);
  const writeId = await mk('Write the notes', standup);
  const openId = await mk('Open parent', standup);
  const loneId = await mk('Lone leaf', openId);
  const goneId = await mk('Old idea', standup);
  // People traffic: a wait, a chase and a reply all land in the day log under the task.
  const askId = await mk('Ask Dana for the keys', standup);
  await page.request.post(`${base}/api/v1/tasks/${askId}/status`, { data: { status: 'waiting', waiting_on: 'Dana' } });
  await page.request.post(`${base}/api/v1/tasks/${askId}/chase`, { data: { note: 'pinged on Teams', source: 'agent' } });
  await page.request.post(`${base}/api/v1/tasks/${askId}/reply`, { data: { outcome: { kind: 'answered' }, summary: 'keys are in the drawer' } });
  await finish(planId); await finish(loneId); await finish(goneId, 'dropped'); await finish(writeId);
  // Backdate "Write the notes" to yesterday noon by editing its note, then reload.
  const yest = new Date(); yest.setDate(yest.getDate() - 1); yest.setHours(12, 0, 0, 0);
  const noteFile = path.join(dir, 'notes', fs.readdirSync(path.join(dir, 'notes')).find(f => f.startsWith(writeId + '-')));
  fs.writeFileSync(noteFile, fs.readFileSync(noteFile, 'utf8').replace(/^completed: .*$/m, 'completed: ' + yest.toISOString().replace(/\.\d{3}Z$/, 'Z')));
  await page.request.post(base + '/api/v1/reload');
  const iso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  const todayIso = iso(new Date()), yestIso = iso(yest);
  await page.goto(base + '/done'); await page.waitForSelector('.daylog.week'); await settle();
  const col = (d) => page.locator(`section.day[data-day="${d}"]`);
  const dayCols = await page.locator('section.day').evaluateAll(els => els.map(e => e.dataset.day));
  check(dayCols.length === 7 && dayCols[6] === todayIso, `seven days ending today (${dayCols.join(' ')}; today ${todayIso})`);
  check(/today/.test(await col(todayIso).locator('header').textContent()) && /yesterday/.test(await col(yestIso).locator('header').textContent()), 'today and yesterday are named');
  // Earlier scenarios finished tasks in other stories today; each sits under its own story block.
  const heads = await col(todayIso).locator('.dstory .story-title').allTextContents();
  check(heads.includes('Standup story') && heads.length >= 2 && new Set(heads).size === heads.length, `today's work is grouped under its stories, one block each (${heads.join(' | ')})`);
  const block = col(todayIso).locator('.dstory', { has: page.locator('.story-title', { hasText: 'Standup story' }) });
  const tTitles = await block.locator('.daylog-row .title a').allTextContents();
  check(tTitles.join('|') === 'Plan the sprint|Pick the stories|Lone leaf|Ask Dana for the keys', `outline order, parent before the subtask it closed with (${tTitles.join(' | ')})`);
  const danaRow = block.locator('.daylog-row', { hasText: 'Ask Dana' });
  const acts = await danaRow.locator('.act').allTextContents();
  check((await danaRow.locator('.mark').textContent()) === '✉' && acts.length === 3 && /waiting on Dana/.test(acts[0]) && /chased Dana.*pinged on Teams.*\[agent\]/.test(acts[1]) && /Dana replied: keys are in the drawer/.test(acts[2]), `a task you only touched shows ✉ with the wait, the ping and the reply under it (${acts.join(' | ')})`);
  const tally = await col(todayIso).locator('header .n').textContent();
  check(/^✓ \d+ · ✉ \d+$/.test(tally), `the day header tallies finished and touched ("${tally}")`);
  const pickDepth = await block.locator('.daylog-row', { hasText: 'Pick the stories' }).evaluate(e => e.style.getPropertyValue('--depth').trim());
  check(pickDepth === '1', `the subtask nests under its parent (depth ${pickDepth})`);
  const loneCrumb = await block.locator('.daylog-row', { hasText: 'Lone leaf' }).locator('.crumb').textContent();
  check(/› Open parent/.test(loneCrumb || ''), `a leaf finished alone shows the chain to its story ("${loneCrumb}")`);
  const yTitles = await col(yestIso).locator('.daylog-row .title a').allTextContents();
  check(yTitles.join('|') === 'Write the notes' && (await col(yestIso).locator('.dstory .story-title').textContent()) === 'Standup story', `yesterday has its own column, under the same story (${yTitles.join(' | ')})`);
  check((await page.locator('.daylog-row', { hasText: 'Old idea' }).count()) === 0, 'dropped tasks are hidden by default');
  await page.locator('#toggle-dropped').click(); await settle();
  check((await page.locator('.daylog-row.dropped', { hasText: 'Old idea' }).count()) === 1, 'the ✗ dropped toggle shows them, struck through');
  await shot('11-done-week');
  await page.locator('#toggle-dropped').click(); await settle();
  await page.locator('[data-day-nav="prev"]').click(); await settle();
  check(/[?&]day=\d{4}-\d{2}-\d{2}/.test(page.url()) && (await page.locator('section.day').count()) === 7 && (await page.locator(`section.day[data-day="${todayIso}"]`).count()) === 0, '‹ moves a week back, carried in the URL');
  await page.locator('[data-day-nav="today"]').click(); await settle();
  check(!/day=/.test(page.url()) && (await col(todayIso).count()) === 1, 'today comes back');
  await page.locator('[data-span="month"]').click(); await settle();
  const cell = page.locator(`section.cell[data-day="${todayIso}"]`);
  const mline = cell.locator('.mline', { hasText: 'Standup story' });
  check(/span=month/.test(page.url()) && (await mline.count()) === 1 && (await mline.locator('.mn').textContent()) === '4' && (await cell.locator('.mline').count()) === heads.length, 'the month grid shows one line per story with its count');
  await shot('12-done-month');
  await cell.locator('.dnum').click(); await settle();
  check((await page.locator('.daylog.week').count()) === 1 && !/span=/.test(page.url()), 'clicking a day opens its week');
  const md = await page.evaluate((d) => window.dayText(d), todayIso);
  check(/^### /.test(md) && md.includes('- **Standup story**') && md.includes('\n  - [x] Plan the sprint\n    - [x] Pick the stories\n') && md.includes('- [x] Lone leaf (› Open parent)') && /- ✉ Ask Dana for the keys\n    - \d\d:\d\d waiting on Dana/.test(md) && /chased Dana.*\[agent\]/.test(md), `copy gives nested Markdown for standup, people traffic included:\n${md.replace(/^/gm, '      ')}`);
  await page.locator('#search').fill('lone'); await settle();
  check((await col(todayIso).locator('.daylog-row .title a').allTextContents()).join('|') === 'Lone leaf' && (await col(todayIso).locator('.dstory').count()) === 1, 'the search narrows the columns to the match, under its story');
  await page.locator('#search').fill('zzz-nothing'); await settle();
  check((await page.locator('[data-clear="all"]').count()) === 1, 'no match offers to clear the filters');
  await page.locator('#search').fill(''); await settle();
  await page.locator('a[data-view="outline"]').click(); await settle();

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
})().catch(async (e) => {
  console.error(e);
  try {
    await page.screenshot({ path: path.join(SHOTS, 'fail.png') });
    console.error('url: ' + page.url());
    console.error('rows: ' + JSON.stringify(await page.locator('li.row').evaluateAll(els => els.map(e => ({ id: e.dataset.id, depth: e.dataset.depth, cls: e.className, title: (e.querySelector('.title') || {}).textContent, meta: (e.querySelector('.meta') || {}).innerHTML })))));
  } catch (_) {}
  daemon.kill('SIGTERM'); process.exit(1);
});
