/** Editor-owned metadata. Effective answered state comes from verified own records, never from
 * a content field supplied by a commenter. Mirrors question_registry.rs. */
import { coreIdentity, id, keys, nonempty, plain, requireValue } from './closed.js';
import { validateAttribution, type Attribution } from './summary-lines.js';
import { QUESTION_LIMIT, QUESTION_TITLE_POINTS } from './questions.js';
import { validQuestionId } from './question-id.js';
import { compareCommentOrder, type ThreadView } from './thread-records.js';
import { taskNumber } from './work.js';

export interface RegisteredAgent {
  id: string;
  name: string;
}
export interface PageCreator {
  agent: RegisteredAgent;
  machineId?: string;
  attribution: Attribution;
}
export interface RegisteredQuestion {
  id: string;
  agent?: RegisteredAgent;
  title?: string;
  group?: string;
  state: 'open' | 'handled';
  task?: string;
  attribution: Attribution;
}
export function validateRegisteredAgent(value: unknown): asserts value is RegisteredAgent {
  const agent = keys(value, ['id', 'name']);
  coreIdentity(agent.id);
  nonempty(agent.name, 128);
}
export function validatePageCreator(value: unknown): asserts value is PageCreator {
  const creator = keys(value, ['agent', 'attribution'], ['machineId']);
  validateRegisteredAgent(creator.agent);
  validateAttribution(creator.attribution);
  if (Object.hasOwn(creator, 'machineId')) id(creator.machineId);
}
export function validateQuestionRegistry(value: unknown): asserts value is RegisteredQuestion[] {
  requireValue(Array.isArray(value) && value.length <= QUESTION_LIMIT);
  const seen = new Set<string>();
  for (const entry of value) {
    const question = keys(
      entry,
      ['id', 'state', 'attribution'],
      ['agent', 'title', 'group', 'task'],
    );
    requireValue(typeof question.id === 'string' && validQuestionId(question.id));
    requireValue(!seen.has(question.id));
    seen.add(question.id);
    if (Object.hasOwn(question, 'agent')) validateRegisteredAgent(question.agent);
    validateAttribution(question.attribution);
    requireValue(question.state === 'open' || question.state === 'handled');
    if (Object.hasOwn(question, 'title')) {
      plain(question.title, QUESTION_TITLE_POINTS * 4);
      requireValue(
        question.title.length > 0 && [...question.title].length <= QUESTION_TITLE_POINTS,
      );
    }
    if (Object.hasOwn(question, 'group'))
      requireValue(typeof question.group === 'string' && validQuestionId(question.group));
    if (Object.hasOwn(question, 'task'))
      requireValue(
        question.state === 'handled' &&
          typeof question.task === 'string' &&
          taskNumber(question.task) !== undefined,
      );
  }
}

export type QuestionState = Omit<RegisteredQuestion, 'state'> & {
  state: 'open' | 'answered' | 'handled';
  answer?: string;
  answeredAt?: string;
  answerRef?: { writer: string; id: string; revision: string };
};
/** Consumes verified readThreads output only. Ask success and publisher labels grant nothing. */
export function questionStates(
  registry: readonly RegisteredQuestion[],
  threads: readonly ThreadView[],
): QuestionState[] {
  validateQuestionRegistry(registry);
  return registry.map((entry) => {
    const answer = threads
      .filter((thread) => !thread.deleted && !thread.proposal)
      .flatMap((thread) => thread.comments)
      .filter(
        (comment) =>
          !comment.deleted && comment.deviceAuthor === true && comment.questionId === entry.id,
      )
      .sort((a, b) =>
        compareCommentOrder(
          { sequence: Number(a.sequence ?? '0'), writer: a.ref.writer, id: a.ref.id },
          { sequence: Number(b.sequence ?? '0'), writer: b.ref.writer, id: b.ref.id },
        ),
      )
      .at(-1);
    if (!answer) return { ...entry };
    return {
      ...entry,
      state: entry.state === 'handled' ? 'handled' : 'answered',
      answer: [
        ...answer.body
          .replace(/\p{Cc}/gu, ' ')
          .split(/\p{White_Space}+/u)
          .filter(Boolean)
          .join(' '),
      ]
        .slice(0, 200)
        .join('')
        .replace(/\p{White_Space}+$/u, ''),
      answeredAt: answer.at,
      answerRef: { ...answer.ref, revision: answer.revision },
    };
  });
}
