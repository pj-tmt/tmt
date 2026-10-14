import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { readCompleteJsonLines, readMockEvents } from '../e2e/harness/readiness.js';

function withLog(run: (log: string) => void) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-complete-log-'));
  try {
    run(path.join(root, 'mock-agent.jsonl'));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

describe('newline-committed mock log events', () => {
  it('yields only complete events when the last event is cut mid-string, then reads its completion', () => {
    withLog((log) => {
      const ready = { event: 'ready' };
      const submitted = { event: 'submitted', body: 'a split message' };
      fs.writeFileSync(log, `${JSON.stringify(ready)}\n{"event":"submitted","body":"a split`);
      expect(readCompleteJsonLines(log)).toEqual([ready]);
      expect(readMockEvents(log)).toEqual([ready]);
      fs.appendFileSync(log, ' message"}');
      // Even valid JSON is not committed until the writer appends its newline.
      expect(readCompleteJsonLines(log)).toEqual([ready]);
      fs.appendFileSync(log, '\n');
      expect(readCompleteJsonLines(log)).toEqual([ready, submitted]);
      expect(readMockEvents(log)).toEqual([ready, submitted]);
    });
  });

  it('throws with the complete offending line and the original JSON error, even before a partial tail', () => {
    withLog((log) => {
      const bad = '{"event":"submitted","body":not-json}';
      fs.writeFileSync(log, `{"event":"ready"}\n${bad}\n{"event":"partial`);
      let failure: unknown;
      try {
        readCompleteJsonLines(log);
      } catch (error) {
        failure = error;
      }
      expect(failure).toBeInstanceOf(SyntaxError);
      expect((failure as SyntaxError).message).toContain(bad);
      expect((failure as SyntaxError).message).toContain(log);
      expect((failure as SyntaxError).cause).toBeInstanceOf(SyntaxError);
      expect(() => readMockEvents(log)).toThrow(bad);
    });
  });

  it('defers an invalid unterminated tail, but fails once that same line is terminated', () => {
    withLog((log) => {
      fs.writeFileSync(log, 'not-json');
      expect(readCompleteJsonLines(log)).toEqual([]);
      fs.appendFileSync(log, '\n');
      expect(() => readCompleteJsonLines(log)).toThrow('not-json');
    });
  });

  it('preserves missing/empty-log behavior and accepts CRLF events without changing the file', () => {
    withLog((log) => {
      expect(readCompleteJsonLines(log)).toEqual([]);
      expect(fs.existsSync(log)).toBe(false);
      fs.writeFileSync(log, '');
      expect(readCompleteJsonLines(log)).toEqual([]);
      const bytes = '\n{"event":"ready"}\r\n\n';
      fs.writeFileSync(log, bytes);
      expect(readCompleteJsonLines(log)).toEqual([{ event: 'ready' }]);
      expect(fs.readFileSync(log, 'utf8')).toBe(bytes);
    });
  });
});
