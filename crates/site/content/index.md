<div class="hero">
<img class="logo" src="/logo.png" alt="A pyramid of orange cubes with two tumbling off">

# A "what needs action next" task server, backed by markdown

<p>Work is a tree: a story spawns subtasks, the subtasks block the story. taskstaka keeps the tree and tells you the one thing that is yours to do right now — and brings the story back the moment its last piece is done.</p>

<div class="install"><span class="prompt">$</span><span>brew install a6wright/taskstaka/taskstaka</span></div>
<p>then <code>taskstaka init</code>. macOS and Linux. <a href="/install">Other ways to install</a>.</p>
</div>

<img src="/shots/next.png" alt="The Next view: the task in hand on top with its notes, then each story as a stack of what you can do now" width="1280" height="800">

<div class="three">
<div><h3>A tree, not a list</h3><p>Nesting means <em>first this</em>. A task with open subtasks is blocked and stays out of your way; the deepest open leaves are what you see.</p></div>
<div><h3>The plate</h3><p>The task in hand sits on top with its notes. Finish it and the next one in the same story takes its place. Put it back and a different one comes up.</p></div>
<div><h3>Waiting on people</h3><p>A wait always has a follow-up date. Chase, record what they said, and the task that depends on them is the one that waits — not the story.</p></div>
</div>

## How a stack works

<!-- diagram:stack -->

A story is the base. Each subtask under it is a prerequisite, knocked out from the bottom. When you tick the plate in hand, the next prerequisite moves into your hand; when the last one closes, the story itself comes back to the top of Next, flagged **resurfaced**. That moment is the whole product. More in [Help](/help#the-model).

## Everywhere you work

`taskstaka next` in a terminal, `taskstaka tui` as an always-on panel, the web outliner at `http://127.0.0.1:7407/`, the macOS menu bar item with <kbd>⌥Space</kbd> quick add, and an MCP server so Claude can keep your list honest. One daemon, one folder of markdown files in `~/todo`, every surface showing the same thing. Your notes stay yours: open the folder in Obsidian, sync it with git.
