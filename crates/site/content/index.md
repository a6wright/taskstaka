<div class="hero">
<img class="logo" src="/logo.png" alt="A pyramid of orange cubes with two tumbling off">

# A "what needs action next" task server, backed by markdown

<p>Work is a tree: a story spawns subtasks, and the subtasks block the story. taskstaka keeps the tree, shows you the one task that is yours right now, and brings the story back the moment its last piece closes.</p>

<div class="install"><span class="prompt">$</span><span>brew install a6wright/taskstaka/taskstaka</span></div>
<p>then <code>taskstaka init</code>. macOS and Linux. <a href="/install">Other ways to install</a>.</p>
</div>

<img src="/shots/next.png" alt="The Next view: the task in hand on top with its notes, then each story as a stack of what you can do now" width="1280" height="800">

<div class="three">
<div><h3>Work is a tree</h3><p>Nesting means <em>first this</em>. A task with open subtasks is blocked and stays out of your way; you see the deepest open leaves.</p></div>
<div><h3>The plate</h3><p>The task in hand sits on top with its notes. Finish it and the next one in the same story takes its place. Put it back and a different one comes up.</p></div>
<div><h3>Waiting on people</h3><p>Each wait carries a follow-up date. Chase, record what they said. The subtask that depends on them is the one that waits, and the story shows their name and how long.</p></div>
</div>

## How a stack works

<!-- diagram:stack -->

A story is the base. Each subtask under it is a prerequisite, knocked out from the bottom. Tick the plate in hand and the next prerequisite moves into your hand. Close the last one and the story comes back to the top of Next, flagged **resurfaced**. You store one status per task; taskstaka computes which tasks are blocked, which are ready and which comes next. More in [Help](/help#the-model).

## Everywhere you work

`taskstaka next` in a terminal, `taskstaka tui` as an always-on panel, the web outliner at `http://127.0.0.1:7407/`, the macOS menu bar item with <kbd>⌥Space</kbd> quick add, and an MCP server so Claude can keep your list honest. One daemon, one folder of markdown files in `~/todo`, and each surface shows the same tree. Your notes stay yours: open the folder in Obsidian, sync it with git.
