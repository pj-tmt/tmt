# Structured shell pages

A shell is an ordinary page containing structured data, never HTML. A space can
have several shells; none is its Home. The app owns the complete look. Maintain
items, order, summary and reading suggestions only; never write shell HTML,
CSS or layout. HTML source commands refuse shells with `COLAB_INPUT_INVALID`.

## Read and choose references

```sh
tmt colab shell create --title Index --json
tmt colab shell read SHELL --json
```

Create returns `pageId` and never opens a browser. Read returns `pageType:"shell"`,
`shell`, `revision`, `spaceId`, `pageId`, `title`, `epoch` and `membershipHead`.
Use that page ID as SHELL. Use real source page/reading UUIDs from the same
space as SOURCE and READING below. Generate a fresh UUID for each new item;
the example item IDs are placeholders to replace, not IDs to reuse across work.

`shell.items` maps item UUIDs to closed objects:

- Page: `{kind:"page",pageId,note?,audience?}`.
- Anchor: `{kind:"anchor",pageId,anchor,note?,audience?}`.
- Reading: `{kind:"reading",pageId,readingId,note?,audience?}`.

Items are references, never copies or access grants. Several may target the same
page. A shell target stays a link, never expands into another shell. Deleted,
unreadable or missing targets stay unavailable; do not remove them automatically.
An anchor is an exact element ID, `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`; zero or
multiple matches are unavailable, never a CSS selector. `order` lists every item
ID exactly once; actions maintain it.

## Conditional actions

For each example, save its JSON as `action.json`, then run the following command.
Before **each** mutation, read SHELL again and replace REVISION with that exact
opaque `revision`; do not reuse a previous token. `--file -` accepts stdin instead.
Actions are closed JSON bodies, not whole-shell replacements or raw Yjs updates.

`item.add` adds one item; optional `beforeId` inserts before an existing item,
otherwise it appends:

```json
{
  "itemId": "10000000-0000-4000-8000-000000000001",
  "item": { "kind": "page", "pageId": "SOURCE", "note": "Start here", "audience": "Ben" }
}
```

```sh
tmt colab shell item-add SHELL --expected-revision REVISION --file action.json --json
```

`reading.suggest` adds a reading reference, with optional `beforeId`, `note` and
`audience`. It never marks a reading read or sends a notice. Read state and the
person's Mark read action belong to the source page.

```json
{ "itemId": "10000000-0000-4000-8000-000000000002", "pageId": "SOURCE", "readingId": "READING" }
```

```sh
tmt colab shell reading-suggest SHELL --expected-revision REVISION --file action.json --json
```

`item.move` moves an existing item before `beforeId`; omitting it moves to the end:

```json
{
  "itemId": "10000000-0000-4000-8000-000000000002",
  "beforeId": "10000000-0000-4000-8000-000000000001"
}
```

```sh
tmt colab shell item-move SHELL --expected-revision REVISION --file action.json --json
```

`item.set` replaces the whole item, preserving position. Omitted note/audience
are cleared. This example changes the page reference to an anchor:

```json
{
  "itemId": "10000000-0000-4000-8000-000000000001",
  "item": { "kind": "anchor", "pageId": "SOURCE", "anchor": "plan" }
}
```

```sh
tmt colab shell item-set SHELL --expected-revision REVISION --file action.json --json
```

`summary.set` writes both lines. The system records authenticated writer and
owner time, plus available display-only agent metadata; never supply attribution.

```json
{ "line1": "Release plan", "line2": "Read before the release" }
```

```sh
tmt colab shell summary-set SHELL --expected-revision REVISION --file action.json --json
```

`summary.clear` removes both lines; it takes no file. Clearing an absent summary
is a valid no-op (`changed:false`).

```sh
tmt colab shell summary-clear SHELL --expected-revision REVISION --json
```

`item.remove` removes only the reference and its order entry, never the source page:

```json
{ "itemId": "10000000-0000-4000-8000-000000000001" }
```

```sh
tmt colab shell item-remove SHELL --expected-revision REVISION --file action.json --json
```

## Bounds and refusal recovery

At most 256 items; complete canonical compact UTF-8 JSON shell value at most
131072 bytes (128 KiB), separate from retained update/history limits. Note is
at most 512 UTF-8 bytes; audience at most 128. Summary line 1 is at most 24
Unicode code points, line 2 at most 60. Text is inert, without control characters;
omit optional fields instead of null. Page, reading and item IDs are canonical UUIDs.

Extra/missing fields, invalid types/IDs/anchors, duplicate or unknown items,
before-self, unknown placement or over-limit values refuse the **whole** action
with `COLAB_INPUT_INVALID`; the document stays unchanged. Fix the body, not the
validation. A changed revision is `COLAB_STALE_BASE`: read again, reconcile the
intent against current data and use the new revision, never blindly retry.

After timeout or uncertain publication, inspect the retained original operation
and read back before another write; never resend or switch to an offline fallback.
Caller-chosen item IDs let you check whether an add landed. Creation uncertainty
requires inspecting `tmt colab ls --json`, not creating a duplicate shell.
