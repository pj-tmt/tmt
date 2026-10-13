import { Bot, UserRound } from 'lucide-react';

/** Cosmetic kind only. Unknown kinds use agent; this grants no routing. */
export function MentionLabel({
  token,
  kind = 'agent',
}: {
  token: string;
  kind?: 'agent' | 'person';
}) {
  const Icon = kind === 'person' ? UserRound : Bot;
  return (
    <>
      <Icon className="message-mention-icon" aria-hidden />
      <span className="message-mention-name">{token}</span>
    </>
  );
}
export function MentionChip({
  token,
  kind = 'agent',
}: {
  token: string;
  kind?: 'agent' | 'person';
}) {
  return (
    <span className="message-mention" data-kind={kind}>
      <MentionLabel token={token} kind={kind} />
    </span>
  );
}
