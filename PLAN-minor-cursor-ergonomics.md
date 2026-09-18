# Minor cursor ergonomics: converged plan

Status: implemented (see Execution status). Supersedes plan 005 and adopts review
006 in full. Codex stated no agreement-only follow-up is needed.
Baseline: `ac33ced8588169f3a9fb6dbcfeab83df6ee7763c` (verifier green, per
response 004).

## Changes from plan 005

1. **Branch resolution is verified and local-branch-only** (review 006,
   required). Claude reproduced the finding independently: with a tag and
   a branch both named `review-target` on different commits, bare
   `git rev-parse review-target` warned and returned the tag's commit;
   `rev-parse --verify "refs/heads/review-target^{commit}"` returned the
   branch's.
2. **Freshness boundary stated**: the recipe captures the latest local
   branch tip; a remote push does not move local refs.
3. **Status selection keys on the pending decision**, and agent agreement
   is not Kevin's decision (review 006, answer 2).
4. Open questions from 005 are closed: branch handoff stays documentation
   only (review 006, answer 1, matching Claude's lean).

## Decision record

Kevin decided on 2026-09-18 to drop plan 003's `--record-reviewed-commit`
flag. Agents should know which branch to review and review its latest
commit, rather than carry a hash between commands. The flag was opt-in, so
it did not prevent forgetting, and its only benefit (no transcription) cost
a two-commit-point operation with partial-success handling. Deriving the
commit from the branch removes transcription at the source.

## Scope contract

1. Add one resting status, `awaiting_decision`.
2. Document branch-based review.
3. Document the responsibility boundary: the cursor routes, artifacts
   preserve evidence, external systems supply current state.

Only (1) changes CLI behavior. Python is the behavioral reference; Rust
mirrors (1) and passes the differential tests. Existing commands, manifest
fields, artifact metadata, phases and schema versions stay.

Single writer is an explicit assumption. Excluded: concurrency mechanisms;
services, schedulers, polling or authorization mechanisms; new commands,
flags or manifest fields; schema migration or rewriting existing cases;
status inference, verdict parsing or Git/network inspection by the tool;
enforcement of the branch handoff; installation, packaging, dependency or
formatter changes; dotfiles, Iron-Wok, global guidance or installed-skill
changes. If an excluded item turns out to be required, stop and explain.

## 1. Add `awaiting_decision`

Add it to the resting set in Python and Rust. Permit it in planning,
implementation and pr_review; complete still requires complete.
`cursor --status awaiting_decision` clears `next_agent` unless one is
supplied; a supplied agent is the resumption owner, never permission to
act. Existing statuses and cursors are untouched.

Selection rule, stated once in PROTOCOL.md and the skill. Choose from the
decision actually pending, not from wording such as "authorize":

- `ready_for_implementation`: Kevin has settled the direction and only a
  go instruction remains.
- `awaiting_decision`: Kevin is still choosing something, and
  `requested_action` names it. This includes whether to pursue proposed
  work at all, even when the agents agree on the plan: agent agreement
  does not establish Kevin's decision. Example: "Decide whether to merge
  the reviewed branch."
- `deferred`: parked; no decision is being requested.

All three are gates. None authorizes action.

## 2. Branch-based review (documentation only)

Add to PROTOCOL.md's `reviewed_commit` section and the skill's review
workflow:

- **The author names the target.** When handing off a code review, the
  author sets `implementation_branch` to the local branch name (not a full
  ref or revision expression) in the same cursor command.
- **The reviewer captures the tip once, at the start**, with a verified
  local-branch lookup:

  ```sh
  commit=$(git -C "$repository_path" rev-parse --verify \
    "refs/heads/${implementation_branch}^{commit}") || exit 1
  ```

  If `implementation_branch` is empty or the branch does not resolve, ask
  for the handoff context. Never fall back to HEAD.
- **Review the captured commit's committed tree and diff**, not a checkout
  that may move or carry uncommitted files.
- **The captured value goes to both places, never retyped**: the review
  artifact's `subject_commit` and `cursor --reviewed-commit "$commit"`.
- **Freshness boundary.** Linked worktrees share local branch refs, so the
  recipe sees the latest local tip from any worktree. A remote push does
  not update local refs. When the subject is a remote PR, check its current
  source state and deliberately update or select the matching local branch
  before capture. The tool does not fetch or synchronize.
- If the captured branch advances during the review, the review still
  covers the captured commit; later commits appear as unreviewed by
  comparing `reviewed_commit` to the branch tip, as PROTOCOL.md already
  describes.

## 3. Responsibility boundary (documentation only)

`requested_action` routes ("Address review 002"). Artifacts hold dated
evidence, including the reviewed commit. Git, PR and deployment systems
hold current branch, merge and deployment state, checked when an action
depends on them. The cursor is not a remote-status cache. Writing guidance
only; no policing.

## Implementation boundaries

Expected to change:

- `python/agents_work.py`, `python/test_agents_work.py`.
- `rust/src/cli.rs`, `rust/src/manifest.rs`.
- `rust/tests/cli_contract.rs`, `rust/tests/manifest_contract.rs`,
  `rust/tests/cursor_contract.rs`, `rust/tests/validate_contract.rs`
  (coordination matrix gains the new status).
- `PROTOCOL.md`, `skills/agents-work/SKILL.md`, `README.md`,
  `COMPATIBILITY.md`, and this plan's execution-status note.

`rust/src/publish.rs`, `rust/src/cursor.rs`, `rust/src/main.rs` and
`rust/tests/publish_contract.rs` should not change. If they must, say why.

## Verification

1. Python first: new status accepted in non-complete phases, rejected with
   complete phase, resting-owner behavior unchanged.
2. Rust mirror, with the coordination matrix extended so the new status is
   compared against the Python oracle.
3. Docs updated; `nix develop 'path:.' --command ./scripts/check` passes
   from an isolated worktree; `git diff --check`; changed files match the
   list above. Commit locally. Push, PR and installation are separate.

| Case | Expected |
| --- | --- |
| New status via cursor and validate | Accepted in planning, implementation, pr_review |
| New status with complete phase | Rejected by the existing invariant |
| `cursor --status awaiting_decision` without `--next-agent` | Clears owner, like other resting statuses |
| Existing statuses, publish, manual `--reviewed-commit` | Unchanged |
| Python/Rust matrix | Identical accept/reject and diagnostics |

## Completion criterion

One new status; documented branch-based review; documented responsibility
boundary; Python/Rust parity; green verifier. Nothing else. This plan does
not authorize implementation.

## Execution status

Implemented on `feature/awaiting-decision` from baseline `ac33ced`.
Planning record: shared case `agents-work/minor-cursor-ergonomics-20260918`,
plans 001, 003, 005 and 007 with reviews 002 and 006 and response 004.

- Python reference: `awaiting_decision` added to the resting statuses, with
  tests for every open phase, owner clearing, an explicit resumption owner
  and complete-phase rejection.
- Rust: status enum, CLI values and error vocabulary mirrored; the
  Python-oracle coordination matrix now includes the new status.
- Docs: PROTOCOL.md (status selection rule, branch-based review, where each
  kind of fact lives), the skill, one README example and a COMPATIBILITY
  grammar-additions note.
- Changed files match the implementation boundaries. `publish.rs`,
  `cursor.rs`, `main.rs` and `publish_contract.rs` are untouched.

No deviations from the plan. Rollout, meaning updating installed binaries
and the installed skill, is a separate step.
