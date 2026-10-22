import { FramePair, Section } from './kit.js';

/**
 * The real Colab Chat input (Lexical composer, @ agent list, chips, attachments) in its own Chat
 * drawer. The frame page lives in Colab's app (`test/agent-input-demo.tsx`): the leaf cannot import
 * Colab, so the gallery shows it through frames, like the shell. Nothing is sent.
 */
export function AgentInputSection() {
  return (
    <Section id="agent-input">
      <FramePair
        title="Chat input · desktop"
        note="Type, then @ for the agent list (↑ ↓ choose, Enter picks, Esc closes). Attach opens a file picker; a file named refuse… is refused."
        frame="agent-input"
        width={1080}
        height={520}
        params={{ state: 'idle' }}
      />
      <FramePair
        title="Multi-line draft"
        note="A long message with line breaks keeps the chip, the attachments and the send row in view."
        frame="agent-input"
        width={480}
        height={520}
        params={{ state: 'draft' }}
      />
      <FramePair
        title="Sending: the write is held"
        note="Press Send. The message shows at once and the input stays free while the write is held."
        frame="agent-input"
        width={480}
        height={520}
        params={{ state: 'busy' }}
      />
      <FramePair
        title="Refused: the write fails"
        note="Press Send. The message stays as a failed row; Retry sends it again, Edit brings it back."
        frame="agent-input"
        width={480}
        height={520}
        params={{ state: 'refused' }}
      />
      <FramePair
        title="Chat input · phone (390 px)"
        note="The drawer fills the screen; the agent list opens under the chip."
        frame="agent-input"
        width={390}
        height={640}
        params={{ state: 'idle' }}
      />
    </Section>
  );
}
