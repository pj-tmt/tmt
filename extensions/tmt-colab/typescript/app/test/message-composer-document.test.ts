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

it('opens the list for @ typed straight after CJK text or punctuation, and for the full-width form', () => {
  // Chinese and Japanese are written without spaces: "請@bot" must offer the list.
  expect(mentionQuery('請@bot', 5)).toEqual({ start: 1, end: 5, query: 'bot' });
  expect(mentionQuery('こんにちは@', 6)).toEqual({ start: 5, end: 6, query: '' });
  expect(mentionQuery('請問（@b', 5)).toEqual({ start: 3, end: 5, query: 'b' });
  // An IME in full-width mode types ＠ (U+FF20); it is the same trigger and the same one UTF16 unit.
  expect(mentionQuery('＠bot', 4)).toEqual({ start: 0, end: 4, query: 'bot' });
  expect(mentionQuery('請 ＠', 3)).toEqual({ start: 2, end: 3, query: '' });
  // Address-like text still never routes.
  for (const text of ['email@bot', 'a.b+c@bot', 'user_1@bot', 'x-y@bot'])
    expect(mentionQuery(text, text.length)).toBeUndefined();
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
