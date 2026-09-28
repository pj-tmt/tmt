import { useEffect, useRef, useState } from 'react';
import type { PointerEvent as ReactPointerEvent } from 'react';
import { catalogFurniture } from '../blocks/block-contract.js';
import type { CatalogPack } from '../props/prop-contract.js';
import type { WorldObject } from './world-contract.js';
import type { CatalogPlacement } from '../rendering/catalog-placement.js';

export interface CatalogDragHandlers {
  start(event: ReactPointerEvent<HTMLButtonElement>, pack: CatalogPack, key: string): void;
  consumeClick(detail?: number): boolean;
}

/** A catalog gesture owns only transient input. The scene owns projection and
 * validation feedback; the existing world editor owns the single final change. */
export function useCatalogDrag({
  disabled,
  commit,
}: {
  disabled: boolean;
  commit: (object: WorldObject, pack: CatalogPack) => void;
}) {
  const placement = useRef<CatalogPlacement | undefined>(undefined);
  const latest = useRef({ disabled, commit });
  latest.current = { disabled, commit };
  const gesture = useRef<
    | {
        pointerId: number;
        startX: number;
        startY: number;
        button: HTMLButtonElement;
        pack: CatalogPack;
        object: WorldObject;
        dragging: boolean;
      }
    | undefined
  >(undefined);
  const suppressClick = useRef(false);
  const [feedback, setFeedback] = useState<{
    x: number;
    y: number;
    message: string;
    valid: boolean;
  }>();
  const [error, setError] = useState<string>();

  function clear() {
    const current = gesture.current;
    gesture.current = undefined;
    if (current?.button.hasPointerCapture(current.pointerId))
      current.button.releasePointerCapture(current.pointerId);
    placement.current?.clear();
    setFeedback(undefined);
  }
  const cancel = useRef(clear);
  cancel.current = clear;
  useEffect(() => {
    if (disabled) cancel.current();
  }, [disabled]);
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const current = gesture.current;
      if (!current || current.pointerId !== event.pointerId) return;
      if (
        !current.dragging &&
        Math.hypot(event.clientX - current.startX, event.clientY - current.startY) <= 5
      )
        return;
      current.dragging = true;
      suppressClick.current = true;
      event.preventDefault();
      const candidate = placement.current?.preview(current.object, event);
      setFeedback({
        x: event.clientX,
        y: event.clientY,
        valid: Boolean(candidate && !candidate.problem),
        message: candidate
          ? (candidate.problem ?? 'Release to place · Esc to cancel')
          : 'Drag onto the floor · Esc to cancel',
      });
    };
    const finish = (event: PointerEvent) => {
      const current = gesture.current;
      if (!current || current.pointerId !== event.pointerId) return;
      if (event.type !== 'pointerup') suppressClick.current = true;
      const candidate =
        current.dragging && event.type === 'pointerup' && !latest.current.disabled
          ? placement.current?.preview(current.object, event)
          : undefined;
      cancel.current();
      if (candidate && !candidate.problem) {
        try {
          latest.current.commit(candidate.object, current.pack);
        } catch (cause) {
          setError(cause instanceof Error ? cause.message : 'Cannot place this object.');
        }
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !gesture.current) return;
      suppressClick.current = true;
      event.preventDefault();
      event.stopPropagation();
      cancel.current();
    };
    const blur = () => {
      if (gesture.current) suppressClick.current = true;
      cancel.current();
    };
    const hidden = () => {
      if (document.hidden) blur();
    };
    window.addEventListener('pointermove', move, { passive: false });
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
    window.addEventListener('lostpointercapture', finish);
    window.addEventListener('keydown', escape, true);
    window.addEventListener('blur', blur);
    document.addEventListener('visibilitychange', hidden);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      window.removeEventListener('lostpointercapture', finish);
      window.removeEventListener('keydown', escape, true);
      window.removeEventListener('blur', blur);
      document.removeEventListener('visibilitychange', hidden);
      cancel.current();
    };
  }, []);
  const handlers: CatalogDragHandlers = {
    start(event, pack, key) {
      suppressClick.current = false;
      // Touch retains native catalog scrolling and the click alternative.
      if (
        event.pointerType === 'touch' ||
        event.button !== 0 ||
        latest.current.disabled ||
        !placement.current ||
        gesture.current
      )
        return;
      setError(undefined);
      gesture.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        button: event.currentTarget,
        pack,
        object: {
          id: crypto.randomUUID(),
          kind: 'decoration',
          surface: { type: 'floor' },
          placement: catalogFurniture(pack, key, 0, 0),
          extension: null,
        },
        dragging: false,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    consumeClick(detail) {
      const suppress = detail === 0 ? false : suppressClick.current;
      suppressClick.current = false;
      return suppress;
    },
  };
  return { placement, handlers, feedback, error };
}
