---
name: git-bug
description: Read and manage this project's tasks and bugs, which live in git-bug (issues stored in the repo under refs/bugs), not in a roadmap file or GitHub Issues. Use when asked what's next, to list, open, update, comment on or close a task or bug, when work finishes something an issue tracks, or when a follow-up or bug turns up that should be recorded.
---

# git-bug

Tasks and bugs for tread-pod are git-bug issues. There is no roadmap file. git-bug comes from the
devenv shell (nixpkgs-unstable, >= 0.12), so run every command through it:

```bash
devenv shell -- git bug bug
```

Filter the devenv banner lines (`✓ …`, `• …`, `tread-pod dev shell …`) out of the output when
reading it.

## Reading

```bash
git bug bug                          # open and closed, oldest first
git bug bug status:open              # open only
git bug bug -s open -l cadence       # by label
git bug bug "accelerometer"          # full-text search
git bug bug -s open -f json          # machine-readable
git bug bug show <id>                # title, labels, comments
```

IDs are hex prefixes; the 7-character short ID from the list is enough.

## Writing

Always pass the text on the command line with `--non-interactive`, or git-bug opens an editor.
Never use `git bug bug select`: it is shared state across every checkout and worktree.

```bash
id=$(git bug bug new --non-interactive -t "Title" -m "Body" | awk '{print $1}')
git bug bug label new "$id" task cadence
git bug bug comment new "$id" --non-interactive -m "What was found or done"
git bug bug title edit "$id" --non-interactive -t "Better title"
git bug bug label rm "$id" someday
git bug bug status close "$id"
```

When closing an issue, first add a comment saying what settled it (the commit, or what a run showed).

## Conventions

- Every issue has exactly one kind label: `task` (work to do) or `bug` (something broken).
- Plus one or more area labels: `engine`, `firmware`, `button`, `cadence`, `config-ui`,
  `config-bluetooth`, `garmin-connect`, `hardware`, `feature`, `release`, `project` (tooling,
  docs, repo setup). Add a new area label only when none fits.
- Closed issues are the history of what's done: each has a closing comment naming the commits that
  did it (and what a run showed, if it was a check).
- `someday` marks ideas that are not planned. Leave those out when asked what's next.
- Titles are short imperative or noun phrases: "Pin a treadmill from the web UI".
- Bodies stand alone: give the context (why it matters, what a test run showed, with dates), then
  what to do and how to check it. Wrap at about 100 columns.
- Refer to other issues by short ID in the text ("Follow-up to 8539811"), and to code by path or
  item name (`engine::watch::Gate`).

## Syncing

Issues are local until pushed. `git bug pull` and `git bug push` sync them with `origin`. Both are
outward-facing, so only run them when the user asks.
