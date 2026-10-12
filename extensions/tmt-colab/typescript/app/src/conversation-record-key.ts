/** Display identities distinguish comments/user turns from admitted agent replies. */
export function conversationRecordKey(
  kind: 'comment' | 'ask' | 'reply',
  writer: string,
  id: string,
) {
  return `${kind}:${writer}:${id}`;
}
