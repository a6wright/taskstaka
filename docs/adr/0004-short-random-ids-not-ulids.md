---
status: accepted
date: 2026-09-29
---

# Task ids are six random Crockford base32 characters, not ULIDs

Ids must be typed at a prompt (`tododo done k7x2mq`) and read in filenames and wikilinks. A ULID's first ten characters encode the timestamp, so every task created the same day shares a prefix and git-style prefix matching is useless; 26 characters is also hostile to typing. Sortability buys nothing because `created` is already in the frontmatter.

Six characters from the Crockford alphabet (`0-9 a-z` minus `i l o u`) give about a billion values; the store collision-checks against the loaded tree on creation. Prefix resolution lives in the core so every client shares the same "ambiguous" behaviour.

## Consequences

- Test fixture ids must use the alphabet (`stry00`, not `story0`), or `TaskId::from_str` rejects them. This bit us once already.
