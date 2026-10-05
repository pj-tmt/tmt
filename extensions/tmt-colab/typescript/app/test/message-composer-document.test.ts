import { expect, it } from 'vite-plus/test';
import { createEditor } from 'lexical';
import {
  $messageText,
  $replaceMessage,
  fuzzyMessageCandidates,
  mentionQuery,
  MessageMentionNode,
} from '../src/components/message-composer-document.js';

it('round-trips exact plaintext paragraphs including empty and trailing lines and UTF16 characters', () => {
  const editor = createEditor({
    namespace: 'fixture',
    nodes: [MessageMentionNode],
    onError: (error) => {
      throw error;
    },
  });
  for (const value of ['', '\n', '\n\n', 'one\ntwo\n', 'é😀\n\nthree\n']) {
    editor.update(() => $replaceMessage(value), { discrete: true });
    expect(editor.getEditorState().read($messageText)).toBe(value);
  }
});

it('finds optional mention edits at the current UTF16 caret without routing ordinary text', () => {
  expect(mentionQuery('😀 before @bot after', 14)).toEqual({ start: 10, end: 14, query: 'bot' });
  expect(mentionQuery('text @b\nafter', 7)).toEqual({ start: 5, end: 7, query: 'b' });
  expect(mentionQuery('email@bot', 9)).toBeUndefined();
  expect(mentionQuery('Ask without any prefix', 22)).toBeUndefined();
});

it('fuzzy matching is stable, searches machine names, and never treats matches as selection', () => {
  const candidates = [
    { agentName: 'Alpha', machineName: 'Laptop' },
    { agentName: 'Beta', machineName: 'Alpha machine' },
    { agentName: 'Alphabet', machineName: 'Desktop' },
  ];
  expect(fuzzyMessageCandidates(candidates, 'alp')).toEqual([
    candidates[0],
    candidates[2],
    candidates[1],
  ]);
  expect(fuzzyMessageCandidates(candidates, 'laptop')).toEqual([candidates[0]]);
  expect(fuzzyMessageCandidates(candidates, 'zzz')).toEqual([]);
});
