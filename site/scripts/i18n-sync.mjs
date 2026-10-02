// Keeps handbook translations honest (#1116). English is the source: a translated
// page names its English chapter and the git blob SHA it was written from, and a
// page whose source has changed since is reported as stale. Stale is a warning, not
// a failure; a malformed or dangling page is a bug and fails the check.
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SOURCE_PREFIX = "site/src/chapters/";
const BLOB_SHA = /^[0-9a-f]{40}$/;
// A language's UI strings follow the English strings the same way pages follow
// chapters: strings.json carries a reserved "$source" claim (see lang/strings.ts).
const STRINGS_SOURCE = "site/src/lang/strings.ts";
const STRINGS_CLAIM = "$source";

/** The id `git hash-object` gives these bytes, so no git history is needed. */
export function blobSha(bytes) {
  return createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

/** Flat `key: value` pairs of a leading `---` block, or null when there is none. */
export function readFrontMatter(text) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/.exec(text);
  if (!match) return null;
  const fields = {};
  for (const line of match[1].split(/\r?\n/)) {
    const pair = /^([A-Za-z][\w-]*):[ \t]*(.*?)[ \t]*$/.exec(line);
    if (!pair) continue;
    fields[pair[1]] = pair[2].replace(/^(["'])(.*)\1$/, "$2");
  }
  return fields;
}

const mdxFiles = (directory) =>
  existsSync(directory) ? readdirSync(directory).filter((name) => name.endsWith(".mdx")) : [];

/**
 * Compares one translated file's source claim with the English file it names.
 * `expectedSource` is the only source the file may name.
 */
function checkSource(repository, path, fields, expectedSource, { errors, stale }) {
  const { source, sourceRevision } = fields;
  if (source !== expectedSource) {
    errors.push({ path, message: `source must be ${expectedSource}` });
    return;
  }
  if (!existsSync(join(repository, source))) {
    errors.push({ path, message: `source ${source} does not exist` });
    return;
  }
  if (!BLOB_SHA.test(sourceRevision ?? "")) {
    errors.push({
      path,
      message: `sourceRevision must be a 40-character git blob SHA (git hash-object ${source})`,
    });
    return;
  }
  const current = blobSha(readFileSync(join(repository, source)));
  if (current !== sourceRevision) {
    stale.push({
      path,
      message: `is stale: written from ${sourceRevision.slice(0, 7)}, ${source} is now ${current.slice(0, 7)} (update the translation, then set sourceRevision to ${current})`,
    });
  }
}

/**
 * @param repository absolute repository root
 * @param languages directory (repository relative) to HTML language tag, from the
 *   layout allowlist's `languageExceptions`
 */
export function checkTranslations(repository, languages) {
  const results = { errors: [], stale: [], untranslated: [] };
  const { errors, untranslated } = results;
  const chapters = mdxFiles(join(repository, SOURCE_PREFIX));
  for (const directory of Object.keys(languages)) {
    const translated = new Set(mdxFiles(join(repository, directory)));
    for (const name of translated) {
      const path = `${directory}/${name}`;
      const fields = readFrontMatter(readFileSync(join(repository, path), "utf8"));
      if (!fields) {
        errors.push({ path, message: "starts without front matter (source, sourceRevision)" });
        continue;
      }
      checkSource(repository, path, fields, `${SOURCE_PREFIX}${name}`, results);
    }
    for (const name of chapters) {
      if (!translated.has(name)) untranslated.push(`${directory}/${name}`);
    }
    const stringsPath = `${directory}/strings.json`;
    if (existsSync(join(repository, stringsPath))) {
      let data;
      try {
        data = JSON.parse(readFileSync(join(repository, stringsPath), "utf8"));
      } catch {
        errors.push({ path: stringsPath, message: "is not valid JSON" });
        continue;
      }
      const claim = data?.[STRINGS_CLAIM];
      if (!claim || typeof claim !== "object" || Array.isArray(claim)) {
        errors.push({
          path: stringsPath,
          message: `needs a top-level "${STRINGS_CLAIM}" object with source and sourceRevision`,
        });
        continue;
      }
      checkSource(repository, stringsPath, claim, STRINGS_SOURCE, results);
    }
  }
  return results;
}

function main() {
  const repository = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const layout = JSON.parse(
    readFileSync(join(repository, ".github", "repository-layout.json"), "utf8"),
  );
  const { errors, stale, untranslated } = checkTranslations(
    repository,
    layout.languageExceptions ?? {},
  );
  const annotate = process.env.GITHUB_ACTIONS === "true";
  // Escape the characters GitHub's workflow commands treat as delimiters.
  const escape = (text) => text.replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
  for (const [level, items] of [
    ["error", errors],
    ["warning", stale],
  ]) {
    for (const { path, message } of items) {
      if (annotate)
        console.log(`::${level} file=${path},title=Handbook translation::${escape(message)}`);
      console.log(`${level}: ${path} ${message}`);
    }
  }
  if (untranslated.length) {
    console.log(
      `info: ${untranslated.length} chapter page(s) not translated yet (English fallback).`,
    );
  }
  if (errors.length) process.exit(1);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();
