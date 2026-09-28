import { exactRecord, uuidIdentifier } from '../contracts/record.js';

export const WHITEBOARD_LIMITS = {
  width: 1600,
  height: 1000,
  documentBytes: 2 * 1024 * 1024,
  elements: 2048,
  points: 65536,
  textBytes: 16 * 1024,
  totalTextBytes: 64 * 1024,
} as const;

export type Point = readonly [number, number];
export interface PathElement {
  id: string;
  kind: 'stroke' | 'arrow';
  points: Point[];
  color: string;
  strokeWidth: number;
}
interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface ShapeElement extends Box {
  id: string;
  kind: 'rectangle' | 'ellipse';
  color: string;
  fill: string;
  strokeWidth: number;
}
export interface TextElement extends Box {
  id: string;
  kind: 'text' | 'note';
  text: string;
  color: string;
  fill: string;
  fontSize: number;
}
export type WhiteboardElement = PathElement | ShapeElement | TextElement;
export interface WhiteboardScene {
  formatVersion: 1;
  width: 1600;
  height: 1000;
  background: string;
  elements: WhiteboardElement[];
}

function integer(value: unknown, minimum: number, maximum: number): number {
  if (!Number.isInteger(value) || (value as number) < minimum || (value as number) > maximum)
    throw new Error('Whiteboard coordinate or size is out of bounds.');
  return value as number;
}
function color(value: unknown, allowNone = false): string {
  if (
    typeof value !== 'string' ||
    (!/^#[0-9a-f]{6}$/.test(value) && !(allowNone && value === 'none'))
  )
    throw new Error('Invalid whiteboard color.');
  return value;
}
function box(value: Record<string, unknown>): Box {
  const x = integer(value.x, 0, WHITEBOARD_LIMITS.width);
  const y = integer(value.y, 0, WHITEBOARD_LIMITS.height);
  return {
    x,
    y,
    width: integer(value.width, 1, WHITEBOARD_LIMITS.width - x),
    height: integer(value.height, 1, WHITEBOARD_LIMITS.height - y),
  };
}
export function whiteboardText(value: unknown): string {
  if (
    typeof value !== 'string' ||
    new TextEncoder().encode(value).length > WHITEBOARD_LIMITS.textBytes ||
    Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return (
        (code <= 0x1f && code !== 0x09 && code !== 0x0a) ||
        (code >= 0x7f && code <= 0x9f) ||
        (code >= 0xd800 && code <= 0xdfff)
      );
    })
  )
    throw new Error('Invalid whiteboard text.');
  return value;
}

function element(value: unknown): WhiteboardElement {
  if (!value || typeof value !== 'object') throw new Error('Invalid whiteboard element.');
  const kind = (value as Record<string, unknown>).kind;
  if (kind === 'stroke' || kind === 'arrow') {
    const item = exactRecord(value, ['id', 'kind', 'points', 'color', 'strokeWidth'], 'path');
    if (
      !Array.isArray(item.points) ||
      item.points.length < 2 ||
      item.points.length > WHITEBOARD_LIMITS.points ||
      (kind === 'arrow' && item.points.length !== 2)
    )
      throw new Error('Invalid whiteboard path.');
    return {
      id: uuidIdentifier(item.id),
      kind,
      points: Array.from(item.points, (point: unknown): Point => {
        if (!Array.isArray(point) || point.length !== 2)
          throw new Error('Invalid whiteboard point.');
        return [
          integer(point[0], 0, WHITEBOARD_LIMITS.width),
          integer(point[1], 0, WHITEBOARD_LIMITS.height),
        ];
      }),
      color: color(item.color),
      strokeWidth: integer(item.strokeWidth, 1, 16),
    };
  }
  if (kind === 'rectangle' || kind === 'ellipse') {
    const item = exactRecord(
      value,
      ['id', 'kind', 'x', 'y', 'width', 'height', 'color', 'fill', 'strokeWidth'],
      'shape'
    );
    return {
      id: uuidIdentifier(item.id),
      kind,
      ...box(item),
      color: color(item.color),
      fill: color(item.fill, true),
      strokeWidth: integer(item.strokeWidth, 1, 16),
    };
  }
  if (kind === 'text' || kind === 'note') {
    const item = exactRecord(
      value,
      ['id', 'kind', 'x', 'y', 'width', 'height', 'text', 'color', 'fill', 'fontSize'],
      'text element'
    );
    return {
      id: uuidIdentifier(item.id),
      kind,
      ...box(item),
      text: whiteboardText(item.text),
      color: color(item.color),
      fill: color(item.fill, true),
      fontSize: integer(item.fontSize, 12, 72),
    };
  }
  throw new Error('Unsupported whiteboard element kind.');
}

/** Own the admitted value: callers cannot mutate it through the input object. */
export function decodeWhiteboardScene(value: unknown): WhiteboardScene {
  const scene = exactRecord(
    value,
    ['formatVersion', 'width', 'height', 'background', 'elements'],
    'whiteboard'
  );
  if (
    scene.formatVersion !== 1 ||
    scene.width !== WHITEBOARD_LIMITS.width ||
    scene.height !== WHITEBOARD_LIMITS.height ||
    !Array.isArray(scene.elements) ||
    scene.elements.length > WHITEBOARD_LIMITS.elements
  )
    throw new Error('Unsupported whiteboard scene.');
  const elements = Array.from(scene.elements, element);
  let points = 0;
  let textBytes = 0;
  const ids = new Set<string>();
  for (const item of elements) {
    if (ids.has(item.id)) throw new Error('Duplicate whiteboard element ID.');
    ids.add(item.id);
    if ('points' in item) points += item.points.length;
    if ('text' in item) textBytes += new TextEncoder().encode(item.text).length;
  }
  if (points > WHITEBOARD_LIMITS.points || textBytes > WHITEBOARD_LIMITS.totalTextBytes)
    throw new Error('Whiteboard document budget exceeded.');
  const result: WhiteboardScene = {
    formatVersion: 1,
    width: WHITEBOARD_LIMITS.width,
    height: WHITEBOARD_LIMITS.height,
    background: color(scene.background),
    elements,
  };
  if (new TextEncoder().encode(JSON.stringify(result)).length > WHITEBOARD_LIMITS.documentBytes)
    throw new Error('Whiteboard document is too large.');
  return result;
}

export function parseWhiteboardScene(source: string): WhiteboardScene {
  if (new TextEncoder().encode(source).length > WHITEBOARD_LIMITS.documentBytes)
    throw new Error('Whiteboard document is too large.');
  return decodeWhiteboardScene(JSON.parse(source));
}
