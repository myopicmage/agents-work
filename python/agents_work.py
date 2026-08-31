#!/usr/bin/env python3

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import sys
import tempfile
import tomllib
from typing import Any


ARTIFACT_PATTERN = re.compile(
    r"^(?P<sequence>[0-9]{3})-[a-z0-9][a-z0-9-]*-"
    r"[a-z0-9][a-z0-9-]*-(?P<artifact_id>[0-9a-f]{6})[.]md$"
)
SLUG_PATTERN = re.compile(r"^[a-z0-9][a-z0-9-]*$")
ID_PATTERN = re.compile(r"^[0-9a-f]{6}$")
HASH_PATTERN = re.compile(r"^[0-9a-f]{64}$")

KINDS = frozenset({"plan", "review", "proposal", "decision", "response"})
PHASES = frozenset({"planning", "implementation", "pr_review", "complete"})
ACTIVE_STATUSES = frozenset(
    {"drafting", "awaiting_review", "revision_requested"}
)
RESTING_STATUSES = frozenset(
    {"ready_for_implementation", "deferred", "complete"}
)
STATUSES = ACTIVE_STATUSES | RESTING_STATUSES

REQUIRED_ARTIFACT_FIELDS = {
    "artifact_schema_version",
    "artifact_id",
    "sequence",
    "kind",
    "topic",
    "author",
    "created_at",
    "responds_to",
    "supersedes",
}
# Ordered, because `draft` renders front matter in this order and a set would
# emit the fields differently on every run.
OPTIONAL_FIELD_ORDER = (
    "source_branch",
    "source_commit",
    "source_path",
    "subject_repository",
    "subject_path",
    "subject_commit",
)
OPTIONAL_STRING_FIELDS = frozenset(OPTIONAL_FIELD_ORDER)


class ValidationFailure(Exception):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def parse_front_matter(data: bytes, source: Path) -> dict[str, Any]:
    opening = b"+++\n"
    closing = b"\n+++\n"

    if not data.startswith(opening):
        raise ValidationFailure(f"{source.name}: missing TOML front matter")

    end = data.find(closing, len(opening))
    if end < 0:
        raise ValidationFailure(f"{source.name}: unterminated TOML front matter")

    raw = data[len(opening) : end]
    try:
        parsed = tomllib.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ValidationFailure(
            f"{source.name}: invalid TOML front matter: {error}"
        ) from error

    return parsed


def artifact_field_errors(
    metadata: dict[str, Any], source: Path
) -> list[str]:
    errors: list[str] = []
    missing = REQUIRED_ARTIFACT_FIELDS - metadata.keys()
    if missing:
        errors.append(
            f"{source.name}: missing fields: {', '.join(sorted(missing))}"
        )
        return errors

    if metadata["artifact_schema_version"] != 1:
        errors.append(f"{source.name}: unsupported artifact schema")

    artifact_id = metadata["artifact_id"]
    if not isinstance(artifact_id, str) or not ID_PATTERN.fullmatch(artifact_id):
        errors.append(f"{source.name}: artifact_id must be 6 lowercase hex")

    sequence = metadata["sequence"]
    if (
        not isinstance(sequence, int)
        or isinstance(sequence, bool)
        or sequence < 1
    ):
        errors.append(f"{source.name}: sequence must be a positive integer")

    kind = metadata["kind"]
    if not isinstance(kind, str) or kind not in KINDS:
        errors.append(
            f"{source.name}: kind must be one of {', '.join(sorted(KINDS))}"
        )

    for field in ("topic", "author"):
        value = metadata[field]
        if not isinstance(value, str) or not SLUG_PATTERN.fullmatch(value):
            errors.append(f"{source.name}: {field} must be a lowercase slug")

    created_at = metadata["created_at"]
    if not isinstance(created_at, dt.datetime) or created_at.tzinfo is None:
        errors.append(f"{source.name}: created_at must include a UTC offset")

    for field in ("responds_to", "supersedes"):
        value = metadata[field]
        if not isinstance(value, list) or not all(
            isinstance(item, str)
            and item == Path(item).name
            and ARTIFACT_PATTERN.fullmatch(item)
            for item in value
        ):
            errors.append(
                f"{source.name}: {field} must contain case-relative artifact names"
            )

    for field in OPTIONAL_STRING_FIELDS:
        value = metadata.get(field, "")
        if not isinstance(value, str):
            errors.append(f"{source.name}: {field} must be a string")

    return errors


def validate_artifact_metadata(
    metadata: dict[str, Any], source: Path
) -> list[str]:
    errors = artifact_field_errors(metadata, source)

    if not errors:
        expected = artifact_filename(metadata)
        if source.name != expected:
            errors.append(
                f"{source.name}: filename does not match front matter: {expected}"
            )

    return errors


def artifact_filename(metadata: dict[str, Any]) -> str:
    return (
        f"{metadata['sequence']:03d}-{metadata['topic']}-"
        f"{metadata['author']}-{metadata['artifact_id']}.md"
    )


def discovered_artifacts(case: Path) -> list[Path]:
    """The case directory is authoritative for which artifacts exist."""
    return sorted(
        path
        for path in case.iterdir()
        if path.is_file() and ARTIFACT_PATTERN.fullmatch(path.name)
    )


def coordination_errors(manifest: dict[str, Any], label: str) -> list[str]:
    """The phase, status and next_agent rules, shared by validation and cursor.

    Stated once so the command that writes a cursor and the command that
    checks one cannot disagree about what a legal cursor is.
    """
    errors: list[str] = []

    phase = manifest.get("phase")
    if phase not in PHASES:
        errors.append(
            f"{label}: phase must be one of {', '.join(sorted(PHASES))}"
        )

    status = manifest.get("status")
    if status not in STATUSES:
        errors.append(
            f"{label}: status must be one of {', '.join(sorted(STATUSES))}"
        )

    next_agent = manifest.get("next_agent")
    if not isinstance(next_agent, str):
        errors.append(f"{label}: next_agent must be a string")
    elif status in ACTIVE_STATUSES and not next_agent:
        errors.append(f"{label}: active status requires next_agent")
    elif status == "complete" and next_agent:
        errors.append(f"{label}: complete status requires empty next_agent")

    if phase == "complete" and status != "complete":
        errors.append(f"{label}: complete phase requires complete status")
    if status == "complete" and phase != "complete":
        errors.append(f"{label}: complete status requires complete phase")

    return errors


def read_manifest(case: Path) -> tuple[dict[str, Any] | None, list[str]]:
    manifest_path = case / "work.toml"
    if not manifest_path.is_file():
        return None, [f"{manifest_path}: missing work.toml"]

    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        return None, [f"{manifest_path}: invalid work.toml: {error}"]

    errors: list[str] = []
    schema = manifest.get("schema_version")
    if schema not in {1, 2}:
        errors.append(f"{manifest_path}: schema_version must be 1 or 2")

    errors.extend(coordination_errors(manifest, str(manifest_path)))

    if schema == 2:
        legacy = {"latest_sequence", "latest_artifacts", "artifacts"}
        present = sorted(legacy & manifest.keys())
        if present:
            errors.append(
                f"{manifest_path}: schema 2 contains legacy fields: "
                f"{', '.join(present)}"
            )

    return manifest, errors


def validate_case(case: Path) -> bool:
    case = case.expanduser().resolve()
    errors: list[str] = []
    infos: list[str] = []
    inventory: list[tuple[Path, dict[str, Any] | None, str | None]] = []

    manifest, manifest_errors = read_manifest(case)
    errors.extend(manifest_errors)

    discovered = discovered_artifacts(case)
    discovered_names = {path.name for path in discovered}

    sidecars = sorted(case.glob("*.md.sha256"))
    for sidecar in sidecars:
        artifact = case / sidecar.name.removesuffix(".sha256")
        if not artifact.is_file():
            errors.append(f"{sidecar.name}: orphan sidecar")

    metadata_by_name: dict[str, dict[str, Any]] = {}
    hash_by_name: dict[str, str] = {}

    for artifact in discovered:
        data = artifact.read_bytes()
        digest = sha256_bytes(data)
        hash_by_name[artifact.name] = digest
        metadata: dict[str, Any] | None = None

        try:
            metadata = parse_front_matter(data, artifact)
            errors.extend(validate_artifact_metadata(metadata, artifact))
            metadata_by_name[artifact.name] = metadata
        except ValidationFailure as error:
            errors.append(str(error))

        sidecar = artifact.with_name(f"{artifact.name}.sha256")
        if not sidecar.is_file():
            errors.append(f"{artifact.name}: missing sidecar")
        else:
            expected = f"{digest}  {artifact.name}\n"
            try:
                actual = sidecar.read_text(encoding="ascii")
            except (OSError, UnicodeDecodeError) as error:
                errors.append(f"{sidecar.name}: unreadable sidecar: {error}")
            else:
                if actual != expected:
                    errors.append(f"{sidecar.name}: hash or format mismatch")

        inventory.append((artifact, metadata, digest))

    for name, metadata in metadata_by_name.items():
        for relationship in ("responds_to", "supersedes"):
            for target in metadata.get(relationship, []):
                if target not in discovered_names:
                    errors.append(f"{name}: {relationship} target missing: {target}")

    if manifest is not None and manifest.get("schema_version") == 1:
        records = manifest.get("artifacts", [])
        if not isinstance(records, list):
            errors.append(f"{case / 'work.toml'}: artifacts must be an array")
            records = []

        listed: set[str] = set()
        for record in records:
            if not isinstance(record, dict):
                errors.append(f"{case / 'work.toml'}: malformed artifact record")
                continue

            path = record.get("path")
            digest = record.get("sha256")
            if not isinstance(path, str) or path != Path(path).name:
                errors.append(f"{case / 'work.toml'}: invalid legacy artifact path")
                continue

            listed.add(path)
            if path not in discovered_names:
                errors.append(f"{case / 'work.toml'}: listed artifact missing: {path}")
            elif digest != hash_by_name.get(path):
                errors.append(f"{case / 'work.toml'}: legacy hash mismatch: {path}")

        for name in sorted(discovered_names - listed):
            infos.append(f"discovered artifact absent from legacy manifest: {name}")

    print(f"case: {case}")
    print("inventory:")
    if not inventory:
        print("  (none)")
    for artifact, metadata, digest in inventory:
        if metadata is None:
            print(f"  {artifact.name}  INVALID  {digest}")
        else:
            print(
                f"  {artifact.name}  {metadata.get('kind', '?')}  "
                f"{metadata.get('author', '?')}  {digest}"
            )

    for info in infos:
        print(f"info: {info}")
    for error in errors:
        print(f"error: {error}", file=sys.stderr)

    if errors:
        print(f"result: invalid ({len(errors)} errors)")
        return False

    print(f"result: valid ({len(inventory)} artifacts)")
    return True


def draft_hint(case: Path, metadata: dict[str, Any] | None = None) -> str:
    kind = metadata.get("kind") if metadata is not None else None
    author = metadata.get("author") if metadata is not None else None

    kind_argument = kind if isinstance(kind, str) and kind in KINDS else "KIND"
    author_argument = (
        author
        if isinstance(author, str) and SLUG_PATTERN.fullmatch(author)
        else "AUTHOR"
    )
    return (
        f"hint: agents-work draft {shlex.quote(str(case))} --kind {kind_argument} "
        f"--author {author_argument} generates valid front matter"
    )


def validate_prepared_artifact(draft: Path, case: Path) -> dict[str, Any]:
    data = draft.read_bytes()
    try:
        metadata = parse_front_matter(data, draft)
    except ValidationFailure as error:
        raise ValidationFailure(f"{error}\n{draft_hint(case)}") from error

    errors = artifact_field_errors(metadata, draft)
    if errors:
        errors.append(draft_hint(case, metadata))
        raise ValidationFailure("\n".join(errors))

    discovered = {path.name for path in discovered_artifacts(case)}
    for relationship in ("responds_to", "supersedes"):
        for target in metadata.get(relationship, []):
            if target not in discovered:
                errors.append(f"{draft.name}: {relationship} target missing: {target}")

    if errors:
        raise ValidationFailure("\n".join(errors))

    return metadata


def next_sequence(case: Path) -> int:
    highest = 0
    for path in discovered_artifacts(case):
        match = ARTIFACT_PATTERN.fullmatch(path.name)
        if match:
            highest = max(highest, int(match.group("sequence")))

    return highest + 1


def unused_artifact_id(case: Path) -> str:
    taken = set()
    for path in discovered_artifacts(case):
        match = ARTIFACT_PATTERN.fullmatch(path.name)
        if match:
            taken.add(match.group("artifact_id"))

    while True:
        candidate = secrets.token_hex(3)
        if candidate not in taken:
            return candidate


def resolve_reference(reference: str, case: Path) -> str:
    """Accept a full artifact filename or a bare sequence number.

    Sequence numbers are a convenience only. They are ambiguous by design,
    because concurrent writers may legally share one, so an ambiguous
    reference is an error rather than a guess.
    """
    names = [path.name for path in discovered_artifacts(case)]
    if reference in names:
        return reference

    if not reference.isdigit():
        raise ValidationFailure(f"unknown artifact reference: {reference}")

    wanted = int(reference)
    matches = [name for name in names if int(name[:3]) == wanted]

    if not matches:
        raise ValidationFailure(f"no artifact with sequence {wanted}")

    if len(matches) > 1:
        raise ValidationFailure(
            f"sequence {wanted} is ambiguous, name one of: "
            f"{', '.join(matches)}"
        )

    return matches[0]


def render_front_matter(metadata: dict[str, Any]) -> str:
    def array(items: list[str]) -> str:
        if not items:
            return "[]"

        body = "".join(f'  "{item}",\n' for item in items)
        return f"[\n{body}]"

    created_at = metadata["created_at"]
    lines = [
        "+++",
        "artifact_schema_version = 1",
        f'artifact_id = "{metadata["artifact_id"]}"',
        f"sequence = {metadata['sequence']}",
        f'kind = "{metadata["kind"]}"',
        f'topic = "{metadata["topic"]}"',
        f'author = "{metadata["author"]}"',
        f"created_at = {created_at.isoformat()}",
        f"responds_to = {array(metadata['responds_to'])}",
        f"supersedes = {array(metadata['supersedes'])}",
    ]
    lines.extend(f'{field} = ""' for field in OPTIONAL_FIELD_ORDER)
    lines.append("+++")

    return "\n".join(lines) + "\n"


def draft(
    case: Path,
    *,
    kind: str,
    author: str,
    topic: str | None = None,
    responds_to: list[str] | None = None,
    supersedes: list[str] | None = None,
    output: Path | None = None,
) -> Path:
    case = case.expanduser().resolve()

    manifest, _ = read_manifest(case)
    if manifest is None:
        raise ValidationFailure(f"{case}: missing or unreadable work.toml")

    resolved_topic = topic if topic is not None else manifest.get("id")
    if not isinstance(resolved_topic, str) or not SLUG_PATTERN.fullmatch(
        resolved_topic
    ):
        raise ValidationFailure(
            "topic must be a lowercase slug; work.toml has no usable id, "
            "so pass --topic"
        )

    metadata: dict[str, Any] = {
        "artifact_schema_version": 1,
        "artifact_id": unused_artifact_id(case),
        "sequence": next_sequence(case),
        "kind": kind,
        "topic": resolved_topic,
        "author": author,
        "created_at": dt.datetime.now().astimezone().replace(microsecond=0),
        "responds_to": [
            resolve_reference(reference, case)
            for reference in responds_to or []
        ],
        "supersedes": [
            resolve_reference(reference, case) for reference in supersedes or []
        ],
    }
    metadata.update({field: "" for field in OPTIONAL_FIELD_ORDER})

    filename = artifact_filename(metadata)
    draft_path = (
        output.expanduser().resolve()
        if output is not None
        else case / f".draft-{filename}"
    )

    # Prove the skeleton is publishable before writing it, so a draft can only
    # fail on what the author adds rather than on what the tool generated.
    errors = validate_artifact_metadata(metadata, draft_path.with_name(filename))
    if errors:
        raise ValidationFailure("\n".join(errors))

    body = f"{render_front_matter(metadata)}\n# TITLE\n"
    descriptor = os.open(
        draft_path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL,
        0o644,
    )
    with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
        handle.write(body)

    print(draft_path)
    return draft_path


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def publish(case: Path, draft: Path) -> Path:
    case = case.expanduser().resolve()
    draft = draft.expanduser().resolve()

    if not (case / "work.toml").is_file():
        raise ValidationFailure(f"{case}: missing work.toml")
    if not draft.is_file():
        raise ValidationFailure(f"{draft}: draft does not exist")

    metadata = validate_prepared_artifact(draft, case)
    filename = artifact_filename(metadata)
    final_path = case / filename
    sidecar_path = case / f"{filename}.sha256"

    if final_path.exists() or sidecar_path.exists():
        raise ValidationFailure(
            f"{filename}: final artifact or sidecar already exists; use a new ID"
        )

    data = draft.read_bytes()
    digest = sha256_bytes(data)
    sidecar_data = f"{digest}  {filename}\n".encode("ascii")

    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".artifact-", suffix=".tmp", dir=case
    )
    temporary_path = Path(temporary_name)

    try:
        with os.fdopen(descriptor, "wb") as temporary:
            os.fchmod(temporary.fileno(), 0o644)
            temporary.write(data)
            temporary.flush()
            os.fsync(temporary.fileno())

        sidecar_descriptor = os.open(
            sidecar_path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL,
            0o644,
        )
        try:
            with os.fdopen(sidecar_descriptor, "wb") as sidecar:
                sidecar.write(sidecar_data)
                sidecar.flush()
                os.fsync(sidecar.fileno())

            try:
                os.link(temporary_path, final_path)
            except OSError:
                sidecar_path.unlink(missing_ok=True)
                raise

            fsync_directory(case)
        except BaseException:
            if not final_path.exists():
                sidecar_path.unlink(missing_ok=True)
            raise
    finally:
        temporary_path.unlink(missing_ok=True)

    print(final_path)
    return final_path


# The manifest is rendered in this order, grouped as the files are grouped in
# practice: identity, repository, coordination, pointers, timestamps. A blank
# line separates groups. Ordered for the same reason OPTIONAL_FIELD_ORDER is:
# a set would emit the fields differently on every run.
MANIFEST_FIELD_GROUPS: tuple[tuple[str, ...], ...] = (
    ("schema_version", "id", "title"),
    ("repository_name", "repository_path"),
    ("phase", "status", "next_agent", "requested_action"),
    (
        "implementation_branch",
        "pull_request_system",
        "pull_request_id",
        "reviewed_commit",
    ),
    ("created_at", "updated_at"),
)

# The fields the cursor command may write. Everything else in the manifest is
# identity, which the command has no flag for and therefore cannot touch.
CURSOR_FIELDS = frozenset(
    {
        "phase",
        "status",
        "next_agent",
        "requested_action",
        "implementation_branch",
        "pull_request_system",
        "pull_request_id",
        "reviewed_commit",
    }
)


def render_toml_value(field: str, value: Any) -> str:
    # bool first, because bool is a subclass of int and would render as 0/1.
    if isinstance(value, bool):
        return "true" if value else "false"

    if isinstance(value, int):
        return str(value)

    if isinstance(value, str):
        # JSON string escaping is valid TOML basic-string escaping for
        # everything json.dumps emits, so the quoting cannot be hand-rolled
        # wrong.
        return json.dumps(value)

    raise ValidationFailure(
        f"cannot render manifest field {field} of type {type(value).__name__}"
    )


def render_manifest(manifest: dict[str, Any]) -> str:
    known = [field for group in MANIFEST_FIELD_GROUPS for field in group]
    groups: list[list[str]] = []

    for group in MANIFEST_FIELD_GROUPS:
        lines = [
            f"{field} = {render_toml_value(field, manifest[field])}"
            for field in group
            if field in manifest
        ]
        if lines:
            groups.append(lines)

    # Fields this tool does not know about are preserved rather than dropped,
    # so a forward-compatible cursor edit cannot silently discard somebody
    # else's state.
    unknown = sorted(manifest.keys() - set(known))
    if unknown:
        groups.append(
            [
                f"{field} = {render_toml_value(field, manifest[field])}"
                for field in unknown
            ]
        )

    return "\n\n".join("\n".join(group) for group in groups) + "\n"


def cursor(case: Path, updates: dict[str, str]) -> Path:
    """Move the coordination cursor, refusing to write an illegal one.

    Reads the manifest raw rather than through validation, because the one
    time this command is most needed is when the current cursor is already
    illegal and hand-editing is the only other way out.
    """
    case = case.expanduser().resolve()
    manifest_path = case / "work.toml"

    if not manifest_path.is_file():
        raise ValidationFailure(f"{case}: missing work.toml")

    unexpected = sorted(updates.keys() - CURSOR_FIELDS)
    if unexpected:
        raise ValidationFailure(
            f"cursor cannot write: {', '.join(unexpected)}"
        )

    if not updates:
        raise ValidationFailure("cursor requires at least one field to set")

    try:
        manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ValidationFailure(
            f"{manifest_path}: invalid work.toml: {error}"
        ) from error

    if manifest.get("schema_version") != 2:
        raise ValidationFailure(
            f"{manifest_path}: cursor requires schema_version 2; "
            "migrate a legacy manifest by hand first"
        )

    merged = dict(manifest)
    merged.update(updates)

    # A resting status is a gate, not permission to act. Moving to one without
    # an explicit owner clears stale ownership. ready_for_implementation and
    # deferred may keep an explicitly named resumption owner; complete may not.
    status = merged.get("status")
    if status in RESTING_STATUSES and "next_agent" not in updates:
        merged["next_agent"] = ""

    merged["updated_at"] = (
        dt.datetime.now().astimezone().replace(microsecond=0).isoformat()
    )

    errors = coordination_errors(merged, str(manifest_path))
    if errors:
        raise ValidationFailure("\n".join(errors))

    rendered = render_manifest(merged)

    # Prove the rendering reads back as what was written, the same discipline
    # `draft` applies to front matter: the tool's own output cannot be the
    # thing that fails validation later.
    if tomllib.loads(rendered) != merged:
        raise ValidationFailure(
            f"{manifest_path}: rendered manifest does not round-trip"
        )

    descriptor, temporary_name = tempfile.mkstemp(
        prefix=".work-", suffix=".tmp", dir=case
    )
    temporary_path = Path(temporary_name)

    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as temporary:
            os.fchmod(temporary.fileno(), 0o644)
            temporary.write(rendered)
            temporary.flush()
            os.fsync(temporary.fileno())

        # Replace rather than no-clobber, deliberately: the manifest is the
        # one mutable file in a case, and atomicity is what protects a
        # concurrent reader from a half-written cursor.
        os.replace(temporary_path, manifest_path)
        fsync_directory(case)
    finally:
        temporary_path.unlink(missing_ok=True)

    agent = merged.get("next_agent") or "(none)"
    print(
        f"{manifest_path}: phase={merged.get('phase')} "
        f"status={merged.get('status')} next_agent={agent}"
    )
    return manifest_path


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="agents-work",
        description="Validate and publish shared agent-work artifacts.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("case", nargs="+", type=Path)

    draft_parser = subparsers.add_parser(
        "draft",
        help="Write a publishable skeleton with front matter already filled in",
    )
    draft_parser.add_argument("case", type=Path)
    draft_parser.add_argument("--kind", required=True, choices=sorted(KINDS))
    draft_parser.add_argument("--author", required=True)
    draft_parser.add_argument(
        "--topic", help="defaults to the work.toml id"
    )
    # `extend`, so a repeated flag accumulates. The default action with
    # `nargs="*"` silently keeps only the last occurrence, which discards a
    # relationship the author explicitly named.
    draft_parser.add_argument(
        "--responds-to",
        nargs="*",
        action="extend",
        default=[],
        metavar="REF",
        help="artifact filename or bare sequence number; may be repeated",
    )
    draft_parser.add_argument(
        "--supersedes",
        nargs="*",
        action="extend",
        default=[],
        metavar="REF",
    )
    draft_parser.add_argument(
        "--output",
        type=Path,
        help="defaults to a hidden draft beside the case",
    )

    publish_parser = subparsers.add_parser("publish")
    publish_parser.add_argument("case", type=Path)
    publish_parser.add_argument("draft", type=Path)

    cursor_parser = subparsers.add_parser(
        "cursor",
        help="Move work.toml's coordination fields, refusing an illegal cursor",
    )
    cursor_parser.add_argument("case", type=Path)
    cursor_parser.add_argument("--phase", choices=sorted(PHASES))
    cursor_parser.add_argument("--status", choices=sorted(STATUSES))
    cursor_parser.add_argument(
        "--next-agent",
        dest="next_agent",
        help='the agent expected to act next; "" to clear',
    )
    cursor_parser.add_argument(
        "--action",
        dest="requested_action",
        help="what the next agent is being asked to do",
    )
    cursor_parser.add_argument(
        "--implementation-branch", dest="implementation_branch"
    )
    cursor_parser.add_argument("--pr-system", dest="pull_request_system")
    cursor_parser.add_argument("--pr-id", dest="pull_request_id")
    cursor_parser.add_argument("--reviewed-commit", dest="reviewed_commit")

    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    try:
        if args.command == "validate":
            results = [validate_case(case) for case in args.case]
            return 0 if all(results) else 1

        if args.command == "draft":
            draft(
                args.case,
                kind=args.kind,
                author=args.author,
                topic=args.topic,
                responds_to=args.responds_to,
                supersedes=args.supersedes,
                output=args.output,
            )
            return 0

        if args.command == "publish":
            publish(args.case, args.draft)
            return 0

        if args.command == "cursor":
            updates = {
                field: value
                for field, value in (
                    ("phase", args.phase),
                    ("status", args.status),
                    ("next_agent", args.next_agent),
                    ("requested_action", args.requested_action),
                    ("implementation_branch", args.implementation_branch),
                    ("pull_request_system", args.pull_request_system),
                    ("pull_request_id", args.pull_request_id),
                    ("reviewed_commit", args.reviewed_commit),
                )
                if value is not None
            }
            cursor(args.case, updates)
            return 0
    except (OSError, ValidationFailure) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
