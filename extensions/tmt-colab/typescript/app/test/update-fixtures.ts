/** Hostile update encoders only; production foreign decoding remains in Worker. */
import * as Y from 'yjs';
export function invalidUpdates(): Uint8Array[] {
  const unknown = new Y.Doc();
  unknown.getText('permission').insert(0, 'editor');
  const rich = new Y.Doc();
  rich.getText('html').insert(0, 'formatted', { bold: true });
  const nested = new Y.Doc();
  nested.getMap('meta').set('title', new Y.Map());
  const mixed = new Y.Doc();
  mixed.getMap('html').set('not-text', 'value');
  return [unknown, rich, nested, mixed].map((doc) => {
    const bytes = Y.encodeStateAsUpdate(doc);
    doc.destroy();
    return bytes;
  });
}
