import { $createParagraphNode, $createTextNode, $getRoot } from 'lexical';

/** Single LF between paragraphs, preserving blank and trailing lines. */
export function $richInputText() {
  return $getRoot()
    .getChildren()
    .map((node) => node.getTextContent())
    .join('\n');
}
export function $replaceRichInput(value: string) {
  const root = $getRoot();
  root.clear();
  for (const line of value.split('\n'))
    root.append($createParagraphNode().append($createTextNode(line)));
  root.selectEnd();
}
