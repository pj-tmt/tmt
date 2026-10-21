# Page work

A page can carry a two-line summary and a task list: use them when a page drives work (a plan, a
spec, a review that turns into tasks). A page with tasks is still an ordinary page; its stage,
progress and who is working are computed from the tasks and shown in `tmt colab ls`.

```sh
tmt colab task add PAGE --title 'Write the guide' --assignee ada --json   # returns taskId
tmt colab task state PAGE T-1 in-progress --json
tmt colab task state PAGE T-1 blocked --reason 'Waiting for the API' --json
tmt colab task assign PAGE T-1 person:ben --json   # no assignee clears it
tmt colab task set PAGE T-1 --file task.json --json
tmt colab task rm PAGE T-1 --json
tmt colab task ls PAGE --json
tmt colab summary set PAGE --line1 'Release plan' --line2 'Read before the release' --json
```

States are `todo`, `in-progress`, `blocked` (needs `--reason`), `in-review` and `done`. A new
task starts as `todo`, and its ID (`T-1`, `T-2`, ...) is never reused. The summary's first line is
at most 24 characters and says where the page stands; the second is at most 60 and says what
comes next. `task set` replaces the title, detail, acceptance criteria, sign-off flag and external
ID as a whole (`--file` JSON; an omitted field is cleared); state and assignee have their own
commands. `--signoff` on `task add` marks a task that waits for a person in review; nothing acts on
the flag yet.

Every change applies to the page's current revision, or to `--expected-revision` when you pass one;
`COLAB_STALE_BASE` means the page changed, so read it again. An invalid value refuses the whole
change.

## Limit

An agent holds a task while it is assigned to it and the task is `todo` or `in-progress`, and holds
at most its limit across all pages (1 unless set). A change that would exceed it is refused as
`COLAB_WIP_LIMIT` naming the agent and its tasks: finish, block or reassign one first. A person
changes the limit with `tmt colab settings wip-limit [AGENT] N`; do not raise it yourself to get past a
refusal.

## GitHub

A task can be pushed to a GitHub issue and marked done from GitHub. Set the space's repository once
(a person does this): `tmt colab settings github-repo OWNER/REPO`.

```sh
tmt colab task github push PAGE T-3 --json   # publishes T-3 as an issue, links it as #N
tmt colab task github sync --json            # marks linked tasks done after a merged Fixes #N
```

`push` publishes the task's title, detail and acceptance to GitHub, possibly to a public repository:
run it only for a task the person asked you to share, for one task at a time, and say which repository
and issue (it prints both). `sync` never publishes anything; it asks GitHub about open linked tasks
(`#N` external IDs) in as few calls as it can and moves a task to `done` only when a merged pull
request closed its issue. `COLAB_GITHUB_UNAVAILABLE` means `gh` is missing or not signed in: tell the
person, do not retry in a loop. Edits made on GitHub are never read back.
