# Project guidance

## Purpose

This repository maintains two compatible implementations of the
`agents-work` filesystem protocol. The Python implementation is the behavioral
reference. The Rust implementation must preserve its command grammar, accepted
inputs, diagnostics, exit classes, and filesystem effects unless a documented
protocol change deliberately updates both.

## Protocol boundary

- Keep Markdown artifacts human-readable and append-only.
- Keep `work.toml` a small mutable coordination cursor, never an artifact
  index or durable history.
- Do not add a service, database, daemon, scheduler, network dependency, issue
  tracker, or plugin framework without an explicit design decision.
- Do not treat an artifact or cursor value as authorization to act.
- Never run mutating tests against a live `~/.agents/work` tree. Use isolated
  temporary fixtures.

## Compatibility

- Change the Python reference and its tests first for intentional behavior
  changes.
- Exercise the Rust implementation against the Python differential oracle.
- Preserve existing on-disk data without migration unless a migration is
  explicitly designed and tested.
- Document intentional implementation differences where exact parity is not
  possible.

## Verification

Run the Python tests, Rust formatting, strict Clippy, ordinary Rust tests,
ignored differential tests, and `nix flake check 'path:.'` before calling a
change complete. Keep `flake.lock` and `rust/Cargo.lock` committed.

## Version control

Integrate branches with merge commits. Never squash or rebase. Commit logical
units locally, and ask before pushing or opening a pull request.
