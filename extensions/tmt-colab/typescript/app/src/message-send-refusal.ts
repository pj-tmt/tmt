import { OwnAppendRefused, OwnRecordConflict } from './own-publication.js';
import { SessionEndedError, SessionEvictedError } from './ask-remote.js';

/** Display-only references. Never derive these from an error message or a record identifier. */
export type MessageSendReference =
  | 'S1'
  | 'S2'
  | 'S3'
  | 'S4'
  | 'S5'
  | 'S6'
  | 'S7'
  | 'S8'
  | 'S9'
  | 'S10'
  | 'S11'
  | 'S12'
  | 'S13'
  | 'S14'
  | 'S15'
  | 'S16'
  | 'S17'
  | 'S18'
  | 'S19'
  | 'S20';
export interface MessageSendGate {
  sending: boolean;
  recorded: boolean;
  archived: boolean;
  blocked: boolean;
  discussion: boolean;
  allowed: boolean;
  needsBinding: boolean;
  binding: boolean;
  value: string;
}
/** The button and explicit submit share presentation admission; effect owners still revalidate. */
export function messageSendRefusal(state: MessageSendGate): MessageSendReference | undefined {
  if (state.sending) return 'S1';
  if (state.recorded) return 'S2';
  if (state.archived) return 'S3';
  if (state.blocked) return 'S4';
  if (!state.discussion) return 'S5';
  if (!state.allowed) return 'S6';
  if (state.needsBinding && !state.binding) return 'S7';
  if (!state.value.trim()) return 'S8';
}
export type MessageSendStage = 'admission' | 'capture' | 'attachments' | 'record' | 'dispatch';
export function messageSendFailureReference(
  stage: MessageSendStage,
  error: unknown,
): MessageSendReference {
  if (stage === 'admission') return 'S20';
  if (stage === 'capture') return 'S9';
  if (stage === 'attachments') return 'S11';
  if (stage === 'dispatch') return 'S13';
  if (error instanceof OwnRecordConflict) return 'S15';
  if (error instanceof OwnAppendRefused) return 'S16';
  if (error instanceof SessionEvictedError) return 'S18';
  if (error instanceof SessionEndedError)
    return error.code === 'REMOTE_SEQUENCE_UNAVAILABLE' ? 'S19' : 'S17';
  return 'S12';
}
