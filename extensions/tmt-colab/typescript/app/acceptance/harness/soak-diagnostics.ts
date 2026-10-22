import fs from 'node:fs';
import path from 'node:path';
import policy from './soak-policy.json' with { type: 'json' };
import type { SoakDictionary } from './soak-literals.mjs';

const row = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
const array = (value: unknown) => (Array.isArray(value) ? value : []);
const time = (value: unknown): value is number =>
  Number.isSafeInteger(value) && (value as number) >= 0 && (value as number) <= 1_800_000;
const sources = ['core', 'remote', 'colab'];
const spanNames = new Set(Object.values(policy.api));

/** This same closed reconstruction runs before attachment and independently after report input. */
export function projectSoakFailure(input: unknown, dictionary: SoakDictionary) {
  const value = row(input);
  if (value.version !== 1 || value.scenario !== policy.id || !time(value.observedMs)) return;
  const observed = value.observedMs;
  const cutoff = Math.max(0, observed - policy.timelineMs);
  const flags = row(value.truncated);
  const truncated = {
    spans: flags.spans === true,
    logs: flags.logs === true,
    input: flags.input === true || array(value.children).length > 16,
    bytes: flags.bytes === true,
  };
  const error = (input: unknown) => {
    const value = row(input);
    const code =
      typeof value.code === 'string' && dictionary.codes.has(value.code) ? value.code : null;
    const message =
      typeof value.message === 'string' && dictionary.messages.has(value.message)
        ? value.message
        : null;
    return { code, message, unrecognized: code === null && message === null };
  };
  const spans = array(value.spans)
    .slice(-512)
    .flatMap((input) => {
      const span = row(input);
      if (
        typeof span.name !== 'string' ||
        !spanNames.has(span.name) ||
        !time(span.startMs) ||
        !time(span.durationMs)
      )
        return [];
      const end = Math.min(observed, span.startMs + span.durationMs);
      if (span.startMs > observed || end < cutoff) return [];
      if (span.startMs < cutoff) truncated.spans = true;
      const startMs = Math.max(cutoff, span.startMs);
      return [{ name: span.name, startMs, durationMs: end - startMs }];
    })
    .sort((a, b) => a.startMs - b.startMs);
  if (array(value.spans).length > 512 || spans.length > policy.spans) truncated.spans = true;
  const keptSpans = spans.slice(-policy.spans);
  const logs = array(value.logs)
    .slice(-256)
    .flatMap((input) => {
      const log = row(input);
      if (
        typeof log.source !== 'string' ||
        !sources.includes(log.source) ||
        typeof log.event !== 'string' ||
        !['error', 'exit'].includes(log.event) ||
        !time(log.atMs) ||
        log.atMs > observed
      )
        return [];
      const span = [...keptSpans]
        .reverse()
        .find(
          (span) =>
            span.startMs <= (log.atMs as number) &&
            span.startMs + span.durationMs >= (log.atMs as number),
        );
      return [
        {
          source: log.source as string,
          event: log.event as string,
          atMs: log.atMs,
          ...error(log),
          span: span?.name ?? null,
        },
      ];
    })
    .sort((a, b) => a.atMs - b.atMs);
  if (array(value.logs).length > 256 || logs.length > policy.tailLines) truncated.logs = true;
  const children = array(value.children)
    .slice(0, 16)
    .flatMap((input, index) => {
      const child = row(input);
      if (
        typeof child.role !== 'string' ||
        !policy.roles.includes(child.role) ||
        typeof child.alive !== 'boolean'
      )
        return [];
      const exitCode =
        Number.isSafeInteger(child.exitCode) &&
        (child.exitCode as number) >= 0 &&
        (child.exitCode as number) <= 255
          ? (child.exitCode as number)
          : null;
      const signal =
        typeof child.signal === 'string' && policy.signals.includes(child.signal)
          ? child.signal
          : null;
      return [{ index, role: child.role, alive: child.alive, exitCode, signal }];
    });
  const alive = row(value.alive);
  const assertionId =
    typeof value.assertion === 'string' ? value.assertion : row(value.assertion).id;
  const assertion =
    typeof assertionId === 'string' && Object.hasOwn(dictionary.assertions, assertionId)
      ? { id: assertionId, text: dictionary.assertions[assertionId] }
      : { id: 'unknown', text: 'The scenario stopped outside a named assertion.' };
  const result = {
    version: 1,
    scenario: policy.id,
    observedMs: observed,
    assertion,
    error: error(value.error),
    spans: keptSpans,
    logs: logs.slice(-policy.tailLines),
    children,
    alive: {
      socket: alive.socket === true,
      door: alive.door === true,
      method: 'owned-resource-probe',
    },
    truncated,
  };
  while (
    Buffer.byteLength(JSON.stringify(result)) > policy.maxBytes &&
    (result.spans.length || result.logs.length)
  ) {
    truncated.bytes = true;
    if (result.spans.length) result.spans.shift();
    else result.logs.shift();
  }
  if (Buffer.byteLength(JSON.stringify(result)) > policy.maxBytes) return;
  return result;
}

/** Read only the end of one exact, already-owned log file; never follow a symbolic link. */
export function readSoakTail(root: string, name: string) {
  const empty = { lines: [] as string[], available: false, truncated: false };
  if (!/^[a-z0-9-]{1,80}\.stderr$/.test(name)) return empty;
  let fd: number | undefined;
  try {
    const directory = fs.lstatSync(root);
    if (
      !directory.isDirectory() ||
      directory.isSymbolicLink() ||
      !Number.isInteger(fs.constants.O_NOFOLLOW)
    )
      return empty;
    const file = path.join(root, name);
    const named = fs.lstatSync(file);
    if (!named.isFile() || named.isSymbolicLink()) return empty;
    fd = fs.openSync(file, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
    const opened = fs.fstatSync(fd);
    if (
      !opened.isFile() ||
      opened.dev !== named.dev ||
      opened.ino !== named.ino ||
      opened.nlink !== 1
    )
      return empty;
    const bytes = Buffer.alloc(Math.min(opened.size, policy.tailBytes));
    const offset = Math.max(0, opened.size - bytes.length);
    const count = fs.readSync(fd, bytes, 0, bytes.length, offset);
    let content = bytes.subarray(0, count).toString('utf8');
    if (offset) content = content.includes('\n') ? content.slice(content.indexOf('\n') + 1) : '';
    const lines = content.split('\n').filter(Boolean);
    return {
      lines: lines.slice(-policy.tailLines),
      available: true,
      truncated: offset > 0 || lines.length > policy.tailLines,
    };
  } catch {
    return empty;
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
}

/** Private fixture log only: fixed name, no command arguments or authority material. */
export function appendSoakInvocation(
  root: string,
  source: 'core' | 'remote' | 'colab',
  exitCode: number | null,
  signal: string | null,
  output: string,
) {
  let fd: number | undefined;
  try {
    if (!/^tmt-acc-[a-zA-Z0-9-]+$/.test(path.basename(root))) return;
    const directory = fs.lstatSync(root);
    if (
      !directory.isDirectory() ||
      directory.isSymbolicLink() ||
      !Number.isInteger(fs.constants.O_NOFOLLOW)
    )
      return;
    fd = fs.openSync(
      path.join(root, 'soak-native.stderr'),
      fs.constants.O_WRONLY |
        fs.constants.O_APPEND |
        fs.constants.O_CREAT |
        fs.constants.O_NOFOLLOW,
      0o600,
    );
    const stat = fs.fstatSync(fd);
    if (!stat.isFile() || stat.nlink !== 1) return;
    const atMs = Date.now();
    const records: Record<string, unknown>[] = [{ source, event: 'exit', exitCode, signal, atMs }];
    try {
      const failure = row(row(JSON.parse(output.slice(-policy.tailBytes))).error);
      if (typeof failure.code === 'string' && failure.code.length <= 64)
        records.push({
          source,
          event: 'error',
          code: failure.code,
          message:
            typeof failure.message === 'string' && failure.message.length <= 200
              ? failure.message
              : null,
          atMs,
        });
    } catch {
      /* Human output is not parsed as a code or private diagnostic. */
    }
    fs.writeSync(fd, records.map((record) => JSON.stringify(record)).join('\n') + '\n');
  } catch {
    /* An optional observation never replaces the original child outcome. */
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
}
