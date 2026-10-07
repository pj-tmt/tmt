import {
  $createParagraphNode,
  $createRangeSelection,
  $getSelection,
  $isElementNode,
  $isRangeSelection,
  $setSelection,
  $createTextNode,
  $getRoot,
  $isTextNode,
  TextNode,
  type LexicalNode,
  type NodeKey,
  type SerializedTextNode,
  type Spread,
} from 'lexical';
import type { ComposerEdit, RecipientKey } from './message-composer-edit.js';

type SerializedMention = Spread<{ recipient: RecipientKey; token: string }, SerializedTextNode>;

/** Editing metadata only: routing remains in the parent's independent recipient state. */
export class MessageMentionNode extends TextNode {
  __recipient: RecipientKey;
  __token: string;
  static getType() {
    return 'message-mention';
  }
  static clone(node: MessageMentionNode) {
    return new MessageMentionNode(node.__text, node.__recipient, node.__token, node.__key);
  }
  constructor(text: string, recipient: RecipientKey, token = text, key?: NodeKey) {
    super(text, key);
    this.__recipient = recipient;
    this.__token = token;
  }
  static importJSON(serialized: SerializedMention) {
    return new MessageMentionNode(
      serialized.text,
      serialized.recipient,
      serialized.token,
    ).updateFromJSON(serialized);
  }
  exportJSON(): SerializedMention {
    return {
      ...super.exportJSON(),
      type: 'message-mention',
      recipient: this.__recipient,
      token: this.__token,
    };
  }
  isTextEntity() {
    return true;
  }
}

/** Single LF between paragraphs, preserving blank and trailing lines. */
export function $messageText() {
  return $getRoot()
    .getChildren()
    .map((node) => node.getTextContent())
    .join('\n');
}

export function $replaceMessage(value: string, mention?: ComposerEdit['mention']) {
  const root = $getRoot();
  root.clear();
  let offset = 0;
  for (const line of value.split('\n')) {
    const paragraph = $createParagraphNode();
    const start = mention ? mention.range.start - offset : -1;
    const end = mention ? mention.range.end - offset : -1;
    if (mention && start >= 0 && end > start && end <= line.length) {
      if (start) paragraph.append($createTextNode(line.slice(0, start)));
      paragraph.append(new MessageMentionNode(line.slice(start, end), mention.key));
      if (end < line.length) paragraph.append($createTextNode(line.slice(end)));
    } else if (line) paragraph.append($createTextNode(line));
    root.append(paragraph);
    offset += line.length + 1;
  }
}

/** Node identity/history retains the token through surrounding edits; edited tokens are invalidated. */
export function $messageMention(): ComposerEdit['mention'] {
  let offset = 0;
  let mention: ComposerEdit['mention'];
  function visit(node: LexicalNode) {
    if ($isTextNode(node)) {
      if (node instanceof MessageMentionNode && node.getTextContent() === node.__token)
        mention = {
          key: node.__recipient,
          range: { start: offset, end: offset + node.getTextContentSize() },
        };
      offset += node.getTextContentSize();
    } else if ($isElementNode(node)) {
      for (const child of node.getChildren()) visit(child);
    } else offset += node.getTextContentSize();
  }
  const paragraphs = $getRoot().getChildren();
  paragraphs.forEach((node, index) => {
    if (index) offset++;
    visit(node);
  });
  return mention;
}

export function mentionQuery(value: string, caret: number) {
  const before = value.slice(0, caret);
  const match = /(?:^|\s)@([^\s@]*)$/.exec(before);
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

export function $messageCaret() {
  const selection = $getSelection();
  if (!$isRangeSelection(selection) || !selection.isCollapsed()) return undefined;
  const anchor = selection.anchor;
  let offset = 0;
  let caret: number | undefined;
  function visit(node: LexicalNode) {
    if (node.getKey() === anchor.key) {
      caret =
        offset +
        (anchor.type === 'text'
          ? anchor.offset
          : $isElementNode(node)
            ? node
                .getChildren()
                .slice(0, anchor.offset)
                .reduce((sum, child) => sum + child.getTextContentSize(), 0)
            : 0);
    }
    if ($isElementNode(node)) node.getChildren().forEach(visit);
    else offset += node.getTextContentSize();
  }
  $getRoot()
    .getChildren()
    .forEach((node, index) => {
      if (index) offset++;
      visit(node);
    });
  return caret;
}

export function $selectMessageRange(start: number, end: number) {
  const selection = $createRangeSelection();
  function setPoint(point: typeof selection.anchor, target: number) {
    let offset = 0;
    const paragraphs = $getRoot().getChildren();
    for (const paragraph of paragraphs) {
      const size = paragraph.getTextContentSize();
      if (target <= offset + size && $isElementNode(paragraph)) {
        function within(element: LexicalNode, local: number) {
          if (!$isElementNode(element)) return;
          const children = element.getChildren();
          for (let index = 0; index < children.length; index++) {
            const child = children[index];
            const length = child.getTextContentSize();
            if (local <= length) {
              if ($isTextNode(child)) point.set(child.getKey(), local, 'text');
              else if ($isElementNode(child)) within(child, local);
              else point.set(element.getKey(), index + (local > 0 ? 1 : 0), 'element');
              return;
            }
            local -= length;
          }
          point.set(element.getKey(), children.length, 'element');
        }
        within(paragraph, target - offset);
        return;
      }
      offset += size + 1;
    }
    const last = paragraphs.at(-1);
    if ($isElementNode(last)) point.set(last.getKey(), last.getChildrenSize(), 'element');
  }
  setPoint(selection.anchor, start);
  setPoint(selection.focus, end);
  $setSelection(selection);
  return selection;
}

export function $clearMessageMentions() {
  function visit(node: LexicalNode) {
    if (node instanceof MessageMentionNode) node.replace($createTextNode(node.getTextContent()));
    else if ($isElementNode(node)) node.getChildren().forEach(visit);
  }
  $getRoot().getChildren().forEach(visit);
}
