import { drawWhiteboard, drawWhiteboardSelection } from './drawing.js';
import { checkSnapshotImage } from './snapshot-contract.js';
import type { WhiteboardSnapshot } from './snapshot-contract.js';

/** Render the retained capture, never read pixels from the mutable editor canvas. */
export async function renderSnapshotImage(snapshot: WhiteboardSnapshot): Promise<Blob> {
  const canvas = document.createElement('canvas');
  canvas.width = snapshot.scene.width;
  canvas.height = snapshot.scene.height;
  try {
    const context = canvas.getContext('2d', { alpha: false, colorSpace: 'srgb' });
    if (!context) throw new Error('Whiteboard image rendering is unavailable.');
    drawWhiteboard(context, snapshot.scene);
    drawWhiteboardSelection(context, snapshot.scene, snapshot.selectedElementIds);
    return checkSnapshotImage(
      await new Promise<Blob>((resolve, reject) => {
        canvas.toBlob(
          (blob) =>
            blob ? resolve(blob) : reject(new Error('Whiteboard image could not be encoded.')),
          'image/png'
        );
      })
    );
  } finally {
    canvas.width = 0;
    canvas.height = 0;
  }
}
