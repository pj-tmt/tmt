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

The blog is built as complete HTML under `${SITE_BASE}blog/`. The full Vite build
remains in `site/dist`; `node src/blog/pages.mjs` creates the blog-only
`site/dist-pages` directory for GitHub Pages. It retains `blog/**` and only the
shared assets those pages reference. Home, locale Homes, handbook redirects and
Colab remain in source and the full build, but are not published. The Pages root
redirects to the blog with a meta refresh and visible link; `404.html` links to
the blog and repository. Blog header/footer outbound links use the repository. The working title is
TMT Journal. Its shared layout uses a black-only ninja palette in both color
preferences, rice-paper text, a red accent and a file-backed hooded mark.
Provenance is recorded in [the blog style source note](src/blog/PROVENANCE.md).

Marketing owns and reviews the blog end to end: code in `src/blog/` and approved
articles in `src/posts/<slug>/`. English (`en`) stays at the slug root. Each
translation has its own `ja/`, `zh-hant/` or `zh-hans/` subdirectory with the same
`article.html`, `meta.json`, `approval.json`, `scripts/` and raster `images/` files
as needed. Metadata `slug` identifies the English post and `language` identifies
the variant. English must exist before importing translations; each language
requires its own proposal and trusted owner approval of its exact bytes.

English URLs remain `${SITE_BASE}blog/` and `blog/posts/<slug>/`. Other languages
use `blog/<language>/` and `blog/<language>/posts/<slug>/`. The link switcher keeps
the article slug and selects an index language; it needs no client script. Shell
labels remain English and declare `lang="en"`; article HTML uses `en`, `ja`,
`zh-Hant` or `zh-Hans` as appropriate. Indexes show the approved translation or
label an English fallback. A missing article translation renders English with a
visible notice, English canonical URL and `noindex,follow`; it is excluded from
article alternates, feeds and the sitemap. A language index with zero approved
translations also falls back to English: `lang="en"`, English canonical, visible
notice and `noindex,follow`; it is excluded from index alternates and the sitemap.
Each language has its own `feed.xml` with its RSS language tag and only approved
variants; an empty feed links to the English index. The sitemap lists available
language indexes and actual approved article URLs, never fallback URLs.

Before committing a translation, add its exact directory (for example
`site/src/posts/example-post/ja`: `ja`) to the repository `languageExceptions` map
in the same PR, with Core review. Only article prose and metadata title/summary
may use that language; keys, code and comments remain English. The layout guard
requires the complete article/metadata/approval bundle; wildcard and empty
allowances are rejected. Each translation is delivered as a separate approved post PR.

Run from `site/`:

```sh
pnpm install --frozen-lockfile
pnpm check
export SITE_ORIGIN=https://example.com
export SITE_REPOSITORY=owner/repository
pnpm build                          # full production build; no illustrative fixtures
node src/blog/pages.mjs              # create and list dist-pages
node src/blog/verify-dist.mjs --pages # verify the exact deployment directory
SITE_BASE=/ pnpm build               # root deployment: /blog/
pnpm blog:demo                      # explicit noindex example, empty RSS/sitemap
pnpm exec vp preview --outDir dist-pages --port 4176 --strictPort
```

`SITE_BASE` defaults to `/tmt/`. A production build requires `SITE_ORIGIN`, the
HTTPS origin the site is served from, without a trailing slash or path; it has no
default, because it is the host in the blog's canonical links, RSS and sitemap.
`.github/workflows/site.yml` derives it from the repository owner
(`https://<owner>.github.io`), so a custom domain is the one value there.
`SITE_REPOSITORY` is `owner/repository`; it overrides `GITHUB_REPOSITORY`, which
Actions supplies. Production builds require one of them and derive the GitHub
URL without hard-coding an owner. Local dev/demo runs use a neutral example
repository when neither is set. No DNS or install-script serving changes are
made by this pipeline: `install.tmt.ninja` serves the installer from Releases.
`verify-dist` takes the same `SITE_ORIGIN` and fails a production artifact whose
canonical links, RSS or sitemap name another origin. The dev server, `pnpm blog:demo`
and the unit tests need none and use a neutral local origin.
The blog requires an absolute directory base. The handbook-only relative/hash
preview (`SITE_BASE=./ VITE_SITE_HISTORY=hash pnpm exec vp build`) still works,
but excludes the blog. For local article HMR use `TMT_BLOG_DEMO=1 pnpm dev`;
production artifacts always reject the development policy. Generated `blog/`
inputs are ignored and rebuilt; approved sources are never cleared by a build.

### Tags

Optional `tags` metadata holds zero to three unique lowercase slugs from the
fixed vocabulary in [tags.mjs](src/blog/tags.mjs): `announcements` (project news),
`colab` (Colab use), `design` (interface design), `engineering` (implementation)
and `workflow` (working with agents). Unknown, duplicate or excessive tags fail
validation; values are never silently normalized. Omitting tags preserves the
existing metadata and approval hash. Adding or changing tags requires a new
approved proposal, whose body lists the tags.

Posts and listing rows show ordinary `#tag` links. The build generates
`${SITE_BASE}blog/tags/<tag>/` for English, or `blog/<language>/tags/<tag>/` for
translations, listing only matching approved variants in that language. Tags on
English fallback articles link to the English topic page. Unused tag/language
combinations have no empty page and no switcher link; tag alternates include only
languages with real matching posts. These pages have their own
canonical URLs and sitemap entries, and each RSS item includes its tags as
`category` elements. There is no client-side search or separate deployment.

### Colab review and import

Use the installed native TMT CLI, optionally with `TMT_EXECUTABLE` set to its
absolute path. Draft source must contain exactly one `article#blog-article`.
Metadata requires `slug`, `title`, `summary`, `date` (YYYY-MM-DD) and
`language` (`en`, `ja`, `zh-hant` or `zh-hans`), with optional `tags` as described above. Images use full `colab-attachment:<UUID>` references.
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

Import refuses to overwrite an existing language variant. For a revision, privately
back up the variant, prepare and approve a new proposal, remove that variant's
tracked directory in the feature branch and import again. For English, back up
the entire slug directory first; after importing the approved English revision,
restore its translation directories unchanged before checking the result.
Review the complete diff and restore the backup if import fails. Changing English
does not approve or rewrite existing translations. Never edit committed
article/assets or metadata without renewing approval.

Only the existing `Build site` job checks/builds, prunes and verifies the blog-only
output before uploading `dist-pages`. `verify-dist --pages` rejects non-blog
HTML, unused shared assets, missing HTML/CSS/module dependencies, invalid root
redirect/404 links and a mismatched repository. Production verification is the default; demo requires `--demo` and
is rejected by the upload gate. Main merges follow the existing Pages workflow;
PRs never deploy. Publication timing remains Ben's decision.
