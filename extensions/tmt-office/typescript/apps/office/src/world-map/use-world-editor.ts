import { useCallback, useEffect, useRef, useState } from 'react';
import { createWorldYjs } from './world-yjs.js';
import { sameWorld } from './world-contract.js';
import type { WorldDocument } from './world-contract.js';
import { WorldConflict, WorldValidationError } from './world-port.js';
import type { WorldPort, WorldSnapshot, WorldPlacementIssue } from './world-port.js';

/** One Yjs history and one serialized, revision-fenced auto-apply queue. */
export function useWorldEditor(initial: WorldSnapshot, port: WorldPort) {
  const [saved, setSaved] = useState(initial);
  const [history] = useState(() => createWorldYjs(initial.layout));
  const [world, setWorld] = useState(() => history.world);
  const current = useRef({ saved });
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [blocked, setBlocked] = useState(false);
  const [error, setError] = useState<string>();
  const [issues, setIssues] = useState<readonly WorldPlacementIssue[]>([]);
  const request = useRef<AbortController | undefined>(undefined);
  const blockedReason = useRef<'validation' | 'conflict' | 'uncertain' | undefined>(undefined);
  useEffect(() => {
    history.resume();
    return () => {
      request.current?.abort();
      history.destroy();
    };
  }, [history]);
  const dirty = !sameWorld(world, saved.layout);
  const historyChanged = useCallback(() => {
    // Publish a detached projection only when history changes, not on each render.
    setWorld(history.world);
  }, [history]);
  function acknowledge(next: WorldSnapshot) {
    current.current.saved = next;
    setSaved(next);
  }
  useEffect(() => {
    const state = current.current;
    if (
      !request.current &&
      !blocked &&
      sameWorld(history.world, state.saved.layout) &&
      initial.revision > state.saved.revision
    ) {
      acknowledge(initial);
      history.observe(initial.layout);
      historyChanged();
    }
  }, [initial, blocked, history, historyChanged, saving, busy, saved]);
  function change(edit: (world: WorldDocument) => WorldDocument, historyGroup?: string) {
    if (busy) return false;
    try {
      history.change(edit(history.world), historyGroup);
      historyChanged();
      if (!blocked) {
        setError(undefined);
        setIssues([]);
      }
      return true;
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'This change is invalid.');
      return false;
    }
  }
  const apply = useCallback(async () => {
    if (request.current || busy) return;
    const { saved } = current.current;
    const layout = history.world;
    if (sameWorld(layout, saved.layout)) {
      blockedReason.current = undefined;
      setBlocked(false);
      setError(undefined);
      setIssues([]);
      return;
    }
    const controller = new AbortController();
    request.current = controller;
    setSaving(true);
    setError(undefined);
    setIssues([]);
    try {
      const result = await port.save(
        { expectedRevision: saved.revision, legacyBasis: saved.legacyBasis, layout },
        controller.signal
      );
      if (controller.signal.aborted) return;
      acknowledge(result);
      blockedReason.current = undefined;
      setBlocked(false);
      // Keep newer local changes and undo history; the next write uses this acknowledgement.
    } catch (cause) {
      if (controller.signal.aborted) return;
      blockedReason.current =
        cause instanceof WorldValidationError
          ? 'validation'
          : cause instanceof WorldConflict
            ? 'conflict'
            : 'uncertain';
      setBlocked(true);
      setError(
        cause instanceof WorldValidationError || cause instanceof WorldConflict
          ? cause.message
          : 'Changes were not confirmed. Local changes are kept. Retry explicitly or reload the saved layout.'
      );
      if (cause instanceof WorldValidationError) setIssues(cause.issues);
    } finally {
      if (request.current === controller) request.current = undefined;
      if (!controller.signal.aborted) setSaving(false);
    }
  }, [busy, port, history]);
  useEffect(() => {
    if (!dirty || saving || busy || blocked) return;
    const timer = window.setTimeout(() => void apply(), 300);
    return () => window.clearTimeout(timer);
  }, [world, saved, dirty, saving, busy, blocked, apply]);
  useEffect(() => {
    if (!dirty && !saving) return;
    const warn = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty, saving]);
  async function reload() {
    if (request.current) return;
    const controller = new AbortController();
    request.current = controller;
    setBusy(true);
    try {
      const latest = await port.show(controller.signal);
      if (!controller.signal.aborted) {
        acknowledge(latest);
        history.reset(latest.layout);
        historyChanged();
        blockedReason.current = undefined;
        setBlocked(false);
        setError(undefined);
        setIssues([]);
      }
    } catch {
      if (!controller.signal.aborted) setError('Reload failed. Local changes are still kept.');
    } finally {
      if (request.current === controller) request.current = undefined;
      if (!controller.signal.aborted) setBusy(false);
    }
  }
  function travel(direction: 'undo' | 'redo') {
    if (busy) return;
    try {
      history[direction]();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'This history step is unavailable.');
    }
    historyChanged();
    if (
      direction === 'undo' &&
      blockedReason.current === 'validation' &&
      !request.current &&
      sameWorld(history.world, current.current.saved.layout)
    ) {
      blockedReason.current = undefined;
      setBlocked(false);
      setError(undefined);
      setIssues([]);
    }
  }
  return {
    world,
    saved,
    dirty,
    busy,
    saving,
    blocked,
    error,
    issues,
    historyNotice: history.historyNotice,
    change,
    stopCapturing: history.stopCapturing,
    reload,
    retry: apply,
    undo: () => travel('undo'),
    redo: () => travel('redo'),
    canUndo: history.canUndo,
    canRedo: history.canRedo,
  };
}
