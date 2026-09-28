import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { PointerEvent } from 'react';
import { documentPoint, dragBox, hitElement, moveElement } from './geometry.js';
import { drawStrokeSegment, drawWhiteboard, drawWhiteboardSelection } from './drawing.js';
import { WHITEBOARD_LIMITS } from './scene-contract.js';
import type { Point, WhiteboardElement, WhiteboardScene } from './scene-contract.js';

export type WhiteboardTool =
  | 'select'
  | 'stroke'
  | 'arrow'
  | 'rectangle'
  | 'ellipse'
  | 'text'
  | 'note'
  | 'erase';
interface Gesture {
  pointerId: number;
  start: Point;
  scene: WhiteboardScene;
  tool: WhiteboardTool;
  color: string;
  pointBudget: number;
  element?: WhiteboardElement;
}
interface Props {
  scene: WhiteboardScene;
  tool: WhiteboardTool;
  color: string;
  disabled: boolean;
  selected?: string;
  select(id?: string): void;
  commit(scene: WhiteboardScene): void;
}

export function WhiteboardCanvas({
  scene,
  tool,
  color,
  disabled,
  selected,
  select,
  commit,
}: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const gesture = useRef<Gesture | undefined>(undefined);
  const [preview, setPreview] = useState<WhiteboardScene>();
  const [drawing, setDrawing] = useState(false);
  const [unavailable, setUnavailable] = useState(false);
  const [resolution, setResolution] = useState({ width: 1600, height: 1000 });
  useLayoutEffect(() => {
    const element = canvas.current!;
    const resize = () => {
      const width = Math.max(
        1,
        Math.round(
          element.getBoundingClientRect().width * Math.min(window.devicePixelRatio || 1, 2)
        )
      );
      const height = Math.max(
        1,
        Math.round((width * WHITEBOARD_LIMITS.height) / WHITEBOARD_LIMITS.width)
      );
      setResolution((previous) =>
        previous.width === width && previous.height === height ? previous : { width, height }
      );
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const visible = preview ?? scene;
  useEffect(() => {
    const context = canvas.current?.getContext('2d');
    if (!context) {
      setUnavailable(true);
      return;
    }
    const active = gesture.current;
    // A resize during a pen gesture redraws its buffer once at the new resolution.
    const painted =
      active?.tool === 'stroke' && active.element
        ? { ...visible, elements: [...visible.elements, active.element] }
        : visible;
    drawWhiteboard(context, painted);
    drawWhiteboardSelection(context, visible, selected ? [selected] : []);
  }, [visible, selected, drawing, resolution]);

  // Undo, reload or a save state change invalidates an in-flight gesture's base.
  useEffect(() => {
    gesture.current = undefined;
    setPreview(undefined);
    setDrawing(false);
  }, [scene, disabled]);

  function cancel() {
    gesture.current = undefined;
    setPreview(undefined);
    setDrawing(false);
  }
  function point(event: PointerEvent<HTMLCanvasElement>): Point {
    return documentPoint(event.clientX, event.clientY, event.currentTarget.getBoundingClientRect());
  }
  function start(event: PointerEvent<HTMLCanvasElement>) {
    if (disabled || unavailable || event.button !== 0 || gesture.current) return;
    const at = point(event);
    const hit = hitElement(scene.elements, at);
    let element = tool === 'select' || tool === 'erase' ? hit : undefined;
    if (tool === 'note' || tool === 'text') {
      element = {
        id: crypto.randomUUID(),
        kind: tool,
        x: Math.min(at[0], 1300),
        y: Math.min(at[1], 820),
        width: 300,
        height: 180,
        text: tool === 'note' ? 'New note' : 'Your text',
        color,
        fill: tool === 'note' ? '#ffe49a' : 'none',
        fontSize: 24,
      };
    }
    select(element?.id);
    if ((tool === 'select' || tool === 'erase') && !element) return;
    const pointBudget =
      WHITEBOARD_LIMITS.points -
      scene.elements.reduce(
        (total, item) => total + ('points' in item ? item.points.length : 0),
        0
      );
    gesture.current = {
      pointerId: event.pointerId,
      start: at,
      scene,
      element,
      tool,
      color,
      pointBudget,
    };
    setDrawing(true);
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }
  function update(
    event: PointerEvent<HTMLCanvasElement>,
    finished = false
  ): WhiteboardScene | undefined {
    const active = gesture.current;
    if (!active || active.pointerId !== event.pointerId || disabled) return;
    const at = point(event);
    const { tool, color } = active;
    if (tool === 'erase')
      return {
        ...active.scene,
        elements: active.scene.elements.filter((item) => item.id !== active.element?.id),
      };
    if (tool === 'select' && active.element)
      return {
        ...active.scene,
        elements: active.scene.elements.map((item) =>
          item.id === active.element!.id
            ? moveElement(active.element!, at[0] - active.start[0], at[1] - active.start[1])
            : item
        ),
      };
    if (tool === 'stroke') {
      if (active.pointBudget < 2) return;
      const previous = active.element;
      const path = previous && 'points' in previous ? previous : undefined;
      const last = path?.points.at(-1) ?? active.start;
      if (!path)
        active.element = {
          id: crypto.randomUUID(),
          kind: 'stroke',
          points: [active.start, at],
          color,
          strokeWidth: 4,
        };
      else if ((last[0] !== at[0] || last[1] !== at[1]) && path.points.length < active.pointBudget)
        path.points.push(at);
      // The mutable point buffer belongs only to this gesture, never to scene/history.
      const context = canvas.current?.getContext('2d');
      if (context && (!path || path.points.at(-1) === at))
        drawStrokeSegment(context, last, at, color, 4);
      if (!finished) return;
    } else if (tool === 'arrow') {
      const previous = active.element;
      if (active.pointBudget < 2) return;
      const points: Point[] = [active.start, at];
      active.element = {
        id: previous?.id ?? crypto.randomUUID(),
        kind: tool,
        points,
        color,
        strokeWidth: 4,
      };
    } else if (tool === 'rectangle' || tool === 'ellipse') {
      const box = dragBox(active.start, at);
      if (!box.width || !box.height) return;
      active.element = {
        id: active.element?.id ?? crypto.randomUUID(),
        kind: tool,
        ...box,
        color,
        fill: 'none',
        strokeWidth: 4,
      };
    }
    if (active.element)
      return { ...active.scene, elements: [...active.scene.elements, active.element] };
  }
  return (
    <div className="whiteboard-surface">
      {unavailable && (
        <p role="alert">
          Drawing is unavailable in this browser. The element list and text editor remain available.
        </p>
      )}
      <canvas
        ref={canvas}
        width={resolution.width}
        height={resolution.height}
        tabIndex={0}
        aria-label="Whiteboard drawing surface"
        aria-disabled={disabled}
        onPointerDown={start}
        onPointerMove={(event) => {
          const next = update(event);
          if (next) setPreview(next);
        }}
        onPointerUp={(event) => {
          if (gesture.current?.pointerId !== event.pointerId) return;
          const next = update(event, true);
          const id = gesture.current.tool === 'erase' ? undefined : gesture.current.element?.id;
          cancel();
          if (next) {
            commit(next);
            select(id);
          }
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={(event) => {
          if (gesture.current?.pointerId === event.pointerId) cancel();
        }}
        onLostPointerCapture={cancel}
        onKeyDown={(event) => {
          if (
            event.key === 'Escape' ||
            ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z')
          ) {
            if (event.key === 'Escape') event.stopPropagation();
            cancel();
          }
        }}
      />
      {scene.elements.length === 0 && !preview && !drawing && (
        <p className="whiteboard-empty">
          A little room for big ideas.<span>Pick a pen, add a note, or draw a connection.</span>
        </p>
      )}
    </div>
  );
}
