# Visual source

The template adapts the TMT Colab marketing landing's system font stack,
square controls, hairline rules and spacing. Its black-only ninja palette uses
ink (`#0a0a0a`), rice-paper text (`#f2efe9`), muted text (`#8f8b84`) and one
red accent (`#e2452c`) in both color preferences. It does not import the
Colab application or the private browser component package.

Source repository: https://github.com/pj-tmt/tmt

Verified original Colab source commit: `2981a7ba178cc60c35a8c42445f11377e1e4b205`.

- `site/src/colab/colab-landing.css`, last path change
  `031b1569c427515a0e4876930a9fce04f13a2acd`.
  SHA-256: `6e94873a76261684a11a65320213e241781e45f07518a3eb9e766c5ddb845cb0`.
- The original neutral palette was inherited by that landing from `site/src/styles.css`, last
  path change `4f76003de96c1d7b1c536860769f6cdb231509fe`.
  SHA-256 at the verified main commit:
  `78b3fa6af5145b568ef907948d25faea8d617f520f3900ec346d26c74e047619`.
- `site/src/blog/mark.svg` is the owner-reviewed, hand-drawn hooded terminal
  mark based on the TMT GitHub organization avatar. It replaces the blog's former
  aurora mark; Home and Colab keep their own assets. The mark remains an external
  SVG file in development and production, never an inline data URL.
- `site/src/blog/favicon.png` is the owner-supplied team icon (hooded ninja with
  a terminal prompt and red mask), padded to square on black and resized to
  192×192. It is the blog's tab and touch icon; the header keeps `mark.svg`.
  SHA-256: `ee71f53e5e4c98fd4307a4224aaccdedeafc82bca83259799f78b3004ed5914c`.

The landing loads canonical design tokens but uses its own layout CSS. The blog
retains its editorial structure with an independent black-only treatment.
These are committed local adaptations; builds never fetch the product repository.

Upstream TMT source is MIT licensed. See LICENSE for the retained notice.
