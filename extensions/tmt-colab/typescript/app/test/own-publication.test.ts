import { expect, it } from 'vite-plus/test';
import { OwnPublicationTimeout, publicationDiagnostic } from '../src/own-publication.js';

const diagnostic = {
  root: 'messages',
  kind: 'ask-state',
  streamId: '30000000-0000-4000-8000-000000000001',
  submittedSeq: '7',
  admittedSeq: '4',
  leaderReceipt: true,
};

it('copies only the closed publication diagnostic, including an unknown replay sequence', () => {
  expect(publicationDiagnostic(diagnostic)).toEqual(diagnostic);
  expect(publicationDiagnostic(diagnostic)).not.toBe(diagnostic);
  expect(publicationDiagnostic({ ...diagnostic, submittedSeq: null })).toEqual({
    ...diagnostic,
    submittedSeq: null,
  });
});

it.each([
  ['threads', 'thread'],
  ['intents', 'ask'],
  ['intents', 'attachment-publication'],
  ['messages', 'comment'],
  ['messages', 'ask-state'],
  ['messages', 'thread-status'],
  ['messages', 'thread-notification'],
  ['messages', 'proposal-decision'],
  ['replies', 'ask-reply'],
])('keeps the admitted record kind %s/%s', (root, kind) => {
  expect(publicationDiagnostic({ ...diagnostic, root, kind })).toEqual({
    ...diagnostic,
    root,
    kind,
  });
});

it.each([
  { key: 'PRIVATE' },
  { value: 'PRIVATE' },
  { url: 'https://PRIVATE' },
  { root: 'PRIVATE' },
  { kind: 'PRIVATE' },
  { streamId: 'PRIVATE' },
  { submittedSeq: '01' },
  { submittedSeq: 7 },
  { submittedSeq: '18446744073709551616' },
  { admittedSeq: null },
  { leaderReceipt: 'true' },
  { root: 'threads' },
])('rejects unknown fields or malformed diagnostics: %j', (change) => {
  expect(publicationDiagnostic({ ...diagnostic, ...change })).toBeUndefined();
});

it('never echoes an unknown kind, record key or record value', () => {
  const error = new OwnPublicationTimeout(
    { root: 'messages', key: 'PRIVATE', value: { kind: 'PRIVATE', body: 'PRIVATE' } },
    diagnostic.streamId,
    '4',
    { seq: null },
  );
  expect(error.message).toBe('Own publication catchup timed out');
  expect(error.diagnostic).toEqual({ ...diagnostic, kind: 'unknown', submittedSeq: null });
  expect(JSON.stringify(error)).not.toContain('PRIVATE');
});
