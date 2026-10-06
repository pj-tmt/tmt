---
name: tmt-design
description: Build, check and translate the handbook site in `site/` (Vite, MDX, per-language pages) and its design tokens. Load when editing site/**, design/tokens, handbook chapters or translations. Owner - tmt-design.
---

# Handbook site

Static site in `site/` with its own lockfile outside the TypeScript workspace.
The public site renders only the v9 Home (`home/HomePage.tsx`), including installation,
workflow and Squad/Colab illustrations. Layout owns theme, language and anchor scrolling;
the router normalizes old chapter/unknown paths to the localized Home. Retained MDX,
chapter components and translations are source-only for future refinement: the public
entry point does not import or render them. Do not restore chapter exposure implicitly.
`scripts/spa-routes.mjs` publishes four Home entries with Home-only alternates/canonicals,
noindex redirect stubs for historical chapter/zh paths, and a noindex Home-app 404 fallback.

Merging `site/**` or `design/tokens/**` to `main` deploys to GitHub Pages
(`.github/workflows/site.yml`); PRs never deploy. Existing tokens and source-only chapter
scene behavior remain owned here. No handbook prose is authored by OpenAI members.

```sh
cd site
pnpm install --frozen-lockfile
pnpm dev                        # http://127.0.0.1:5173/tmt/
pnpm check                      # types, Vite+ lint/format, MDX imports, translation sync
pnpm build                      # four public Homes and historical redirect stubs
SITE_BASE=/ pnpm build          # root path or custom domain
SITE_BASE=./ VITE_SITE_HISTORY=hash pnpm exec vp build   # preview at an unknown path
```

Run `pnpm docs:format:check` from `typescript/` before pushing doc edits; bare
prettier skips docs. A blank line inside `<Code>`/`<Cmd>` in MDX lets the formatter
swap `*` and `_` in samples: grep and run samples after formatting.

## Translations

English is the source. Languages are `ja`, `zh-hant`, `zh-hans`; a language is
allowed once `languageExceptions` in `.github/repository-layout.json` lists its
directory, and [AGENTS](../../../AGENTS.md#repository-content-language) owns the
exception (front matter, keys, code, comments, tests and commits stay English).

- Public Home exists at `/`, `/ja/`, `/zh-hant/` and `/zh-hans/`; `/zh/` redirects
  to Traditional Chinese Home. Historical chapter URLs resolve to Home in the same
  locale. The language dropdown sets `<html lang>` and preserves Home anchors.
- A translation's front matter is kept out of the page and exported as `frontmatter`;
  `scripts/mdx-imports.mjs` checks translated pages too.
- A page `site/src/chapters/<page>.mdx` is translated as
  `site/src/i18n/<lang>/<page>.mdx` with front matter `source`, `sourceRevision`
  (`git hash-object site/src/chapters/<page>.mdx`) and `title`. After updating a
  translation set `sourceRevision` to the new blob SHA. A stale source is a warning
  (CI annotation); missing front matter, a wrong `source` or a malformed
  `sourceRevision` fails `pnpm check`. A page without a file falls back to English.
- UI and home strings override `site/src/lang/strings.ts` key by key in
  `site/src/i18n/<lang>/strings.json`, whose reserved `"$source"` object
  (`source` and `sourceRevision` of `strings.ts`) is never merged and is checked the
  same way.
- Keep in English: command names, flags and ids, `talk`, `reply`, `receipt`, board
  marks, sample output and code. A translated heading keeps the English slug as an
  explicit id (`<h3 id="install">安裝</h3>`), because `slug()` drops non-Latin text.
- `zh-hant` is Traditional Chinese with Taiwan usage and also keeps `agent`,
  `driver`, `harness`, `board`, `colab`, `meet` (窗格 pane, 終端機 terminal, 擴充套件
  extension, 卡住 blocked, 恢復 resume). `zh-hans` is translated directly from English
  with Mainland usage (文件, 设置, 程序, 服务器, 默认).
- `scripts/spa-routes.mjs` writes Home-only `<html lang>` and `hreflang` alternates;
  `SITE_ORIGIN` makes them fully qualified. Retained chapters do not get alternates.

## Reviewing translations

Before approving a translation PR, check that every `sourceRevision` equals the English blob
on the base, that heading ids, links and `<Cmd>`/`<Code>` bytes equal the English, and look at
light captures at 1440 and 390 pixels wide. CLI-look reviews apply
[`design/cli-style.md`](../../../design/cli-style.md).
