import { requireValue } from '@tmt/colab-client';
import type { AskAgainInput } from './ask-again.js';
import { formatAskMessage } from './ask-intent.js';
import type { AskBinding, PageAsk } from './ask-panel.js';
import { ASK_MESSAGE_BYTES } from './ask-records.js';
import { sendCommentAsks, type MessageSendFailure } from './comment-send.js';
import type { AgentDestination } from './live-ask.js';
import { answerOf, type Question, type QuestionGroup } from './questions.js';
import type { QuoteSelector, ThreadView } from './thread-records.js';
import {
  captureConversation,
  commentForAsk,
  type AnswerBatchContext,
  type CommentContext,
  type ThreadBinding,
} from './thread-store.js';

/** Room kept under the Ask size cap for what the signed message adds around the composed text:
 * the delivery header with a device name, and a page link that is longer than the one captured. */
const FRAME_MARGIN_BYTES = 512;
const bytes = (value: string) => new TextEncoder().encode(value).length;

/** The sorted IDs of the comments an Ask input is about: the batch's answers, or the one comment. */
export function askedMessageIds(input: Pick<AskAgainInput, 'context' | 'batch'>): string[] {
  return (
    input.batch?.answers.map((answer) => answer.context.message.id) ?? [input.context.message.id]
  ).sort();
}

/** The Ask text of a batch: one line per answered question, in tab order. The frame around it (page,
 * link, quote) is the one every Ask has. */
export function batchAnswersText(answers: readonly { title: string; answer: string }[]) {
  return [
    `Answers to ${answers.length} questions:`,
    ...answers.map(({ title, answer }) => `${title}: ${answer}`),
  ].join('\n');
}

/** What the admitted page says about its questions: the source's own groups, never a card's claim. */
export interface AdmittedQuestions {
  questions: readonly Question[];
  groups: readonly QuestionGroup[];
}

/** The origin of a batch Ask, rechecked in the parent's verified projection before the Ask captures
 * it (colab-v1, answer batches). Every answer is this device's own, undeleted answer comment to a
 * question of the one group, the answers follow the group's tab order, and each comment is in an
 * anchored question thread of the discussion with its recorded revisions. Anything else throws and
 * nothing is asked. */
export function commentsForAnswerBatch(
  threads: ThreadView[],
  batch: AnswerBatchContext,
  page: AdmittedQuestions,
  deviceId: string,
) {
  const groups = page.groups.filter((group) => group.id === batch.group);
  requireValue(groups.length === 1);
  const order = groups[0].questions;
  requireValue(batch.answers.length >= 2 && batch.answers.length <= order.length);
  let previous = -1;
  const answers = batch.answers.map(({ question: id, context }) => {
    const index = order.indexOf(id);
    requireValue(index > previous);
    previous = index;
    const question = page.questions.filter((value) => value.id === id);
    requireValue(question.length === 1);
    const origin = commentForAsk(threads, context);
    const thread = threads.find((value) => value.threadId === origin.thread)!;
    const comment = thread.comments.find((value) => value.messageId === origin.messageIds[0])!;
    const answer = answerOf(question[0], comment.body);
    requireValue(
      comment.deviceAuthor === true &&
        comment.ref.writer === deviceId &&
        answer !== undefined &&
        !thread.proposal &&
        thread.anchor !== null,
    );
    return { origin, title: question[0].title, answer };
  });
  const messageIds = answers.map((value) => value.origin.messageIds[0]).sort();
  requireValue(messageIds.every((id, index) => index === 0 || messageIds[index - 1] < id));
  return {
    thread: answers[0].origin.thread,
    messageIds,
    quote: answers[0].origin.quote,
    comment: batchAnswersText(answers),
  };
}

/** One answered question of a Send, in tab order. `thread` is the question's canonical thread when
 * it has one (the answer is a reply there), otherwise `selector` anchors a new thread. */
export interface BatchAnswer {
  question: Question;
  body: string;
  selector: QuoteSelector;
  thread?: ThreadView;
}
export interface AnswerBatchInput {
  group: QuestionGroup;
  /** Only the answered questions, in the group's tab order. */
  answers: readonly BatchAnswer[];
  /** The visible recipients, frozen by the caller; none means the answers stay on the page. */
  recipients: readonly AgentDestination[];
  title: string;
  url: string;
}
/** One answer's comment. `failed` is "not confirmed recorded": an error before the publication left
 * this device, or a lost acknowledgement after it, which this call cannot tell apart. Either way no
 * Ask was attached to it, so no agent was contacted for it; if it was in fact appended, it shows up
 * as a verified answer on that question and the next Send's comment supersedes it (`currentAnswer`
 * is the newest own answer). The draft stays for that next Send. */
export interface AnswerOutcome {
  question: string;
  state: 'recorded' | 'failed';
  context?: CommentContext;
}
export type AnswerBatchResult =
  | { refused: 'too-large' }
  | {
      /** In the order given. */
      answers: AnswerOutcome[];
      /** The first recorded answer in tab order, whose thread the agent's one reply belongs to; absent
       * when nothing was recorded. After a reload the same thread is the Ask's own `thread`. */
      canonical?: { question: string; context: CommentContext };
      /** One per recipient whose Ask failed or is uncertain; its input retries the same Ask. */
      failures: MessageSendFailure[];
    };

/** Whether the Ask of these answers stays under the Ask size cap, before anything is published. */
export function batchFits(input: Pick<AnswerBatchInput, 'answers' | 'title' | 'url'>) {
  const lines = input.answers.map(({ question, body }) => {
    const answer = answerOf(question, body);
    requireValue(answer !== undefined);
    return { title: question.title, answer };
  });
  const quote = input.answers[0].thread?.anchor?.exact ?? input.answers[0].selector.exact;
  const message = formatAskMessage({
    title: input.title,
    url: input.url,
    quote,
    comment: batchAnswersText(lines),
  });
  return bytes(message) + FRAME_MARGIN_BYTES <= ASK_MESSAGE_BYTES;
}

/** One explicit trusted Send of an answered group (#2875): one answer comment per answered question
 * (each keeps its own thread, quote and history), then one Ask per recipient over exactly the
 * comments that are recorded. A question whose comment failed is not in that Ask; the caller sends
 * it again later. One recorded answer is the same single-comment Ask as before batches. */
export async function sendAnswers(
  bindings: { discussion: ThreadBinding; ask?: AskBinding },
  input: AnswerBatchInput,
  asks: readonly PageAsk[],
): Promise<AnswerBatchResult> {
  const { answers, recipients } = input;
  requireValue(answers.length > 0);
  if (answers.length > 1 && !batchFits(input)) return { refused: 'too-large' };
  // The earlier conversation of a thread is captured before this Send adds its comment.
  const earlier = answers.map(({ thread }) => captureConversation(thread, asks));
  const states: AnswerOutcome[] = [];
  const recorded: { answer: BatchAnswer; context: CommentContext; conversation: number }[] = [];
  for (const [index, answer] of answers.entries()) {
    try {
      const context = answer.thread
        ? await bindings.discussion.reply(answer.thread.ref, answer.body, answer.thread.revision)
        : await bindings.discussion.create(answer.body, answer.selector);
      states.push({ question: answer.question.id, state: 'recorded', context });
      recorded.push({ answer, context, conversation: index });
    } catch {
      states.push({ question: answer.question.id, state: 'failed' });
    }
  }
  if (!recorded.length) return { answers: states, failures: [] };
  const [first] = recorded;
  const canonical = { question: first.answer.question.id, context: first.context };
  if (!recipients.length) return { answers: states, canonical, failures: [] };
  const common = { title: input.title, url: input.url };
  const failures =
    recorded.length === 1
      ? await sendCommentAsks(bindings.ask, first.context, recipients, {
          ...common,
          quote: first.answer.selector.exact,
          comment: first.answer.body,
          conversation: earlier[first.conversation],
        })
      : await sendCommentAsks(
          bindings.ask,
          first.context,
          recipients,
          {
            ...common,
            quote: first.answer.selector.exact,
            comment: first.answer.body,
          },
          {
            group: input.group.id,
            answers: recorded.map(({ answer, context }) => ({
              question: answer.question.id,
              context,
            })),
          },
        );
  return { answers: states, canonical, failures };
}
