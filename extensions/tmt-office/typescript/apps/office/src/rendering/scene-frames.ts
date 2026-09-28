/** One demand-driven frame owner per mounted scene; no idle animation loop. */
export function createSceneFrames(render: () => void) {
  let frame: number | undefined;
  let dirty = false;
  let disposed = false;

  function schedule() {
    if (disposed || document.hidden || !dirty || frame !== undefined) return;
    frame = requestAnimationFrame(() => {
      frame = undefined;
      if (disposed || document.hidden || !dirty) return;
      dirty = false;
      render();
    });
  }

  function visibilityChanged() {
    if (document.hidden && frame !== undefined) {
      cancelAnimationFrame(frame);
      frame = undefined;
    }
    schedule();
  }
  document.addEventListener('visibilitychange', visibilityChanged);

  return {
    invalidate() {
      if (disposed) return;
      dirty = true;
      schedule();
    },
    dispose() {
      if (disposed) return;
      disposed = true;
      document.removeEventListener('visibilitychange', visibilityChanged);
      if (frame !== undefined) cancelAnimationFrame(frame);
      frame = undefined;
    },
  };
}
