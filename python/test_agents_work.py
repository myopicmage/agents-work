from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest


MODULE_PATH = Path(__file__).with_name("agents_work.py")
SPEC = importlib.util.spec_from_file_location("agents_work", MODULE_PATH)
assert SPEC is not None
assert SPEC.loader is not None
agents_work = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agents_work)


def artifact_text(
    *,
    artifact_id: str = "a1b2c3",
    sequence: int = 1,
    kind: str = "plan",
    topic: str = "test-plan",
    author: str = "codex",
    responds_to: tuple[str, ...] = (),
) -> str:
    relationships = ", ".join(f'"{item}"' for item in responds_to)
    return f"""+++
artifact_schema_version = 1
artifact_id = "{artifact_id}"
sequence = {sequence}
kind = "{kind}"
topic = "{topic}"
author = "{author}"
created_at = 2026-07-27T19:00:00+09:00
responds_to = [{relationships}]
supersedes = []
source_branch = ""
source_commit = ""
source_path = ""
subject_repository = ""
subject_path = ""
subject_commit = ""
+++
# Test artifact

Body.
"""


def manifest_text(
    *,
    status: str = "drafting",
    next_agent: str = "codex",
    schema_version: int = 2,
) -> str:
    return f"""schema_version = {schema_version}
id = "test-case"
title = "Test case"
repository_name = "repo"
repository_path = "/tmp/repo"
phase = "planning"
status = "{status}"
next_agent = "{next_agent}"
requested_action = "Test."
implementation_branch = ""
pull_request_system = ""
pull_request_id = ""
reviewed_commit = ""
created_at = "2026-07-27T19:00:00+09:00"
updated_at = "2026-07-27T19:00:00+09:00"
"""


class AgentsWorkTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.case = self.root / "case"
        self.case.mkdir()
        (self.case / "work.toml").write_text(
            manifest_text(), encoding="utf-8"
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def prepare(self, text: str | None = None) -> Path:
        draft = self.root / "draft.md"
        draft.write_text(text or artifact_text(), encoding="utf-8")
        return draft

    def test_publish_then_validate(self) -> None:
        published = agents_work.publish(self.case, self.prepare())

        self.assertEqual(
            "001-test-plan-codex-a1b2c3.md",
            published.name,
        )
        self.assertTrue(agents_work.validate_case(self.case))

    def test_publish_is_no_clobber(self) -> None:
        draft = self.prepare()
        agents_work.publish(self.case, draft)

        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.publish(self.case, draft)

    def test_publish_reports_a_missing_sequence_with_a_draft_hint(self) -> None:
        text = artifact_text().replace("sequence = 1\n", "")

        with self.assertRaises(agents_work.ValidationFailure) as raised:
            agents_work.publish(self.case, self.prepare(text))

        message = str(raised.exception)
        self.assertIn("missing fields: sequence", message)
        self.assertIn("agents-work draft", message)

    def test_publish_reports_a_mistyped_sequence_with_a_draft_hint(self) -> None:
        text = artifact_text().replace("sequence = 1", 'sequence = "1"')

        with self.assertRaises(agents_work.ValidationFailure) as raised:
            agents_work.publish(self.case, self.prepare(text))

        message = str(raised.exception)
        self.assertIn("sequence must be a positive integer", message)
        self.assertIn("agents-work draft", message)

    def test_publish_reports_missing_front_matter_with_a_draft_hint(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure) as raised:
            agents_work.publish(self.case, self.prepare("# No front matter\n"))

        message = str(raised.exception)
        self.assertIn("missing TOML front matter", message)
        self.assertIn("agents-work draft", message)

    def test_publish_does_not_suggest_draft_for_a_missing_relationship(self) -> None:
        text = artifact_text(responds_to=("001-missing-codex-a1b2c3.md",))

        with self.assertRaises(agents_work.ValidationFailure) as raised:
            agents_work.publish(self.case, self.prepare(text))

        self.assertNotIn("agents-work draft", str(raised.exception))

    def test_validator_reports_orphan_sidecar(self) -> None:
        orphan = self.case / "001-test-plan-codex-a1b2c3.md.sha256"
        orphan.write_text("0" * 64 + "  artifact.md\n", encoding="ascii")

        self.assertFalse(agents_work.validate_case(self.case))

    def test_validator_reports_hash_mismatch(self) -> None:
        published = agents_work.publish(self.case, self.prepare())
        published.write_text("changed", encoding="utf-8")

        self.assertFalse(agents_work.validate_case(self.case))

    def test_validator_rejects_active_status_without_agent(self) -> None:
        (self.case / "work.toml").write_text(
            manifest_text(status="awaiting_review", next_agent=""),
            encoding="utf-8",
        )

        self.assertFalse(agents_work.validate_case(self.case))

    def test_draft_is_publishable_without_editing_front_matter(self) -> None:
        drafted = agents_work.draft(self.case, kind="review", author="claude")
        published = agents_work.publish(self.case, drafted)

        self.assertTrue(published.name.startswith("001-test-case-claude-"))
        self.assertTrue(agents_work.validate_case(self.case))

    def test_draft_sequence_follows_the_existing_inventory(self) -> None:
        agents_work.publish(self.case, self.prepare())

        drafted = agents_work.draft(self.case, kind="review", author="claude")

        self.assertTrue(drafted.name.startswith(".draft-002-"))

    def test_draft_resolves_a_bare_sequence_reference(self) -> None:
        published = agents_work.publish(self.case, self.prepare())

        drafted = agents_work.draft(
            self.case, kind="review", author="claude", responds_to=["1"]
        )

        metadata = agents_work.parse_front_matter(
            drafted.read_bytes(), drafted
        )
        self.assertEqual([published.name], metadata["responds_to"])

    def test_draft_rejects_a_reference_that_does_not_exist(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.draft(
                self.case, kind="review", author="claude", responds_to=["7"]
            )

    def test_an_unpublished_draft_is_not_discovered(self) -> None:
        agents_work.draft(self.case, kind="review", author="claude")

        self.assertTrue(agents_work.validate_case(self.case))

    def test_ready_for_implementation_may_name_the_resumption_owner(self) -> None:
        (self.case / "work.toml").write_text(
            manifest_text(
                status="ready_for_implementation",
                next_agent="codex",
            ),
            encoding="utf-8",
        )

        self.assertTrue(agents_work.validate_case(self.case))

    def test_repeated_relationship_flags_accumulate(self) -> None:
        args = agents_work.build_parser().parse_args(
            [
                "draft",
                str(self.case),
                "--kind",
                "review",
                "--author",
                "claude",
                "--responds-to",
                "026",
                "--responds-to",
                "027",
            ]
        )

        self.assertEqual(["026", "027"], args.responds_to)

    # ------------------------------------------------------------- cursor ---

    def read_manifest(self) -> dict:
        import tomllib

        return tomllib.loads(
            (self.case / "work.toml").read_text(encoding="utf-8")
        )

    def test_cursor_moves_the_coordination_fields(self) -> None:
        agents_work.cursor(
            self.case,
            {
                "status": "awaiting_review",
                "next_agent": "codex",
                "requested_action": "Review plan 030.",
            },
        )

        manifest = self.read_manifest()
        self.assertEqual("awaiting_review", manifest["status"])
        self.assertEqual("codex", manifest["next_agent"])
        self.assertEqual("Review plan 030.", manifest["requested_action"])
        self.assertTrue(agents_work.validate_case(self.case))

    def test_cursor_stamps_updated_at_and_nothing_else(self) -> None:
        before = self.read_manifest()

        agents_work.cursor(self.case, {"status": "revision_requested"})

        after = self.read_manifest()
        self.assertNotEqual(before["updated_at"], after["updated_at"])
        self.assertEqual(before["created_at"], after["created_at"])
        self.assertEqual(before["id"], after["id"])
        self.assertEqual(before["title"], after["title"])

    def test_cursor_rejects_an_unknown_status(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(self.case, {"status": "in_progress"})

    def test_cursor_rejects_active_status_without_agent(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(
                self.case,
                {"status": "awaiting_review", "next_agent": ""},
            )

    def test_cursor_clears_the_agent_for_a_resting_status(self) -> None:
        agents_work.cursor(self.case, {"status": "deferred"})

        self.assertEqual("", self.read_manifest()["next_agent"])
        self.assertTrue(agents_work.validate_case(self.case))

    def test_cursor_records_a_deferred_resumption_owner(self) -> None:
        agents_work.cursor(
            self.case,
            {"status": "deferred", "next_agent": "codex"},
        )

        manifest = self.read_manifest()
        self.assertEqual("deferred", manifest["status"])
        self.assertEqual("codex", manifest["next_agent"])
        self.assertTrue(agents_work.validate_case(self.case))

    def test_cursor_records_a_ready_resumption_owner(self) -> None:
        agents_work.cursor(
            self.case,
            {
                "status": "ready_for_implementation",
                "next_agent": "codex",
            },
        )

        manifest = self.read_manifest()
        self.assertEqual("ready_for_implementation", manifest["status"])
        self.assertEqual("codex", manifest["next_agent"])
        self.assertTrue(agents_work.validate_case(self.case))

    def test_cursor_rejects_a_complete_status_with_an_agent(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(
                self.case,
                {
                    "phase": "complete",
                    "status": "complete",
                    "next_agent": "codex",
                },
            )

    def test_cursor_repairs_an_illegal_manifest(self) -> None:
        # The incident this command exists for: a status outside the
        # vocabulary, written by hand. The cursor must be able to read past
        # the illegal value in order to replace it.
        (self.case / "work.toml").write_text(
            manifest_text(status="in_progress"), encoding="utf-8"
        )
        self.assertFalse(agents_work.validate_case(self.case))

        agents_work.cursor(self.case, {"status": "revision_requested"})

        self.assertTrue(agents_work.validate_case(self.case))

    def test_cursor_preserves_fields_it_does_not_know(self) -> None:
        with (self.case / "work.toml").open("a", encoding="utf-8") as handle:
            handle.write('custom_note = "keep me"\n')

        agents_work.cursor(self.case, {"status": "revision_requested"})

        self.assertEqual("keep me", self.read_manifest()["custom_note"])

    def test_cursor_refuses_identity_fields(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(self.case, {"id": "other-case"})

    def test_cursor_refuses_a_legacy_manifest(self) -> None:
        (self.case / "work.toml").write_text(
            manifest_text(schema_version=1), encoding="utf-8"
        )

        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(self.case, {"status": "drafting"})

    def test_cursor_requires_at_least_one_field(self) -> None:
        with self.assertRaises(agents_work.ValidationFailure):
            agents_work.cursor(self.case, {})


if __name__ == "__main__":
    unittest.main()
