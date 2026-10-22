## Colab landing

The English `/colab/` route is a landing design exploration within the site,
separate from Home and the retained handbook chapters. Four large feature
close-ups show annotation, agent conversation, live documents and work
organization, followed by a short installation guide. The UI close-ups are
illustrations. The route reuses the site theme and logo; the development embed
below annotates it directly.

## Local Colab embed preview

The opt-in development overlay annotates the live handbook DOM without copying it
into a Colab HTML page. Start from an isolated checkout:

```sh
VITE_COLAB_EMBED=1 TMT_EMBED_IDENTITY=<your-identity-uuid> pnpm dev --port 5185 --strictPort
```

Use the pointer icon or C to toggle Colab mode without opening a menu. The
shortcut is inactive while editing text, composing with an IME, or holding any
modifier key. The separate details icon opens Connection & threads, even when
Colab mode is off. Hover outlines a component; click it or select
text to open an anchored discussion. Clicking outside collapses it, and reopening
restores its draft. The composer stays at the bottom while messages scroll; new
turns and agent replies scroll the message area to the latest entry. Enter sends to the selected agent; Shift+Enter adds a line.
Comment stores a local-only turn. Resolve closes the bubble and hides its marker;
the discussion remains in the list for reopening. Comments and drafts stay in this browser's local storage
across reload/HMR. They are local review data, not encrypted/shared Colab records.
Missing anchors remain in the annotation list. Open one and use Reattach this
discussion, then click a component or select text to move its anchor without
changing its turns or draft. The Connection view lists local agents and opens
the existing Remote entry for user-controlled device pairing.

Repeated activation and HTTP retries share one bounded server-session send
operation, retained before dispatch across browser reload/HMR. A definite
pre-dispatch error releases the hold. An uncertain failure never redispatches
that operation within the same server session: the composer shows the frozen
message and an explicit clear action. Check TMT before clearing the hold or
restarting the preview, because the request may have reached the agent. Editing
an unacknowledged send cannot allocate a replacement operation until it is
cleared. A temporary reply lookup failure remains visible and does not stop
subsequent automatic checks.

Explicit Ask actions use the configured sender and installed TMT CLI through a
loopback-only, same-origin Vite middleware. Open discussions automatically check
for replies every three seconds. Reply lookup uses the retained TMT exchange
owned by the configured sender, so old threads can read replies after a preview
restart while TMT retains that exchange. No pairing or grants are performed by
the overlay. The development middleware and FAB are excluded from production
builds, even when the embed flag is set.

## Blog

The blog is built as complete HTML under `${SITE_BASE}blog/` alongside Home,
using the same Vite build and `site/dist` Pages artifact. The working title is
TMT Journal. Its shared layout follows the Colab landing palette; provenance is
recorded in [the blog style source note](src/blog/PROVENANCE.md).

Code belongs to `src/blog/` (UX review). Approved English articles belong to
`src/posts/<slug>/`, each with `article.html`, `meta.json`, `approval.json`,
`scripts/` and raster `images/` as needed. No article language except English is
enabled; a new language needs a repository `languageExceptions` entry and
corresponding metadata validation change.

Run from `site/`:

```sh
pnpm install --frozen-lockfile
pnpm check
pnpm build                          # production; excludes all illustrative fixtures
SITE_BASE=/ pnpm build              # custom-domain/root deployment: /blog/
pnpm blog:demo                      # explicit noindex example, empty RSS/sitemap
pnpm preview --port 4176 --strictPort
```

`SITE_BASE` defaults to `/tmt/`. `SITE_ORIGIN` defaults to
`https://pj-tmt.github.io`; it must be an HTTPS origin without a trailing slash.
The blog requires an absolute directory base. The handbook-only relative/hash
preview (`SITE_BASE=./ VITE_SITE_HISTORY=hash pnpm exec vp build`) still works,
but excludes the blog. For local article HMR use `TMT_BLOG_DEMO=1 pnpm dev`;
production artifacts always reject the development policy. Generated `blog/`
inputs are ignored and rebuilt; approved sources are never cleared by a build.

### Colab review and import

Use the installed native TMT CLI, optionally with `TMT_EXECUTABLE` set to its
absolute path. Draft source must contain exactly one `article#blog-article`.
Metadata has exactly `slug`, `title`, `summary`, `date` (YYYY-MM-DD) and
`language` (`en`). Images use full `colab-attachment:<UUID>` references.
Draft creation uses Colab's `column` layout and adds the new page to the `blog`
project, with strict markup validation and no automatic browser opening.

```sh
pnpm blog:colab draft /private/path/source.html /private/path/meta.json
pnpm blog:colab prepare PAGE_UUID /private/path/meta.json
# Ben reviews the proposal and clicks Approve from a trusted owner device.
pnpm blog:colab publish PROPOSAL_UUID
pnpm check
pnpm build
```

Prepare inventories each article module/hash and the external-request policy in
the proposal. Import checks the page identity, full-source SHA-256, proposal body,
trusted decision and artifact hash, then re-reads source before writing. Any
source change requires a new proposal. Agents must never approve proposals or
pair browsers. Colab authenticates any paired owner device, not Ben's human
identity; Ben controls those devices and performs approval operationally.

Approval evidence retains the prepared page revision. The opaque document
revision may advance when a decision or discussion is stored; an unchanged
full-source hash preserves the content binding across those updates.
The trusted decision's timestamp is retained as returned by Colab (a decimal
millisecond string); stored ISO timestamps also remain valid.

The importer reads source and authenticated proposal state through the public
CLI, never discussion exports. `.private/` review records and temporary
attachments are local; attachments are removed after capture. Do not commit full
Colab pages, exports, conversations or pending drafts. CI checks canonical
article/assets and recorded hashes; it cannot authenticate Colab without private
keys. Review the content and approval evidence in the PR.

Intentional `type=module` snippets become content-addressed source files, then
Vite bundles them into external assets. They must be self-contained: build-time
imports are rejected. The existing blog CSP allows same-origin scripts/styles
and raster/data images, blocks connections, frames and forms, and adds no
external sources. Article code is trusted reviewed code on the same origin as
Home; it can access that origin's DOM/storage. Do not include secrets or assume
an article is sandboxed. Inline event handlers and external resource URLs are
rejected; inline article CSS cannot load resources.

Import refuses to overwrite an existing post. For a revision, privately back up
the current post, prepare and approve a new proposal, remove only that post's
tracked directory in the feature branch and import again. Review the complete
diff; restore the backup if import fails. Never edit committed article/assets or
metadata without renewing approval.

Only the existing `Build site` job checks/builds and verifies the output before
upload. Production verification is the default; demo requires `--demo` and
is rejected by the upload gate. Main merges follow the existing Pages workflow;
PRs never deploy. Publication timing remains Ben's decision.
