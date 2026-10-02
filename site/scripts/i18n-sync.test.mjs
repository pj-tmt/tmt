import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { blobSha, checkTranslations, readFrontMatter } from "./i18n-sync.mjs";

const languages = { "site/src/i18n/ja": "ja", "site/src/i18n/zh": "zh-Hant" };

// A repository root with English chapters and optional translated pages.
function fixture(files, run) {
  const root = mkdtempSync(join(tmpdir(), "i18n-sync-"));
  try {
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    return run(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

const page = (source, sourceRevision) =>
  `---\nsource: ${source}\nsourceRevision: ${sourceRevision}\ntitle: "Start: 開始"\n---\n\n本文\n`;

test("blobSha equals git hash-object, with no history needed", () => {
  for (const text of ["", "one line\n", "日本語のページ\n"]) {
    const expected = execFileSync("git", ["hash-object", "--stdin"], { input: text })
      .toString()
      .trim();
    assert.equal(blobSha(Buffer.from(text)), expected);
  }
});

test("readFrontMatter reads flat pairs, strips quotes and ignores other lines", () => {
  assert.deepEqual(readFrontMatter(page("site/src/chapters/start.mdx", "a".repeat(40))), {
    source: "site/src/chapters/start.mdx",
    sourceRevision: "a".repeat(40),
    title: "Start: 開始",
  });
  assert.equal(readFrontMatter("# No front matter\n"), null);
  assert.equal(readFrontMatter("text\n---\nsource: x\n---\n"), null);
});

test("a translation written from the current English page is fresh", () => {
  const english = "# Start\n";
  const result = fixture(
    {
      "site/src/chapters/start.mdx": english,
      "site/src/i18n/ja/start.mdx": page(
        "site/src/chapters/start.mdx",
        blobSha(Buffer.from(english)),
      ),
    },
    (root) => checkTranslations(root, languages),
  );
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.stale, []);
});

test("a changed English page makes its translation stale, as a warning and not an error", () => {
  const result = fixture(
    {
      "site/src/chapters/start.mdx": "# Start, edited\n",
      "site/src/i18n/ja/start.mdx": page(
        "site/src/chapters/start.mdx",
        blobSha(Buffer.from("# Start\n")),
      ),
    },
    (root) => checkTranslations(root, languages),
  );
  assert.deepEqual(result.errors, []);
  assert.equal(result.stale.length, 1);
  assert.equal(result.stale[0].path, "site/src/i18n/ja/start.mdx");
  assert.match(result.stale[0].message, /is stale: written from \w{7}, .* is now \w{7}/);
});

test("untranslated chapters are listed per language and are not errors", () => {
  const english = "# Start\n";
  const result = fixture(
    {
      "site/src/chapters/start.mdx": english,
      "site/src/chapters/squad.mdx": "# Squad\n",
      "site/src/i18n/ja/start.mdx": page(
        "site/src/chapters/start.mdx",
        blobSha(Buffer.from(english)),
      ),
    },
    (root) => checkTranslations(root, languages),
  );
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.untranslated.sort(), [
    "site/src/i18n/ja/squad.mdx",
    "site/src/i18n/zh/squad.mdx",
    "site/src/i18n/zh/start.mdx",
  ]);
});

test("malformed pages are errors, each for its own reason", () => {
  const sha = "b".repeat(40);
  const cases = {
    "start.mdx": ["# no front matter\n", /without front matter/],
    "squad.mdx": [page("site/src/chapters/start.mdx", sha), /source must be .*squad\.mdx/],
    "working.mdx": [page("site/src/chapters/working.mdx", sha), /does not exist/],
    "drivers.mdx": [
      page("site/src/chapters/drivers.mdx", "not-a-sha"),
      /40-character git blob SHA/,
    ],
  };
  const files = { "site/src/chapters/start.mdx": "x", "site/src/chapters/squad.mdx": "x" };
  files["site/src/chapters/drivers.mdx"] = "x";
  for (const [name, [text]] of Object.entries(cases)) files[`site/src/i18n/zh/${name}`] = text;
  const result = fixture(files, (root) => checkTranslations(root, languages));
  assert.deepEqual(result.stale, []);
  const byPath = Object.fromEntries(result.errors.map((error) => [error.path, error.message]));
  assert.equal(Object.keys(byPath).length, 4);
  for (const [name, [, pattern]] of Object.entries(cases)) {
    assert.match(byPath[`site/src/i18n/zh/${name}`], pattern);
  }
});
