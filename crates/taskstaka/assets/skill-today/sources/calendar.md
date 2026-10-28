# calendar

Servers whose tools list or search events (names like `list_events`, `search_events`,
`get_event`, `calendar`).

**Window:** yesterday 00:00 through tomorrow 24:00 in the user's zone, one fetch.
Yesterday's events say which meetings happened (their notes and transcripts are where the
action items are: hand each event's title, time and attendees to the `folder` and `docs`
pulls as a search key). Today's say what the day looks like. Tomorrow's only matter when
the user organises or presents.

**Candidates:**
- yesterday: a meeting with a notes or doc link → `update` / `new` candidates come from
  that document, not from the event; cite the event as the *when*.
- today / tomorrow: an event the user organises or presents at (they are the organiser, or
  the title names their demo, review, talk) → `new` prep step, offered in round 5, never
  created unasked.
- an invitation unanswered that someone is waiting on → `new` with `waiting_on` unset; it
  is the user's to answer, so the *how* in the notes is the event and the ask.

**Cite:** `calendar · <organiser> · <date time> · <title> · <link>`.
**Cap:** every event in the window is read; at most eight become candidates.
