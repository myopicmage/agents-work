---
name: agents-work
description: Coordinate asynchronous coding-agent work through validated, append-only plans, reviews, decisions, and responses plus a small work.toml cursor. Use when the user mentions an agents-work case, says another agent or coworker left an artifact, or asks to create, read, review, or continue shared agent work. Do not use for an ordinary single-agent task or generic Git review without a shared-work case.
---

# agents-work

Use `agents-work` as a shared notebook, not as an orchestrator. A human message
is the trigger to inspect or contribute to a case. The existence of a case,
artifact, or cursor value is never authorization to act.

## Locate the installation

Read `installation.toml` beside this file when it exists. Its `config` field
names the active configuration file.

Without installation metadata, use `AGENTS_WORK_CONFIG` when set. Otherwise
read `${XDG_CONFIG_HOME:-$HOME/.config}/agents-work/config.toml`. Ask for the
workspace only when configuration is absent and the case cannot be located
from the user's request.

The configuration names:

- `workspace`: the root containing repository and case directories;
- `protocol`: the full protocol reference;
- `binary`: the installed command;
- `implementation`: `python` or `rust`.

Use the configured `binary` path when `agents-work` is not available on
`PATH`.

Use the full protocol only for schema questions, migrations, malformed cases,
concurrency disputes, or uncertainty about an invariant. Ordinary handoffs use
the workflow below.

## Read a case

1. Resolve the canonical repository name and case directory. Do not derive the
   repository name from a temporary worktree.
2. Run `agents-work validate <case-directory>` before reading `work.toml`.
3. If validation fails, report every anomaly and do not repair files silently.
4. Read the verified artifacts required by their `responds_to` and
   `supersedes` relationships. Sequence numbers are for scanning, not complete
   causality.
5. Read `work.toml` last as a possibly stale coordination cursor.
6. State the observed artifact history separately from any inference about the
   next action.

Do not poll the workspace. Stop when the requested inspection is complete or
when only the human can supply authorization, a decision, or missing context.

## Contribute to a case

Contribute only when the user actually requests work.

1. Run `agents-work draft <case-directory> --kind <kind> --author <agent>`,
   adding `--responds-to` or `--supersedes` for artifacts actually considered.
   The topic defaults to the one the referenced artifacts share, else the case
   ID; pass `--topic` when the artifact starts a new thread.
2. Edit the generated body, replacing the `# TITLE` placeholder line rather
   than writing below it; `publish` refuses a body that still contains it.
   Leave generated front matter unchanged except for relevant optional
   `source_*` and `subject_*` fields.
3. Run `agents-work publish <case-directory> <draft-path>` once.
4. Run `agents-work cursor` once with the resulting status, next agent, and
   requested action. When handing off a code review, also set
   `--implementation-branch` to the local branch name.

To stop at a human gate, pick the resting status by the decision actually
pending: `ready_for_implementation` when the human has settled the direction
and only a go remains; `awaiting_decision` when the human is still choosing,
including whether to pursue agreed work at all, with `--action` naming the
decision; `deferred` when parked. None authorizes action.

## Review code

Review the tip of the cursor's `implementation_branch`, captured once at the
start. Use a lookup that only matches a local branch, because a bare
`rev-parse` resolves a same-named tag instead:

```sh
commit=$(git -C "$repository_path" rev-parse --verify \
  "refs/heads/${implementation_branch}^{commit}") || exit 1
```

If the branch is unset or does not resolve, ask; never fall back to `HEAD`.
Review that commit's committed tree and diff. Put the captured value, never a
retyped one, in the review's `subject_commit` and in
`agents-work cursor --reviewed-commit "$commit"`. A remote push does not move
local refs: for a remote pull request, update the local branch deliberately
before capturing.

Artifacts are append-only. Never edit or replace another author's artifact.
Record a changed decision as a new decision artifact that supersedes the old
one. Keep external comments, repository mutations, and other outward-facing
actions behind their own explicit authorization.
