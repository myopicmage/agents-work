# Implementation compatibility

The Python implementation is the behavioral reference. The Rust implementation
preserves:

- commands, arguments, and accepted values;
- exit-status classes and diagnostic meaning;
- artifact discovery and validation;
- deterministic artifact, sidecar, and manifest bytes where both runtimes can
  represent the value;
- filesystem commit points and no-clobber behavior.

The differential suite runs both commands against isolated copies of the same
fixtures. It normalizes only paths, timestamps, and random artifact IDs before
comparing observable results.

## Grammar additions

Grammar changes land in both implementations together. `awaiting_decision` is
an additive resting status: existing cases stay valid, but binaries built
before it reject it, so update every participating installation before any
case uses it.

`draft` without `--topic` inherits the one topic its `--responds-to` and
`--supersedes` targets share, falling back to the case ID as before. An
explicit `--topic`, or a draft with no references, behaves exactly as before.
Both implementations now resolve references before the topic, so when both
are invalid the reference error is the one reported. A binary built before
this change keeps using the case ID; drafts from mixed installations differ
only in filename, and every resulting artifact is valid.

## Publish tightening

Publishing refuses a body that still contains the `draft` skeleton's `# TITLE`
line, and removes a draft that `draft` generated once the artifact is durable.
Both implementations changed together. A binary built before this change still
accepts the placeholder and leaves drafts behind; neither affects existing
cases, which stay valid.

## Intentional difference

Python renders strings through `json.dumps`. For a non-BMP character such as
an emoji, that produces a UTF-16 surrogate pair which TOML rejects when the
rewritten manifest is parsed. The Python `cursor` command therefore exits
without changing `work.toml`.

Rust writes the Unicode scalar directly. Its `cursor` command succeeds and the
value round-trips through TOML. This is treated as a correctness improvement
rather than reproducing an invalid encoding.

Parser help text and operating-system errors use each implementation's native
formatting. Their command grammar, failing path, cause, exit class, failure
order, and filesystem effects remain compatible.

## Verification

Run the cross-language suite from the repository root:

```sh
AGENTS_WORK_PYTHON_REFERENCE="$PWD/python/agents_work.py" \
  cargo test --manifest-path rust/Cargo.toml --locked -- --ignored
```

The tests never read or mutate a configured live workspace.
