import {
  $createParagraphNode,
  $createRangeSelection,
  $createTextNode,
  $getRoot,
  $getSelection,
  $isElementNode,
  $isRangeSelection,
  $isTextNode,
  $setSelection,
  type LexicalNode,
} from 'lexical';

/** A token's span in the input text: UTF-16 offsets, one LF between paragraphs. */
export type RichInputRange = { start: number; end: number };
export type RichInputToken = { range: RichInputRange; create(text: string): LexicalNode };

/** Single LF between paragraphs, preserving blank and trailing lines. */
export function $richInputText() {
  return $getRoot()
    .getChildren()
    .map((node) => node.getTextContent())
    .join('\n');
}

/** Replaces the document; each token that fits within its line becomes the caller's node. */
export function $replaceRichInput(value: string, tokens: readonly RichInputToken[] = []) {
  const root = $getRoot();
  root.clear();
  let offset = 0;
  for (const line of value.split('\n')) {
    const paragraph = $createParagraphNode();
    let cursor = 0;
    for (const token of tokens) {
      const start = token.range.start - offset,
        end = token.range.end - offset;
      if (start < cursor || end <= start || end > line.length) continue;
      if (start > cursor) paragraph.append($createTextNode(line.slice(cursor, start)));
      paragraph.append(token.create(line.slice(start, end)));
      cursor = end;
    }
    if (cursor < line.length) paragraph.append($createTextNode(line.slice(cursor)));
    root.append(paragraph);
    offset += line.length + 1;
  }
  root.selectEnd();
}

/** The caller's intact tokens with their spans; node identity keeps them through surrounding edits. */
export function $richInputTokens<Node extends LexicalNode>(
  isToken: (node: LexicalNode) => node is Node,
): { node: Node; range: RichInputRange }[] {
  let offset = 0;
  const tokens: { node: Node; range: RichInputRange }[] = [];
  function visit(node: LexicalNode) {
    if (isToken(node) || $isTextNode(node)) {
      if (isToken(node))
        tokens.push({ node, range: { start: offset, end: offset + node.getTextContentSize() } });
      offset += node.getTextContentSize();
    } else if ($isElementNode(node)) node.getChildren().forEach(visit);
    else offset += node.getTextContentSize();
  }
  $getRoot()
    .getChildren()
    .forEach((node, index) => {
      if (index) offset++;
      visit(node);
    });
  return tokens;
}

export function $richInputCaret() {
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

export function $selectRichInputRange(start: number, end: number) {
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
