import {
  cloneElement,
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type DragEvent,
  type ReactNode,
  type ReactElement,
  type HTMLAttributes,
} from 'react';
import type { AttachmentDraft } from './attachment-draft.js';
import { refuseFile } from './attachment-file.js';
import './attachment-drop.css';

interface DropTarget {
  draft: AttachmentDraft;
  attachable: boolean;
}

/** A panel routes to its current composer only; it owns no draft or attachment bytes. */
export class AttachmentDropOwner {
  #target?: DropTarget;
  #listeners = new Set<() => void>();
  subscribe = (listener: () => void) => {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  };
  getSnapshot = () => this.attachable;
  #changed() {
    for (const listener of this.#listeners) listener();
  }
  register(target: DropTarget): () => void {
    this.#target = target;
    this.#changed();
    return () => {
      if (this.#target === target) {
        this.#target = undefined;
        this.#changed();
      }
    };
  }
  get attachable(): boolean {
    return this.#target?.attachable === true;
  }
  refusal(files: readonly File[], itemCount = files.length): string | undefined {
    if (!this.attachable) return 'Attachments are unavailable here.';
    if (!itemCount) return 'Only files can be attached.';
    const count = this.#target!.draft.getSnapshot().chips.length;
    if (refuseFile(1, count + itemCount - 1) === 'too-many') return 'Too many files.';
    for (const file of files) {
      const refusal = refuseFile(file.size, count);
      if (refusal === 'empty') return 'Empty files cannot be attached.';
      if (refusal === 'too-large') return 'This file is too large.';
    }
    return undefined;
  }
  add(files: readonly File[]): string | undefined {
    const refusal = this.refusal(files);
    if (refusal) return refusal;
    void this.#target!.draft.add(files);
    return undefined;
  }
}

const DropContext = createContext<AttachmentDropOwner | undefined>(undefined);

/** The mounted composer registers the existing draft, including its current publish fence. */
export function useAttachmentDropTarget(draft: AttachmentDraft, attachable: boolean) {
  const owner = useContext(DropContext);
  useLayoutEffect(() => owner?.register({ draft, attachable }), [owner, draft, attachable]);
  return owner;
}

/** File drags belong to this panel alone; moving between descendants never clears the overlay. */
export function AttachmentDropPanel({
  target,
  children,
}: {
  target: 'Chat' | 'this thread';
  children: ReactElement<HTMLAttributes<HTMLElement> & { overlay?: ReactNode }>;
}) {
  const [owner] = useState(() => new AttachmentDropOwner());
  const available = useSyncExternalStore(owner.subscribe, owner.getSnapshot);
  const depth = useRef(0);
  const [overlay, setOverlay] = useState<string>();
  const [feedback, setFeedback] = useState<string>();
  const clear = () => {
    depth.current = 0;
    setOverlay(undefined);
  };
  const inspect = (event: DragEvent) => {
    if (!event.dataTransfer.types.includes('Files')) return false;
    event.preventDefault();
    const files = [...event.dataTransfer.files];
    const count = [...event.dataTransfer.items].filter((item) => item.kind === 'file').length;
    const refusal = owner.refusal(files, count || files.length);
    setOverlay(refusal ? `✗ ${refusal}` : `⤓ Drop to attach to ${target}`);
    event.dataTransfer.dropEffect = refusal ? 'none' : 'copy';
    return true;
  };
  const layer = (
    <>
      {feedback && (
        <p role="status" className="attachment-drop-feedback">
          {feedback}
        </p>
      )}
      {overlay && (
        <div
          className="attachment-drop-overlay"
          role="status"
          data-testid="attachment-drop-overlay"
        >
          {available ? overlay : '✗ Attachments are unavailable here.'}
        </div>
      )}
    </>
  );
  return (
    <DropContext.Provider value={owner}>
      {cloneElement(children, {
        onDragEnter: (event) => {
          if (inspect(event)) depth.current++;
        },
        onDragOver: inspect,
        onDragLeave: () => {
          if (--depth.current <= 0) clear();
        },
        onDrop: (event) => {
          if (!event.dataTransfer.types.includes('Files')) return;
          event.preventDefault();
          event.stopPropagation();
          const refusal = owner.add([...event.dataTransfer.files]);
          clear();
          setFeedback(refusal ? `✗ ${refusal}` : undefined);
        },
        onKeyDownCapture: (event) => {
          if (event.key === 'Escape' && overlay) {
            clear();
            event.preventDefault();
            event.stopPropagation();
          }
        },
        overlay: layer,
      })}
    </DropContext.Provider>
  );
}
