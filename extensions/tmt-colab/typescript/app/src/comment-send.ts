import type { AskAgainInput } from './ask-again.js';
import type { AskBinding } from './ask-panel.js';
import type { PreviewAttempt } from './ask-preview.js';
import type { AgentDestination } from './live-ask.js';
import type { AnswerBatchContext, CommentContext } from './thread-store.js';

export type MessageSendFailure = {
  agent: string;
  message: string;
  uncertain: boolean;
  recipient: { machine: string; agent: string };
  input: AskAgainInput;
};

/** An explicit trusted Send has already recorded this comment (or, with `batch`, these answer
 * comments, `origin` being the first) and frozen visible recipients. Each recipient gets one Ask.
 * Preparation failure is unadopted; send failure is uncertain and never replayed here. */
export async function sendCommentAsks(
  binding: AskBinding | undefined,
  origin: CommentContext,
  destinations: readonly AgentDestination[],
  input: Omit<AskAgainInput, 'context' | 'batch'> & {
    conversation?: CommentContext['conversation'];
  },
  batch?: AnswerBatchContext,
  settled?: (operationId: string, outcome: Awaited<ReturnType<PreviewAttempt['send']>>) => void,
): Promise<MessageSendFailure[]> {
  const failures: MessageSendFailure[] = [];
  for (const destination of destinations) {
    if (!binding) break;
    const { conversation, ...detail } = input;
    const askInput: AskAgainInput = {
      ...detail,
      context: { ...origin, conversation },
      ...(batch ? { batch } : {}),
    };
    const failure = {
      agent: destination.agentName,
      message: origin.message.id,
      recipient: { machine: destination.machine, agent: destination.agent },
      input: askInput,
    };
    let attempt: Awaited<ReturnType<AskBinding['prepare']>>;
    try {
      attempt = await binding.prepare({ ...askInput, destination });
    } catch {
      failures.push({ ...failure, uncertain: false });
      continue;
    }
    try {
      const result = await attempt.send();
      settled?.(attempt.preview.view.operationId, result);
      if (
        result.adopted === false ||
        (result.adopted === undefined && result.state === 'uncertain')
      )
        failures.push({ ...failure, uncertain: result.adopted !== false });
    } catch {
      failures.push({ ...failure, uncertain: true });
    }
  }
  return failures;
}
