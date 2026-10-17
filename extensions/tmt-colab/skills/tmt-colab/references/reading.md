# Recommended reading

After creating or updating a page, add reading anchors beside passages the reader must not miss.
Name the agent asking and give a short reason. Use ordinary inert HTML, not a command:

```html
<tmt-read
  data-id="10000000-0000-4000-8000-000000000401"
  data-agent="Your agent name"
  data-why="Read before choosing a deployment"
>
  <h2 data-title>Deployment requirements</h2>
  <p>The passage the reader needs to read.</p>
</tmt-read>
```

Use a unique canonical lowercase UUID, shared with question/group/review IDs. Keep that ID for
the same request. Give a changed passage a new ID when it needs a fresh read. Exactly one direct
child has `data-title`; its normalized text is 1–200 code points. The agent name is required,
1–64 code points, and must resolve uniquely in the agent directory for notification. The reason
is required, 1–200 code points. Do not put control characters in those fields. Keep at most
50 anchors per page. Do not nest them with reading anchors, questions, question groups or review
sections. Run `tmt colab page check page.html --json` before publishing.

Colab draws the band and its trusted `Mark read` action. Page HTML, scroll position and your own
thread resolution cannot mark a passage read. The person's action records an ordinary anchored
thread comment and a person Resolve, then attempts to notify only the named agent once. A
notification can be unavailable or uncertain; do not infer delivery from the read state, and do
not automatically resend. Read the ordinary thread with `tmt colab threads PAGE --json`.
The opening comment contains `Read “<heading>”` and `Reading: <full UUID>`.

`Next ↓` moves to the next unread passage without recording anything. An owner/paired page URL
may append `&read=<full UUID>` to its existing `#space=…&path=…` fragment to open at a passage.
This is only navigation, never admission. Do not put it into a hosted reader link or remove a
reader seed to manufacture an owner link. A missing or removed target does nothing; its old
ordinary thread remains. There is no separate request store.

The [recommended-reading contract](https://github.com/pj-tmt/tmt/blob/main/extensions/tmt-colab/contracts/colab-v1.md#recommended-reading)
owns the exact grammar, state and notification guarantee.
