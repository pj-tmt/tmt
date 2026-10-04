---
name: tmt-design
description: Build, check and translate the handbook site in `site/` (Vite, MDX, per-language pages) and its design tokens. Load when editing site/**, design/tokens, handbook chapters or translations. Owner - tmt-design.
---

# Handbook site

Static site in `site/` with its own lockfile outside the TypeScript workspace.
Chapters are `site/src/chapters/*.mdx`, registered with the page tree in
`site/src/chapters/index.ts`; a chapter opens with one animated scene
(`site/src/scenes/`, `site/src/chapter-scenes/`) whose words live in `site/src/lang/strings.ts`.
Scenes advance only while on screen and rest on one complete frame under
`prefers-reduced-motion`: new scenes must keep that. `index.html` asks Google Fonts for
the token mono family's glyphs of the marks `●○◌◆✗✓↻▸◐` (the latin subset has none);
the family lacks `○✗✓↻`, which fall back to the system monospace font; colors, fonts and marks come from
`design/tokens/tokens.json`. Anything not in a release is marked planned. Merging a
change under `site/**` or `design/tokens/**` to `main` deploys to GitHub Pages
(`.github/workflows/site.yml`; the repository is public). PRs never deploy.

```sh
cd site
pnpm install --frozen-lockfile
pnpm dev                        # http://127.0.0.1:5173/tmt/
pnpm check                      # types, Vite+ lint/format, MDX imports, translation sync
pnpm build                      # dist/ for Pages, one index.html per route
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

- Every page exists at its path and under `/ja/`, `/zh-hant/` and `/zh-hans/`; published
  `/zh/` redirects to `/zh-hant/` preserving page, query and anchor, and the language
  dropdown sets `<html lang>` (`zh-hant` is `zh-Hant`, `zh-hans` is `zh-Hans`).
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
- `scripts/spa-routes.mjs` writes each language's route files with `<html lang>` and
  `hreflang` alternates; `SITE_ORIGIN` makes them fully qualified.

## Reviewing translations

Before approving a translation PR, check that every `sourceRevision` equals the English blob
on the base, that heading ids, links and `<Cmd>`/`<Code>` bytes equal the English, and look at
light captures at 1440 and 390 pixels wide. CLI-look reviews apply
[`design/cli-style.md`](../../../design/cli-style.md).
