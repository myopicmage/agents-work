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
directories. Install one implementation at a time. The repository also ships
one portable skill for Codex and Claude Code.

The supported operating systems are macOS and Linux. See
[COMPATIBILITY.md](COMPATIBILITY.md) for the parity contract and one documented
Unicode edge-case difference.

## Install

Run the interactive installer:

```sh
./install.sh
```

It chooses:

- the Python or Rust implementation;
- Codex, Claude Code, both agents, or neither;
- binary, configuration, workspace data, installer state, and skill
  locations.

Python is the default when Python 3.11 or newer is available. Otherwise the
installer selects Rust when Cargo works. It detects installed agents and uses
their native personal skill locations:

- Codex: `${CODEX_HOME:-$HOME/.codex}/skills/agents-work`;
- Claude Code: `$HOME/.claude/skills/agents-work`.

The installer does not set or change either agent's model configuration.

For a non-interactive installation:

```sh
./install.sh \
  --non-interactive \
  --implementation python \
  --agent codex \
  --agent claude
```

Use `--implementation rust` to build and install the native binary. Run
`./install.sh --help` for every location override.

### XDG defaults

The installer respects XDG base-directory variables:

| Purpose | Default |
| --- | --- |
| Configuration | `${XDG_CONFIG_HOME:-$HOME/.config}/agents-work` |
| Workspace data | `${XDG_DATA_HOME:-$HOME/.local/share}/agents-work` |
| Installer state | `${XDG_STATE_HOME:-$HOME/.local/state}/agents-work` |
| Commands | `${XDG_BIN_HOME:-$HOME/.local/bin}` |

`XDG_BIN_HOME` is supported as a common extension because the XDG base
directory specification does not define a binary directory.

The generated `config.toml` records the selected implementation, command,
workspace, protocol, and agents. Each installed skill receives a small
`installation.toml` pointing to that configuration, so custom locations stay
discoverable.

## Uninstall

The installer adds an `agents-work-uninstall` command:

```sh
agents-work-uninstall
```

Ordinary uninstall removes only files recorded in the installation manifest.
It preserves the entire collaboration workspace. If an installer-owned file
has been modified, uninstall makes no changes until `--force` is explicitly
provided.

Deleting collaboration data is a separate, confirmed operation:

```sh
agents-work-uninstall --purge-data
```

The repository copy also works directly:

```sh
./uninstall.sh
```

The manual methods below install only the CLI. Use `./install.sh` for
configuration, workspace setup, skills, ownership tracking, and reversible
uninstall.

## Manual Python installation

Requirements: Python 3.11 or newer.

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 python/agents_work.py "$HOME/.local/bin/agents-work"
agents-work --help
```

Ensure `$HOME/.local/bin` is on `PATH`. The installed script has no
third-party dependencies.

To update it after pulling a newer version, repeat the `install` command.

## Manual Rust installation

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

## Tool-only installation with Nix

The flake exposes both implementations. These commands install only the CLI,
without configuration or agent skills:

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

## Workspace layout

The installer creates the configured workspace. Its XDG default is
`${XDG_DATA_HOME:-$HOME/.local/share}/agents-work`.

Each collaboration gets a case directory beneath a repository name:

```text
<workspace>/
└── example-project/
    └── auth-redesign/
        └── work.toml
```

Start `work.toml` from [examples/work.toml](examples/work.toml), replacing
the repository path and coordination fields with real values.

The complete data model, lifecycle, concurrency rules, and safety boundaries
are in [PROTOCOL.md](PROTOCOL.md).

## Basic workflow

Use the configured workspace. With the default installation:

```sh
workspace="${XDG_DATA_HOME:-$HOME/.local/share}/agents-work"
```

Validate a case before reading it:

```sh
agents-work validate "$workspace/example-project/auth-redesign"
```

Create and publish an artifact:

```sh
draft_path="$(agents-work draft \
  "$workspace/example-project/auth-redesign" \
  --kind plan \
  --author planner)"

"$EDITOR" "$draft_path"
agents-work publish "$workspace/example-project/auth-redesign" \
  "$draft_path"
```

Move the coordination cursor:

```sh
agents-work cursor "$workspace/example-project/auth-redesign" \
  --status awaiting_review \
  --next-agent reviewer \
  --action "Review the proposed authentication design."
```

The draft filename includes a random ID. Capturing the printed path avoids
reconstructing it.

## Agent skill

The portable [skill](skills/agents-work/SKILL.md) teaches both Codex and Claude
Code to locate the configured workspace, validate before reading, follow
artifact causality, read `work.toml` last, and preserve the human authorization
boundary.

The installer copies the same `SKILL.md` into each selected agent's personal
skill directory. Agents that do not support skills can use
[PROTOCOL.md](PROTOCOL.md) directly.

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
