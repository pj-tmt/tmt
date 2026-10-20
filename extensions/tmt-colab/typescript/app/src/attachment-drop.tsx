import {
  cloneElement,
  createContext,
  useContext,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ClipboardEvent,
  type DragEvent,
  type ReactNode,
  type ReactElement,
  type HTMLAttributes,
} from 'react';
import type { AttachmentDraft } from './attachment-draft.js';
import { refuseFile } from './attachment-file.js';
import { text } from './strings.js';
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
    if (!this.attachable) return text.attachDropRefusal.unavailable;
    if (!itemCount) return text.attachDropRefusal.filesOnly;
    const count = this.#target!.draft.getSnapshot().chips.length;
    if (refuseFile(1, count + itemCount - 1) === 'too-many') return text.attachDropRefusal.tooMany;
    for (const file of files) {
      const refusal = refuseFile(file.size, count);
      if (refusal === 'empty') return text.attachDropRefusal.empty;
      if (refusal === 'too-large') return text.attachDropRefusal.tooLarge;
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

/** The file handlers of a composer: a drop or a files-only paste adds inert chips to its draft, and
 * nothing is stored or sent until its Send. Under a drop panel (`panelOwns`) the panel takes the
 * drop. A paste that carries text stays a text paste. */
export function attachmentInputHandlers(
  draft: AttachmentDraft,
  attachable: boolean,
  panelOwns: boolean,
) {
  const take = (list: FileList | null | undefined, event: ClipboardEvent | DragEvent) => {
    if (!attachable || !list?.length) return;
    event.preventDefault();
    void draft.add([...list]);
  };
  return {
    onDragOver: (event: DragEvent) => {
      if (attachable && event.dataTransfer.types.includes('Files')) event.preventDefault();
    },
    onDrop: (event: DragEvent) => {
      if (!panelOwns) take(event.dataTransfer.files, event);
    },
    onPaste: (event: ClipboardEvent) => {
      if (!event.clipboardData.types.includes('text/plain')) take(event.clipboardData.files, event);
    },
  };
}

/** File drags belong to this panel alone; moving between descendants never clears the overlay. */
export function AttachmentDropPanel({
  target,
  active = true,
  children,
}: {
  target: keyof typeof text.attachDropTarget;
  active?: boolean;
  children: ReactElement<HTMLAttributes<HTMLElement> & { overlay?: ReactNode }>;
}) {
  const [owner] = useState(() => new AttachmentDropOwner());
  const available = useSyncExternalStore(owner.subscribe, owner.getSnapshot);
  const depth = useRef(0);
  const [overlay, setOverlay] = useState<{ refusal?: string }>();
  const [feedback, setFeedback] = useState<string>();
  const clear = () => {
    depth.current = 0;
    setOverlay(undefined);
  };
  useLayoutEffect(() => {
    if (!active) {
      depth.current = 0;
      setOverlay(undefined);
      setFeedback(undefined);
    }
  }, [active]);
  const admits = (event: DragEvent) => {
    if (!active) {
      if (event.dataTransfer.types.includes('Files')) event.preventDefault();
      return false;
    }
    const target = event.target;
    if (!(target instanceof Element)) return false;
    if (target.closest('[role="menu"]')) return false;
    const dialog = target.closest('dialog');
    return !dialog || dialog === event.currentTarget;
  };
  const inspect = (event: DragEvent) => {
    if (!admits(event)) return false;
    if (!event.dataTransfer.types.includes('Files')) return false;
    event.preventDefault();
    const files = [...event.dataTransfer.files];
    const count = [...event.dataTransfer.items].filter((item) => item.kind === 'file').length;
    const refusal = owner.refusal(files, count || files.length);
    setOverlay({ refusal });
    event.dataTransfer.dropEffect = refusal ? 'none' : 'copy';
    return true;
  };
  const layer = (
    <>
      {feedback && (
        <p role="status" className="attachment-drop-feedback">
          <span className="attachment-drop-mark" aria-hidden="true">
            ✗
          </span>{' '}
          {feedback}
        </p>
      )}
      {overlay && (
        <div
          className="attachment-drop-overlay"
          role="status"
          data-testid="attachment-drop-overlay"
        >
          <div className="attachment-drop-frame">
            <span>
              {(!available || overlay.refusal) && (
                <>
                  <span className="attachment-drop-mark" aria-hidden="true">
                    ✗
                  </span>{' '}
                </>
              )}
              {!available
                ? text.attachDropRefusal.unavailable
                : (overlay.refusal ?? text.attachDropPrompt(text.attachDropTarget[target]))}
            </span>
          </div>
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
          if (!admits(event) || !event.dataTransfer.types.includes('Files')) return;
          event.preventDefault();
          event.stopPropagation();
          const refusal = owner.add([...event.dataTransfer.files]);
          clear();
          setFeedback(refusal);
        },
        onKeyDownCapture: (event) => {
          if (
            event.key === 'Escape' &&
            overlay &&
            !(event.target instanceof Element && event.target.closest('[role="menu"]'))
          ) {
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
