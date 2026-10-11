import {
  $replaceRichInput,
  $richInputText as $messageText,
  $richInputTokens,
  $selectRichInputRange as $selectMessageRange,
  $richInputCaret as $messageCaret,
} from './rich-input-document.js';
import {
  $createParagraphNode,
  $getSelection,
  $getNodeByKey,
  $isElementNode,
  $isRangeSelection,
  $setSelection,
  $createTextNode,
  $getRoot,
  DecoratorNode,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical';
import { createElement, type ReactNode } from 'react';
import { MessageMentionChip } from './message-mention-chip.js';
import { resolveMessageMentions } from '../message-recipient.js';
import type { AgentDestination } from '../live-ask.js';
import type { ComposerEdit, RecipientKey } from './message-composer-edit.js';

type SerializedMention = Spread<
  { recipient: RecipientKey; token: string; defaultCreator?: true },
  SerializedLexicalNode
>;

/** Editing tokens only; the parent revalidates their UUID keys before every Send. */
export class MessageMentionNode extends DecoratorNode<ReactNode> {
  __recipient: RecipientKey;
  __token: string;
  __defaultCreator: boolean;
  static getType() {
    return 'message-mention';
  }
  static clone(node: MessageMentionNode) {
    return new MessageMentionNode(
      node.__token,
      node.__recipient,
      node.__key,
      node.__defaultCreator,
    );
  }
  constructor(token: string, recipient: RecipientKey, key?: NodeKey, defaultCreator = false) {
    super(key);
    this.__recipient = recipient;
    this.__token = token;
    this.__defaultCreator = defaultCreator;
  }
  static importJSON(serialized: SerializedMention) {
    return new MessageMentionNode(
      serialized.token,
      serialized.recipient,
      undefined,
      serialized.defaultCreator === true,
    ).updateFromJSON(serialized);
  }
  exportJSON(): SerializedMention {
    return {
      ...super.exportJSON(),
      type: 'message-mention',
      recipient: this.__recipient,
      token: this.__token,
      ...(this.__defaultCreator ? { defaultCreator: true as const } : {}),
    };
  }
  createDOM() {
    const node = document.createElement('span');
    node.className = 'message-mention-host';
    return node;
  }
  updateDOM() {
    return false;
  }
  exportDOM() {
    return { element: document.createTextNode(this.getTextContent()) };
  }
  isInline() {
    return true;
  }
  getTextContent() {
    return this.__token;
  }
  decorate() {
    return createElement(MessageMentionChip, {
      nodeKey: this.getKey(),
      token: this.__token,
      recipient: this.__recipient,
    });
  }
}

export { $richInputText as $messageText } from './rich-input-document.js';

export function $replaceMessage(value: string, mentions: ComposerEdit['mentions'] = []) {
  $replaceRichInput(
    value,
    mentions.map((mention) => ({
      range: mention.range,
      create: (token) =>
        new MessageMentionNode(token, mention.key, undefined, mention.defaultCreator === true),
    })),
  );
}

/** Node identity/history retains every intact token through surrounding edits. */
export function $messageMentions(): NonNullable<ComposerEdit['mentions']> {
  return $richInputTokens(
    (node): node is MessageMentionNode => node instanceof MessageMentionNode,
  ).map(({ node, range }) => ({
    key: node.__recipient,
    ...(node.__defaultCreator ? { defaultCreator: true as const } : {}),
    range,
  }));
}

/** A preference transition edits only its own atomic token, preserving all surrounding text. */
export function $setDefaultCreator(key: RecipientKey, label: string, enabled: boolean) {
  const root = $getRoot();
  const selection = $getSelection()?.clone();
  const nodes: MessageMentionNode[] = [];
  function visit(node: LexicalNode) {
    if (
      node instanceof MessageMentionNode &&
      node.__recipient.machine === key.machine &&
      node.__recipient.agent === key.agent
    )
      nodes.push(node);
    else if ($isElementNode(node)) node.getChildren().forEach(visit);
  }
  visit(root);
  if (enabled && !nodes.length) {
    const first = root.getFirstChild();
    const paragraph = $isElementNode(first) ? first : $createParagraphNode();
    if (!first) root.append(paragraph);
    const mention = new MessageMentionNode(`@${label}`, key, undefined, true);
    const space = $createTextNode(' ');
    const head = paragraph.getFirstChild();
    if (head) {
      head.insertBefore(mention);
      head.insertBefore(space);
    } else paragraph.append(mention, space);
  } else if (!enabled) {
    for (const node of nodes) if (node.__defaultCreator) node.remove();
  }
  if (
    $isRangeSelection(selection) &&
    $getNodeByKey(selection.anchor.key)?.isAttached() &&
    $getNodeByKey(selection.focus.key)?.isAttached()
  )
    $setSelection(selection);
}

/** Replace only newly completed tokens, keeping the caret, surrounding nodes and history. */
export function $bindMessageMentions(agents: readonly AgentDestination[], includeEnd = false) {
  const value = $messageText(),
    existing = $messageMentions(),
    caret = $messageCaret();
  const result = resolveMessageMentions({ value, mentions: existing }, agents, includeEnd);
  if (caret !== undefined) {
    for (const token of [...result.mentions].reverse()) {
      if (existing.some((old) => old.range.start === token.range.start)) continue;
      $selectMessageRange(token.range.start, token.range.end).insertNodes([
        new MessageMentionNode(value.slice(token.range.start, token.range.end), token.key),
      ]);
    }
    $selectMessageRange(caret, caret);
  }
  return { value, mentions: $messageMentions() };
}

/**
 * The @ query at the caret. The trigger is `@` or the full-width `＠` an IME may type, one UTF16
 * unit either way. It must start the text or follow whitespace or any character that is not
 * address-like ASCII: Chinese and Japanese are written without spaces, so "請@bot" asks for a
 * recipient while "email@bot" stays plain text.
 */
export function mentionQuery(value: string, caret: number) {
  const before = value.slice(0, caret);
  const match = /(?:^|[^A-Za-z0-9_.+\-@＠])[@＠]([^\s@＠]*)$/.exec(before);
  return match ? { start: caret - match[1].length - 1, end: caret, query: match[1] } : undefined;
}

/** Fuzzy subsequence, ranked by contiguous prefix then gaps; source order is a stable tie-breaker. */
export function fuzzyMessageCandidates<T extends { agentName: string; machineName: string }>(
  candidates: readonly T[],
  query: string,
): T[] {
  const needle = query.toLowerCase();
  return candidates
    .map((candidate, index) => {
      const haystack = `${candidate.agentName} ${candidate.machineName}`.toLowerCase();
      let cursor = 0;
      let gaps = 0;
      for (const letter of needle) {
        const found = haystack.indexOf(letter, cursor);
        if (found < 0) return { candidate, index, score: Infinity };
        gaps += found - cursor;
        cursor = found + 1;
      }
      return { candidate, index, score: haystack.startsWith(needle) ? 0 : gaps + 1 };
    })
    .filter(({ score }) => Number.isFinite(score))
    .sort((a, b) => a.score - b.score || a.index - b.index)
    .map(({ candidate }) => candidate);
}

export {
  $richInputCaret as $messageCaret,
  $selectRichInputRange as $selectMessageRange,
} from './rich-input-document.js';
