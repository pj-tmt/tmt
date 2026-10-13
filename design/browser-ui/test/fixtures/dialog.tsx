import { useRef, useState } from 'react';
import { BrowserDialog } from '../../src/dialog.js';

export function DialogFixture() {
  const [open, setOpen] = useState(false);
  const [opener, setOpener] = useState(true);
  const [reason, setReason] = useState('none');
  const [presses, setPresses] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const fallback = useRef<HTMLButtonElement>(null);
  return (
    <>
      <header className="tmt-ui-header">
        {opener && (
          <button type="button" onClick={() => setOpen(true)}>
            Open search
          </button>
        )}
        <button type="button" ref={fallback} onClick={() => setPresses((value) => value + 1)}>
          Header action
        </button>
      </header>
      <output data-testid="dialog-reason">{reason}</output>
      <output data-testid="header-presses">{presses}</output>
      <iframe
        title="Author content"
        srcDoc='<button onclick="document.body.dataset.presses = String(Number(document.body.dataset.presses || 0) + 1)" style="width:100%;height:300px">Author button</button>'
        style={{ width: '100%', height: 320 }}
      />
      <BrowserDialog
        open={open}
        title="Search pages"
        initialFocus={() => input.current}
        focusFallback={() => fallback.current}
        onDismiss={(why) => {
          setReason(why);
          setOpen(false);
        }}
      >
        <input aria-label="Search pages" ref={input} />
        <button type="button" onClick={() => setOpener(false)}>
          Remove opener
        </button>
        <button type="button">Last action</button>
      </BrowserDialog>
    </>
  );
}
