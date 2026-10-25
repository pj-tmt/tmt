import { readFileSync } from 'node:fs';
import { expect, test } from 'vite-plus/test';
import { validatePageCreator, validateQuestionRegistry } from '../src/question-registry.js';

const corpus = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/question-metadata-v1.json', import.meta.url),
    'utf8',
  ),
) as { cases: { name: string; kind: 'creator' | 'questions'; value: unknown; valid: boolean }[] };

for (const entry of corpus.cases)
  test(`question metadata: ${entry.name}`, () => {
    const validate = () =>
      entry.kind === 'creator'
        ? validatePageCreator(entry.value)
        : validateQuestionRegistry(entry.value);
    if (entry.valid) expect(validate).not.toThrow();
    else expect(validate).toThrow();
  });

import { questionStates, type RegisteredQuestion } from '../src/question-registry.js';
import { validateDiscussionRecord, type ThreadView } from '../src/thread-records.js';

const answers = JSON.parse(
  readFileSync(
    new URL('../../../contracts/vectors/question-answers-v1.json', import.meta.url),
    'utf8',
  ),
) as {
  cases: {
    name: string;
    registry: RegisteredQuestion[];
    threads: {
      writer: string;
      id: string;
      deleted: boolean;
      comments: Record<string, unknown>[];
    }[];
    deviceWriters: string[];
    expected: unknown;
    pending: string[];
  }[];
};
for (const entry of answers.cases)
  test(`verified question answers: ${entry.name}`, () => {
    const threads = entry.threads.map((thread) => ({
      deleted: thread.deleted,
      comments: thread.comments.map((comment) => ({
        ...comment,
        ref: { writer: comment.writer, id: comment.id },
        deviceAuthor: entry.deviceWriters.includes(comment.writer as string),
      })),
    })) as ThreadView[];
    const states = questionStates(entry.registry, threads);
    expect(states).toEqual(entry.expected);
    expect(
      states.filter((question) => question.state === 'open').map((question) => question.id),
    ).toEqual(entry.pending);
    expect(entry.registry.every((question) => question.state !== ('answered' as string))).toBe(
      true,
    );
  });

test('question bindings on ordinary signed comments use the existing question UUID grammar', () => {
  const vector = JSON.parse(
    readFileSync(new URL('../../../contracts/vectors/discussion-v1.json', import.meta.url), 'utf8'),
  );
  const comment = { ...vector.comment, questionId: '00000000-0000-0000-0000-000000000001' };
  const key = `${comment.messageId}:${comment.revision}`;
  expect(() => validateDiscussionRecord('messages', key, comment)).not.toThrow();
  for (const questionId of [null, '', 'not-a-question', 'ABCDEF00-0000-0000-0000-000000000001'])
    expect(() => validateDiscussionRecord('messages', key, { ...comment, questionId })).toThrow();
});
