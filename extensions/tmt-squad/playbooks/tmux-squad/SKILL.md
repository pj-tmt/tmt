---
name: tmux-squad
description: Propose a tmux layout for a TMT squad - a leads session, a crew session, one window and git worktree per member, and the board on a hotkey. Guidance only; nothing here runs without the user's agreement.
---

# tmux squad playbook (optional)

Use this when you lead a squad (see the `tmt-squad` skill), the user works in
tmux, and they want members laid out predictably. It is a suggestion, not a
procedure. Squad never runs it, and you don't run a step until the user has
agreed to it. Propose the commands, show what they will change, and wait.

## The layout

- **`leads`** session: one window per lead (you, and any other lead).
- **`crew`** session: one window per member, named after the member, each in
  its own git worktree.
- **The board** on a hotkey: `prefix S` opens it as a popup that closes when you
  jump, `prefix B` as a pane that stays open. `jump` works across sessions and
  `back` returns the user's client to where the jump came from.

## Agree on the details first

Ask, then record the answers in your notes (`tmt notes path`):

- the repository and the base branch members start from;
- member names (they become identity names and window names) and branch names;
- which agent each member runs;
- where worktrees live. The examples put them beside the repository as
  `<repo>-<member>`.

## Start the leads session

Skip this if you already run in the session the user wants as `leads`.

```sh
tmux new-session -d -s leads -n lead -c "$(git rev-parse --show-toplevel)"
tmux send-keys -t "leads:=lead" "tmt run -s lead claude" Enter
tmt squad init product --me <the user's saved identity>   # once per squad
tmt squad lead lead
```

## Add a member

Run from the repository. `crew` is created by the first member.

```sh
member=auth-fix; branch=feat/auth-fix; base=main
top="$(git rev-parse --show-toplevel)"
worktree="$(dirname "$top")/$(basename "$top")-$member"
git worktree add -b "$branch" "$worktree" "$base"
if tmux has-session -t crew 2>/dev/null; then
  tmux new-window -d -t crew: -n "$member" -c "$worktree"
else
  tmux new-session -d -s crew -n "$member" -c "$worktree"
fi
tmux send-keys -t "crew:=$member" "tmt run -s $member claude" Enter
```

Replace `claude` with the agent the user chose. When `tmt ls` shows the member
active, put it on the board and say what it is doing:

```sh
tmt squad add "$member"
tmt squad set "$member" state=working task="rotate session tokens" branch="$branch"
```

Members already running elsewhere skip the worktree and window steps; `tmt
squad add` is enough.

## Show the board

Squad installs the hotkeys only with the user's consent:

```sh
tmt squad hotkeys install --print   # what would change; changes nothing
tmt squad hotkeys install           # shows the plan and asks
```

## Retire a member

Do these in order, and stop at the first step the user hasn't approved.

```sh
tmt squad remove "$member"                      # off the board; the agent keeps running
git -C "$worktree" status --short               # anything uncommitted or untracked?
git -C "$worktree" log --oneline "$base..HEAD"  # commits that are not in the base branch?
```

If either command prints anything, tell the user and ask what to do with that
work before going on. Once the member's agent is finished and the work is
merged or pushed:

```sh
tmux kill-window -t "crew:=$member"
git worktree remove "$worktree"
git branch -d "$branch"
```

`git worktree remove` and `git branch -d` refuse when work would be lost. Never
add `--force` or use `-D` unless the user tells you to. The member's identity
stays saved and offline; `tmt rm <name> --force` removes it, only if the user
wants that.
