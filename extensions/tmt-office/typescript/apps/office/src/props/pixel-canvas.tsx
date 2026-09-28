import { useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent } from 'react';
import { IndexedRaster } from '../rendering/indexed-raster.js';
import { paintPixels, pixelSegment } from './pixel-draft.js';
import type { PixelCell, PixelDraft } from './pixel-draft.js';

/** A stroke is one history entry. Pointer capture owns its preview until commit or cancel. */
export function PixelCanvas({
  draft,
  color,
  disabled,
  commit,
  strokeChanged,
}: {
  draft: PixelDraft;
  color: number;
  disabled: boolean;
  commit: (draft: PixelDraft) => void;
  strokeChanged: (active: boolean) => void;
}) {
  const [cursor, setCursor] = useState<PixelCell>({ x: 0, y: 0 });
  const [preview, setPreview] = useState<PixelDraft>();
  const stroke = useRef<
    { pointerId: number; cell: PixelCell; draft: PixelDraft; color: number } | undefined
  >(undefined);
  const cell = (event: PointerEvent<SVGSVGElement>): PixelCell => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const clamp = (value: number) => Math.max(0, Math.min(draft.size - 1, Math.floor(value)));
    return {
      x: clamp(((event.clientX - bounds.left) / bounds.width) * draft.size),
      y: clamp(((event.clientY - bounds.top) / bounds.height) * draft.size),
    };
  };
  function finish(accept: boolean) {
    const pending = stroke.current;
    if (!pending) return;
    stroke.current = undefined;
    setPreview(undefined);
    strokeChanged(false);
    if (accept) commit(pending.draft);
  }
  function move(event: PointerEvent<SVGSVGElement>) {
    const pending = stroke.current;
    if (!pending || event.pointerId !== pending.pointerId) return;
    const next = cell(event);
    pending.draft = paintPixels(pending.draft, pixelSegment(pending.cell, next), pending.color);
    pending.cell = next;
    setCursor(next);
    setPreview(pending.draft);
  }
  function keyboard(event: KeyboardEvent<SVGSVGElement>) {
    if (disabled || stroke.current) return;
    const step = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[
      event.key
    ];
    if (step) {
      event.preventDefault();
      setCursor({
        x: Math.max(0, Math.min(draft.size - 1, cursor.x + step[0]!)),
        y: Math.max(0, Math.min(draft.size - 1, cursor.y + step[1]!)),
      });
    } else if ([' ', 'Enter', 'Backspace', 'Delete'].includes(event.key)) {
      event.preventDefault();
      commit(paintPixels(draft, [cursor], ['Backspace', 'Delete'].includes(event.key) ? 0 : color));
    }
  }
  const image = preview ?? draft;
  const grid = Array.from(
    { length: draft.size + 1 },
    (_, i) => `M${i} 0v${draft.size}M0 ${i}h${draft.size}`
  ).join('');
  return (
    <div className="pixel-canvas-wrap">
      <svg
        className="pixel-canvas"
        viewBox={`0 0 ${draft.size} ${draft.size}`}
        role="group"
        aria-label="Pixel drawing canvas"
        aria-describedby="pixel-canvas-instructions"
        aria-disabled={disabled}
        tabIndex={disabled ? -1 : 0}
        onKeyDown={keyboard}
        onPointerDown={(event) => {
          if (disabled || stroke.current || event.button !== 0) return;
          event.preventDefault();
          // Focus must not scroll the HUD under an active, screen-space stroke.
          event.currentTarget.focus({ preventScroll: true });
          event.currentTarget.setPointerCapture(event.pointerId);
          const position = cell(event);
          const painted = paintPixels(draft, [position], color);
          stroke.current = { pointerId: event.pointerId, cell: position, draft: painted, color };
          strokeChanged(true);
          setCursor(position);
          setPreview(painted);
        }}
        onPointerMove={move}
        onPointerUp={(event) => {
          if (stroke.current?.pointerId !== event.pointerId) return;
          move(event);
          finish(true);
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={() => finish(false)}
        onLostPointerCapture={() => finish(false)}
        onBlur={() => finish(false)}
      >
        <IndexedRaster {...image} width={draft.size} height={draft.size} indexWidth={2} />
        <path d={grid} fill="none" stroke="#263f4640" strokeWidth=".035" />
        <rect className="pixel-cursor" x={cursor.x} y={cursor.y} width="1" height="1" />
      </svg>
      <p id="pixel-canvas-instructions">
        Draw with a pointer. Or focus the canvas: arrow keys move, Space paints, Delete erases.
      </p>
      <output aria-live="polite">
        Pixel {cursor.x + 1}, {cursor.y + 1}
      </output>
    </div>
  );
}
