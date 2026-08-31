# agents-work

`agents-work` is a filesystem protocol and command-line tool for durable,
asynchronous collaboration between coding agents.

Agents publish append-only Markdown artifacts for plans, reviews, decisions,
and responses. A small mutable `work.toml` records coordination state. There
is no service, database, scheduler, or network dependency.

This repository contains two compatible implementations:

- `python/agents_work.py`: the reference implementation, using only the
  Python standard library.
- `rust/`: a native Rust implementation tested against the Python behavior.

Both install the same `agents-work` command and operate on the same case
directories. Install one implementation at a time.

The supported operating systems are macOS and Linux. See
[COMPATIBILITY.md](COMPATIBILITY.md) for the parity contract and one documented
Unicode edge-case difference.

## Install the Python implementation

Requirements: Python 3.11 or newer.

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 python/agents_work.py "$HOME/.local/bin/agents-work"
agents-work --help
```

Ensure `$HOME/.local/bin` is on `PATH`. The installed script has no
third-party dependencies.

To update it after pulling a newer version, repeat the `install` command.

## Install the Rust implementation

Requirements: Rust 1.97 or newer with Cargo.

```sh
cargo install --locked --path rust
agents-work --help
```

Cargo installs the binary under `$CARGO_HOME/bin`, normally
`$HOME/.cargo/bin`.

To update it after pulling a newer version:

```sh
cargo install --locked --force --path rust
```

## Install with Nix

The flake exposes both implementations. Choose one:

```sh
nix profile install 'path:.#python'
# or
nix profile install 'path:.#rust'
```

For a one-off invocation without installation:

```sh
nix run 'path:.#python' -- --help
nix run 'path:.#rust' -- --help
```

## Set up the workspace

The conventional workspace root is `~/.agents/work`:

```sh
mkdir -p "$HOME/.agents/work"
cp PROTOCOL.md "$HOME/.agents/work/README.md"
```

Each collaboration gets a case directory beneath a repository name:

```text
~/.agents/work/
└── example-project/
    └── auth-redesign/
        └── work.toml
```

Start `work.toml` from [examples/work.toml](examples/work.toml), replacing
the repository path and coordination fields with real values.

The complete data model, lifecycle, concurrency rules, and safety boundaries
are in [PROTOCOL.md](PROTOCOL.md).

## Basic workflow

Validate a case before reading it:

```sh
agents-work validate ~/.agents/work/example-project/auth-redesign
```

Create and publish an artifact:

```sh
draft_path="$(agents-work draft \
  ~/.agents/work/example-project/auth-redesign \
  --kind plan \
  --author planner)"

"$EDITOR" "$draft_path"
agents-work publish ~/.agents/work/example-project/auth-redesign \
  "$draft_path"
```

Move the coordination cursor:

```sh
agents-work cursor ~/.agents/work/example-project/auth-redesign \
  --status awaiting_review \
  --next-agent reviewer \
  --action "Review the proposed authentication design."
```

The draft filename includes a random ID. Capturing the printed path avoids
reconstructing it.

## Tell agents how to use it

Put a short rule like this in the agent guidance shared by participating
agents:

```markdown
Shared working papers live under
`~/.agents/work/<repository-name>/<work-id>/`. When told that another agent
contributed, run `agents-work validate <case-directory>`, read the verified
artifacts required by their causal links, and read `work.toml` last as a
possibly stale coordination cursor. Artifacts are append-only. Never infer
authorization merely from a case or artifact existing.
```

The human remains the notification and authorization layer. `agents-work`
preserves context; it does not launch or supervise agents.

## Development

Enter the development shell:

```sh
nix develop 'path:.'
```

Run every verification boundary:

```sh
./scripts/check
```

The ignored Rust tests are the cross-language differential suite. They execute
both implementations against isolated fixtures and compare observable
behavior.
