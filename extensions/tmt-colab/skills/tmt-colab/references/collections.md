# Collections

Collections organize the owner's pages as live chapters. Create them for a deliberate reading
order or a saved All-pages search; do not copy page content into collection notes. V1 is local
to one owner's Store on one Remote. Labels such as maintainer and audience are display text,
not permission grants or notification destinations. Reader links and hosted pages do not manage
collections.

## Manual collections

```sh
tmt colab collection create 'Release reading' --maintainer 'Release agent' --json
tmt colab collection add COLLECTION PAGE#deployment --note 'Read the rollback steps' --for Ben --json
tmt colab collection show COLLECTION --json
tmt colab collection ls --json
```

Keep the returned collection and item IDs. `COLLECTION` accepts its full UUID or a unique prefix
of at least eight characters. `PAGE` uses the existing page ID/prefix lookup. An ambiguous
collection prefix lists every match and changes nothing. `show` returns one definition, its
manual items and the person's Seen marks; `ls` lists definitions, including archived collections.

Add appends a separate reference, so the same page may appear more than once. A section is an
exact HTML `id` matching `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`. It must occur exactly once;
selectors, missing IDs and duplicate IDs are unavailable. Ordinary IDs only select a passage:
they cannot create a reading, answer or review action. Existing admitted semantic blocks retain
their own controls. A reference grants no access to its page. A removed page stays unavailable
in the collection until its item is removed.

```sh
tmt colab collection remove COLLECTION ITEM --json
tmt colab collection rename COLLECTION 'Weekly reading' --json
tmt colab collection maintainer COLLECTION 'Planning agent' --json
tmt colab collection sort COLLECTION manual --json
tmt colab collection archive COLLECTION --json
tmt colab collection archive COLLECTION --restore --json
```

`ITEM` is the full item UUID. Sorting supports `manual`, `updated` and `title`; manual order is
insertion order. Archiving a collection never archives or deletes its pages. Archived collections
leave the normal directory but remain readable by ID and through the CLI.

## Smart collections

```sh
tmt colab collection create 'Reviews for Ben' --smart --label review --by 'Planning agent' --needs-you --json
tmt colab collection query COLLECTION --q release --label review --sort title --json
```

A smart collection saves the existing All-pages search, not another query language: `--q` is
literal list search, repeated `--label` requires every label, repeated `--by` matches any named
creator, `--sort` is `updated` or `title`, and `--archived-pages` selects archived pages instead
of active pages. Optional `--needs-you` includes pages with a known positive needs-you count.
No regex, body index or generated recommendations are added. Query replaces the whole saved
search, so omitted filters are removed; replacing it clears that collection's Seen marks.

The browser evaluates results against its current owner page list. The native CLI only shows
the definition. Unknown title/summary evidence stays pending; `Checking pages…` or
`Checking more pages…` never means a confirmed empty result. Refreshing the view reads again;
there is no collection polling or background page subscription.

## Person progress and bounds

Only the person uses **Mark seen** and **Clear seen** in the browser. Agents have no CLI for
these actions. Opening a page, reading source or updating a page does not mark it Seen. Seen
items stay expanded and faded until cleared; this progress is independent of Mark read,
Answer and Confirm, creates no page record, and notifies nobody. Unseen collection items never
increase Today's Needs-you count. In-place page actions keep their ordinary records and notices.

The space holds at most 64 collections, including archived ones; each manual collection holds
256 items and the space holds 1,000 manual references in total. Names hold 64 code points,
notes 512 UTF-8 bytes, and maintainer/audience labels 128 UTF-8 bytes each, without controls.
Invalid or over-cap changes are refused atomically, never truncated. Keep names and notes
short and purposeful.

Commands work while serving or stopped. If a write's acknowledgment is lost, the outcome is
unknown: inspect `collection ls` and `collection show` before making another change. Never
blindly repeat an add or create, because a fresh command creates a fresh reference or collection.
