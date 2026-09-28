import type { WhiteboardScene, WhiteboardElement } from './scene-contract.js';
import type { Point } from './scene-contract.js';
import { WHITEBOARD_LIMITS } from './scene-contract.js';
import { elementBounds } from './geometry.js';

export function drawWhiteboardSelection(
  context: CanvasRenderingContext2D,
  scene: WhiteboardScene,
  ids: readonly string[]
) {
  const selected = new Set(ids);
  context.save();
  setDocumentTransform(context);
  context.strokeStyle = '#008c91';
  context.lineWidth = 3;
  context.setLineDash([8, 5]);
  for (const element of scene.elements) {
    if (!selected.has(element.id)) continue;
    const box = elementBounds(element);
    context.strokeRect(box.x - 6, box.y - 6, box.width + 12, box.height + 12);
  }
  context.restore();
}

export function setDocumentTransform(context: CanvasRenderingContext2D) {
  context.setTransform(
    context.canvas.width / WHITEBOARD_LIMITS.width,
    0,
    0,
    context.canvas.height / WHITEBOARD_LIMITS.height,
    0,
    0
  );
}

/** Transient pen preview is incremental; a finished scene still uses the full painter. */
export function drawStrokeSegment(
  context: CanvasRenderingContext2D,
  from: Point,
  to: Point,
  color: string,
  width: number
) {
  context.save();
  setDocumentTransform(context);
  context.strokeStyle = color;
  context.lineWidth = width;
  context.lineCap = 'round';
  context.beginPath();
  context.moveTo(...from);
  context.lineTo(...to);
  context.stroke();
  context.restore();
}

/** Screen canvas and later snapshot export share this document-only painter. */
export function drawWhiteboard(context: CanvasRenderingContext2D, scene: WhiteboardScene) {
  context.save();
  setDocumentTransform(context);
  context.fillStyle = scene.background;
  context.fillRect(0, 0, scene.width, scene.height);
  for (const element of scene.elements) drawElement(context, element);
  context.restore();
}

function drawElement(context: CanvasRenderingContext2D, element: WhiteboardElement) {
  context.save();
  context.strokeStyle = element.color;
  context.lineJoin = 'round';
  context.lineCap = 'round';
  if ('points' in element) {
    context.lineWidth = element.strokeWidth;
    context.beginPath();
    context.moveTo(...element.points[0]!);
    for (const point of element.points.slice(1)) context.lineTo(...point);
    if (element.kind === 'arrow') {
      const [from, to] = element.points;
      const angle = Math.atan2(to![1] - from![1], to![0] - from![0]);
      const size = Math.max(12, element.strokeWidth * 4);
      context.moveTo(
        to![0] - Math.cos(angle - Math.PI / 6) * size,
        to![1] - Math.sin(angle - Math.PI / 6) * size
      );
      context.lineTo(...to!);
      context.lineTo(
        to![0] - Math.cos(angle + Math.PI / 6) * size,
        to![1] - Math.sin(angle + Math.PI / 6) * size
      );
    }
    context.stroke();
  } else {
    context.beginPath();
    if (element.kind === 'ellipse')
      context.ellipse(
        element.x + element.width / 2,
        element.y + element.height / 2,
        element.width / 2,
        element.height / 2,
        0,
        0,
        Math.PI * 2
      );
    else context.rect(element.x, element.y, element.width, element.height);
    if (element.fill !== 'none') {
      context.fillStyle = element.fill;
      context.fill();
    }
    if ('strokeWidth' in element) {
      context.lineWidth = element.strokeWidth;
      context.stroke();
    }
    if ('text' in element) {
      context.clip();
      context.fillStyle = element.color;
      context.font = `${element.fontSize}px ui-monospace, monospace`;
      context.textBaseline = 'top';
      const padding = element.kind === 'note' ? 16 : 4;
      const width = Math.max(1, element.width - padding * 2);
      let y = element.y + padding;
      for (const paragraph of element.text.split('\n')) {
        let line = '';
        for (const character of paragraph.replaceAll('\t', '    ')) {
          if (line && context.measureText(line + character).width > width) {
            context.fillText(line, element.x + padding, y);
            y += element.fontSize * 1.4;
            line = '';
          }
          line += character;
        }
        context.fillText(line, element.x + padding, y);
        y += element.fontSize * 1.4;
        if (y > element.y + element.height) break;
      }
    }
  }
  context.restore();
}
