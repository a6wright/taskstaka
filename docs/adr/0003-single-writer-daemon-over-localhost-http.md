---
status: accepted
date: 2026-09-29
---

# One daemon owns the files; every client talks HTTP to it

The CLI, TUI, MCP bridge and web UI all speak JSON over HTTP to a single daemon on `127.0.0.1:7407`. Nothing but the daemon (and `Store` in tests) touches the notes directory. We chose this over "each client links the core and reads the files" because two processes writing markdown files concurrently is how you lose a task, and because a single in-memory tree is the only way `resurfaced` events and long-poll refresh can be correct.

Clients auto-start the daemon when `/health` fails; an advisory lock on `.tododo/lock` guarantees that if two clients race, exactly one daemon wins. A systemd user unit or launchd plist is the recommended always-on path; auto-start is the fallback.

## Consequences

- Phase 1 shipped the CLI in "direct" mode (core in-process, lock held for the command). That stays as `--direct` for scripts and tests; it is safe because the lock is exclusive.
- TCP rather than a Unix socket so the browser UI works. JSON API mutations accept JSON bodies only and the server sets no CORS headers, so a cross-origin page cannot perform them. The HTML form endpoints (`/w/...`) are the exception a browser *can* reach, so they refuse requests whose `Sec-Fetch-Site` is not same-origin or whose `Origin` does not match `Host`. No auth token in v1; the daemon binds loopback only.
- MCP runs as a separate stdio process (`tododo mcp`) that proxies to the daemon rather than living inside it: keeps the Claude config trivial and the daemon the sole writer.
