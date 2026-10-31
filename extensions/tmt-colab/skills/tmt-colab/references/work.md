# Page work

A page's work is its sub-pages: an ordered list of the pages directly under it, plus a two-line
summary. Use them when a page drives work (a plan, a spec, a review that splits into parts): each
part is a page of its own. A page with sub-pages is still an ordinary page, and there are no tasks
or assignees. Its manual page state is independent of progress: use
`tmt colab page state PAGE todo|in-progress|done --json` to change it. Progress counts the direct
sub-pages by their own page state, never deeper levels; `tmt colab show PAGE --json` reads the
state and progress buckets. The browser shows them in the compact panel below the nav; expand it
for the sub-page list. A shared reader shows the verified state with progress unavailable.
`tmt colab ls` shows only current cached work and parent links; missing/stale entries are omitted
without reading page content. Use `subpage ls PAGE` for freshly folded work.

```sh
tmt colab subpage create PAGE --title 'Write the guide' --json   # makes a page and attaches it; prints childPage
tmt colab subpage attach PAGE CHILD --json                       # an existing page; --before SUBPAGE to place it
tmt colab subpage move PAGE CHILD --before SUBPAGE --json        # without --before it goes last
tmt colab subpage detach PAGE CHILD --json                       # the page itself stays
tmt colab subpage ls PAGE --json
tmt colab summary set PAGE --line1 'Release plan' --line2 'Read before the release' --json
```

A sub-page's progress is its page state: set it with `page state` on the sub-page itself. The
summary's first line is at most 24 characters and says where the page stands; the second is at most
60 and says what comes next.

Every change applies to the page's current revision, or to `--expected-revision` when you pass one;
`COLAB_STALE_BASE` means the page changed, so read it again. An invalid value refuses the whole
change.

## Link rules

A page has one parent, a link never closes a loop, and sub-pages nest at any depth. An attach that
breaks a rule is `COLAB_LINK_REFUSED` with a `reason`: `second-parent` (another page lists it; the
message names that page), `cycle` (the page itself or a page above it), `unavailable` (not a page
of this space) or `invalid-hierarchy` (existing links already conflict). Two writers can still race
into a page with two parents or a loop; `tmt colab show PAGE --json` flags them in `page.warnings`.
Repair them with `subpage detach`, which is always admitted.

`subpage create` makes the page first, with the title as its heading, then attaches it. If the
attach fails it prints the page it made and the `subpage attach` command that finishes; it never
searches for a page to adopt, so running it again makes another page.
