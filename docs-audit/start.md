# Readability audit: start

Source: `site/src/chapters/start.mdx` at blob `4755caaba1c2fef857d8c058efef7a3a159f8c49` (baseline `c7622d2c`). Line references refer to that unchanged source. Rendered component copy is identified separately where relevant.

Editorial findings and proposed structure only; no replacement prose. Sentence length is a screening signal, not an automatic defect. Preserve shipped/planned labels, consent gates, uncertainty behavior and data-retention limits during any later rewrite.

## Sentence load

- L37: The installer sentence combines install location, terminal condition, setup discovery, skills/hooks and consent; split install from the optional setup experience.
- L41: The runtime-driver sentence combines immediate usability, per-provider setup, conversation ownership, exact resume and status.
- L84: The start-then-name sentence combines generated naming, shell-tool location and preserved conversation; use a short sequence.
- Rendered Hero, strings.ts L23: The opening lede carries request/receipt, additive architecture, future layers and current integrations in one paragraph. Keep the authority/status qualifications while reducing competing ideas.

## Terms before explanation

- L37: PATH and session hooks are introduced before they are explained.
- L41, L69: host/runtime driver and driver:session are unexplained for a first-time reader.
- L25 and rendered Journey: talk/reply/receipt are named early, but the minimal runnable loop is much later. Distinguish current board availability from Colab/Meet previews.

## Repetition and ownership

- The foundation claim appears in Hero, L25, Journey captions and the showcase; retain one core explanation and let scenes demonstrate the layers.
- L45–54 and L75–89 show two naming workflows. Lead with one recommended path and keep the alternate as a later option.
- L41 and drivers.mdx repeat the setup benefits; keep a compact home summary and link the capability table.

## Where a table or diagram helps

- The existing Hero/Journey/scenes are useful. A first-success strip joining install → named agent → talk → submitted reply would connect them to the later examples.
- L69–73: Annotate the existing LsSample for saved/temporary, driver address and action columns.
- The broad foundation/layer story should distinguish alpha, in-progress and planned at every selected step.

## Section order

- This file is the home page at /, not a separate /start route.
- Keep the visual core story, but surface install and first-success navigation before the long capability tour.
- Put one named launch/talk/reply path before the alternative name-existing-pane path, with resume as the next task.

## Proposed outline

1. Core visual story and current integrations
2. Install and consented setup
3. First success: named agent, talk and reply
4. Core words and receipt correlation
5. One foundation: board alpha, Colab in progress, Meet planned
6. Read the identity list and manage lifetimes
7. Alternative existing-pane naming
8. Resume the same conversation; links to working/drivers
