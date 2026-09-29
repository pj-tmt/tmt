import { readFileSync } from 'node:fs';

/**
 * Waits until `file` exists and holds content, then returns the content.
 *
 * The file existing is not a readiness signal: a shell redirect (`cmd > file`)
 * and `writeFileSync` both create it before writing, so a reader that polls
 * `existsSync` and then reads can see an empty file. This is for files that are
 * written once and never legitimately empty, such as an exit status or a small
 * JSON document.
 */
export async function waitForFileContent(
  file: string,
  { timeoutMs = 5_000, description = `content in ${file}` } = {}
): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const content = readContent(file);
    if (content !== '') return content;
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for ${description}.`);
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
}

function readContent(file: string): string {
  try {
    return readFileSync(file, 'utf8');
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return '';
    throw error;
  }
}
