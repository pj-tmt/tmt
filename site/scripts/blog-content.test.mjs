import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import {
  articleFromSource,
  artifactHash,
  extractArticle,
  metadata,
  sha256,
  validateApproval,
  validateStoredArticle,
} from "../src/blog/content.mjs";

const meta = {
  slug: "one-post",
  title: "One & two",
  summary: "A <short> example",
  date: "2026-10-16",
  language: "en",
};
const pageId = "10000000-0000-4000-8000-000000000001";
const proposalId = "20000000-0000-4000-8000-000000000001";
const approverDevice = "30000000-0000-4000-8000-000000000001";
const actionId = "40000000-0000-4000-8000-000000000001";

test("extracts only the article and preserves intentional module code as a hashed asset", async () => {
  const source = await readFile(
    new URL("./fixtures/blog/example/source.html", import.meta.url),
    "utf8",
  );
  const result = extractArticle(articleFromSource(source));
  assert.equal(result.scripts.length, 1);
  assert.match(
    result.html,
    /<script type="module" src="\.\/scripts\/[a-f0-9]{64}\.mjs"><\/script>/,
  );
  assert.doesNotMatch(result.html, /tmt-proposal|private preview|addEventListener/);
  assert.match(Object.values(result.assets)[0].toString(), /addEventListener/);
  assert.equal(validateStoredArticle(result.html, result.assets).html, result.html);
});

test("rejects hidden executable attributes, frames, external scripts and private chrome", () => {
  const cases = [
    '<p onclick="alert(1)">X</p>',
    '<img src="data:image/svg+xml,x" alt="x">',
    '<a href="jav&#x61;script:alert(1)">X</a>',
    '<a href="data:text/html,x">X</a>',
    '<iframe src="https://example.com"></iframe>',
    '<script src="https://example.com/x.js" type="module"></script>',
    "<script>alert(1)</script>",
    '<tmt-proposal data-id="x">private</tmt-proposal>',
    '<p data-colab-annotation="x">private</p>',
    '<div class="colab-discussion">private</div>',
    '<style>@import "https://example.com/x.css";</style>',
    "<svg><foreignObject><p>bad</p></foreignObject></svg>",
  ];
  for (const body of cases)
    assert.throws(
      () => extractArticle(`<article id="blog-article">${body}</article>`),
      undefined,
      body,
    );
});

test("requires exactly one article boundary, rejecting an unrelated failure positive", () => {
  assert.equal(
    articleFromSource('<article id="blog-article"><p>yes</p></article><aside>private</aside>'),
    '<article id="blog-article"><p>yes</p></article>',
  );
  for (const source of [
    "<p>none</p>",
    "<article>wrong</article>",
    '<article id="blog-article"></article><article>other</article>',
  ])
    assert.throws(() => articleFromSource(source));
});

test("HTML drafting comments are excluded and non-raster bytes cannot masquerade as images", () => {
  const result = extractArticle(
    '<article id="blog-article"><!-- private drafting note --><p>Public</p></article>',
  );
  assert.doesNotMatch(result.html, /private drafting note/);
  const reference = "colab-attachment:" + pageId;
  assert.throws(
    () =>
      extractArticle(
        `<article id="blog-article"><img src="${reference}" alt="x"></article>`,
        new Map([[reference, { bytes: Buffer.from("<svg></svg>"), extension: "png" }]]),
      ),
    /supported raster/,
  );
});

test("copies only referenced raster assets; arbitrary paths and unreferenced files fail", () => {
  const bytes = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
  const reference = "colab-attachment:" + pageId;
  const images = new Map([[reference, { bytes, extension: "png" }]]);
  const result = extractArticle(
    `<article id="blog-article"><img src="${reference}" alt="Diagram"></article>`,
    images,
  );
  assert.equal(Object.keys(result.assets)[0], `images/${sha256(bytes)}.png`);
  assert.equal(validateStoredArticle(result.html, result.assets).html, result.html);
  assert.throws(() =>
    validateStoredArticle(result.html, { ...result.assets, "images/extra.png": bytes }),
  );
  assert.throws(() =>
    validateStoredArticle(result.html.replace("./images/", "../../images/"), result.assets),
  );
  assert.throws(() =>
    extractArticle(
      '<article id="blog-article"><img src="https://tracker.invalid/x.png" alt="x"></article>',
    ),
  );
  assert.throws(() =>
    extractArticle(
      `<article id="blog-article"><img src="${reference}" alt="x"></article>`,
      new Map([[reference, { bytes, extension: "svg" }]]),
    ),
  );
});

test("approval digest binds body, scripts, image bytes and all metadata", () => {
  const result = extractArticle(
    '<article id="blog-article"><p>Approved</p><script type="module">console.log(1)</script></article>',
  );
  const approval = {
    decision: "approved",
    pageId,
    proposalId,
    approverDevice,
    actionId,
    sourceSha256: sha256("source"),
    pageRevision: "opaque-token",
    approvedAt: "2026-10-16T12:00:00Z",
    artifactSha256: artifactHash(meta, result.html, result.assets),
  };
  assert.doesNotThrow(() => validateApproval(approval, meta, result.html, result.assets));
  for (const [changedMeta, changedHtml, changedAssets] of [
    [{ ...meta, title: "Changed" }, result.html, result.assets],
    [meta, result.html.replace("Approved", "Changed"), result.assets],
    [
      meta,
      result.html,
      Object.fromEntries(
        Object.entries(result.assets).map(([path]) => [path, Buffer.from("changed code")]),
      ),
    ],
  ])
    assert.throws(
      () => validateApproval(approval, changedMeta, changedHtml, changedAssets),
      /hash mismatch/,
    );
  for (const decision of ["pending", "declined", null])
    assert.throws(() =>
      validateApproval({ ...approval, decision }, meta, result.html, result.assets),
    );
  assert.throws(() => validateApproval(null, meta, result.html, result.assets));
});

test("validates date/language/slug and rejects unexpected metadata", () => {
  assert.deepEqual(metadata(meta), meta);
  for (const changed of [
    { slug: "../escape" },
    { date: "2026-02-30" },
    { language: "unknown" },
    { language: "zh-Hant" },
    { title: "Bad\u0000title" },
    { title: "" },
    { injected: true },
  ])
    assert.throws(() => metadata({ ...meta, ...changed }));
});
