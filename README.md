# tmt

> **Work in progress.** tmt is an early alpha under active development.
> Commands, storage and extensions change often, and only the alpha upgrade
> channel is available today.

## The story

tmt began as a question: how far can you get building a product from documents
and specs alone? We put a team of agents on it. Leads, implementers, reviewers
and a night shift work around the clock from those documents, and the product
they build and iterate on is tmt itself.

What we want out of it is an agent team manager for one person: no friction,
and yours to host and scale.

What exists today is the core: a layer that abstracts the agent harness and the
terminal, so agents in Claude Code, Codex and others can reach each other
wherever they run. On top of it we are trying out new ways for agents to work
together: squads on a live board, message digests, and shared pages that people
and agents edit side by side. Expect those parts to change.

## Try it

Install on macOS or Linux:

```sh
curl -fsSL https://install.tmt.ninja | sh
```

Set up your agents and the optional extensions:

```sh
tmt setup
```

Open the board to see your agents:

```sh
tmt ops ui
```

Upgrade later with:

```sh
tmt upgrade --channel alpha
```

Only the alpha channel works for now. Installing and upgrading need no GitHub
token.

## The core idea

Give each agent's pane a name, install the tmt skills, and your agents can talk
to each other across harnesses.

```sh
tmt this worker        # name the pane you are in
tmt add %3 reviewer    # or name another pane by its target
tmt install all        # install the tmt skills for your agents
tmt talk worker "Run the tests"
```

`tmt talk` waits for the agent's complete reply, and replies are kept even if a
pane closes. With the skills installed, agents use the same commands to ask and
answer each other. Run `tmt --help` or `tmt learn` for more.

## Learn more

- [Blog](https://tmt-ninja.github.io/tmt/blog/): how we run a team of agents on
  tmt, with real numbers.

## License

[MIT](LICENSE)
