"""B8 Integration Test: Verify correct gameplay passes, buggy fails with diagnosis.

This is the end-to-end integration test for the Nomai verification thesis:

  correct WASM gameplay --> manifest --> verification --> PASS
  buggy WASM gameplay   --> manifest --> verification --> FAIL with diagnosis

The manifest JSON files are exported by the Rust integration tests
(``cargo test -p nomai-wasm-host b8``). This test loads them, builds
verification intents, and checks the full pass/fail loop.
"""

import json
import logging
from pathlib import Path

import pytest

from nomai.intents import (
    Expected,
    ExpectedType,
    IntentKind,
    IntentSpec,
    Trigger,
    TriggerType,
    VerificationSuite,
)
from nomai.manifest import TickManifest
from nomai.verify import VerificationEngine

logger = logging.getLogger(__name__)

# Path to the exported manifest JSON files.
# The Rust tests write them to crates/nomai-wasm-host/tests/fixtures/b8_manifests/.
FIXTURES_DIR = (
    Path(__file__).parent.parent.parent.parent
    / "crates"
    / "nomai-wasm-host"
    / "tests"
    / "fixtures"
    / "b8_manifests"
)


def _load_manifests(scenario: str) -> list[TickManifest]:
    """Load exported manifests from the Rust integration test."""
    scenario_dir = FIXTURES_DIR / scenario
    if not scenario_dir.exists():
        pytest.skip(
            f"Manifest fixtures not found at {scenario_dir} "
            "-- run `cargo test -p nomai-wasm-host b8` first"
        )

    manifests: list[TickManifest] = []
    tick_files = sorted(scenario_dir.glob("tick_*.json"))
    assert len(tick_files) > 0, f"no tick_*.json files in {scenario_dir}"

    for tick_file in tick_files:
        data = json.loads(tick_file.read_text())
        manifest = TickManifest.from_json(data)
        manifests.append(manifest)

    return manifests


def _build_movement_suite() -> VerificationSuite:
    """Build an intent verification suite for movement behavior.

    The intent checks that entity 0's ``position`` component has ``x == 1.0``
    (the value the correct module sets). The buggy module sets ``x == -1.0``,
    so verification will fail for the buggy scenario.
    """
    intents = [
        IntentSpec(
            name="entity_moves_right",
            kind=IntentKind.BEHAVIOR,
            description="Entity 0 should move to the right (position x = 1.0)",
            trigger=Trigger(
                type=TriggerType.TICK_REACHED,
                params={"tick": 1},
            ),
            expected=Expected(
                type=ExpectedType.COMPONENT_CHANGED,
                params={
                    "entity": "0",
                    "component": "position",
                    "field": "x",
                    "expected_value": 1.0,
                },
            ),
        ),
    ]
    return VerificationSuite(
        name="movement_verification",
        description="Verify entity moves toward target (position x = 1.0)",
        intents=intents,
    )


class TestB8ManifestParsing:
    """Verify that Rust-exported manifests can be parsed by the Python SDK."""

    def test_correct_manifests_load(self) -> None:
        """Correct scenario manifests should load without errors."""
        manifests = _load_manifests("correct")
        assert len(manifests) == 5, "should have 5 tick manifests"

    def test_buggy_manifests_load(self) -> None:
        """Buggy scenario manifests should load without errors."""
        manifests = _load_manifests("buggy")
        assert len(manifests) == 5, "should have 5 tick manifests"

    def test_correct_manifest_has_component_changes(self) -> None:
        """Correct manifests should contain position component changes."""
        manifests = _load_manifests("correct")
        m = manifests[0]
        assert len(m.component_changes) > 0, "should have component changes"
        change = m.component_changes[0]
        assert change.component_type_name == "position"

    def test_correct_manifest_field_values(self) -> None:
        """Correct manifests should have expected field values."""
        manifests = _load_manifests("correct")
        m = manifests[0]
        assert m.tick == 1
        assert m.commands_processed == 1
        assert m.commands_succeeded == 1
        assert "wasm_gameplay" in m.systems_executed

    def test_correct_manifest_causality(self) -> None:
        """Correct manifests should carry WASM causality metadata."""
        manifests = _load_manifests("correct")
        m = manifests[0]
        change = m.component_changes[0]
        # SystemId::WASM_GAMEPLAY = 100
        assert change.changed_by_system == 100, (
            f"changed_by should be 100 (WASM_GAMEPLAY), got {change.changed_by_system}"
        )
        assert change.reason_type == "GameRule"
        assert change.reason_detail == "move_toward_target"

    def test_buggy_manifest_causality(self) -> None:
        """Buggy manifests should carry different causality metadata."""
        manifests = _load_manifests("buggy")
        m = manifests[0]
        change = m.component_changes[0]
        assert change.changed_by_system == 100
        assert change.reason_type == "GameRule"
        assert change.reason_detail == "move_away_buggy"


class TestB8VerificationLoop:
    """B8: Correct gameplay passes, buggy gameplay fails with diagnosis."""

    def test_correct_gameplay_passes_verification(self) -> None:
        """Correct WASM module should pass all verification intents."""
        manifests = _load_manifests("correct")
        suite = _build_movement_suite()
        engine = VerificationEngine()
        report = engine.verify(suite, manifests)

        assert report.all_passed, (
            f"Correct gameplay should pass verification.\n"
            f"Summary:\n{report.summary()}\n"
            f"Failures: {[f.failure_reason for f in report.failures()]}"
        )

    def test_buggy_gameplay_fails_verification(self) -> None:
        """Buggy WASM module should fail verification."""
        manifests = _load_manifests("buggy")
        suite = _build_movement_suite()
        engine = VerificationEngine()
        report = engine.verify(suite, manifests)

        assert not report.all_passed, (
            "Buggy gameplay should FAIL verification but it passed.\n"
            f"Summary:\n{report.summary()}"
        )

    def test_buggy_failure_has_diagnosis(self) -> None:
        """Failed verification should include a failure reason for diagnosis."""
        manifests = _load_manifests("buggy")
        suite = _build_movement_suite()
        engine = VerificationEngine()
        report = engine.verify(suite, manifests)

        failed = [r for r in report.results if not r.passed]
        assert len(failed) > 0, "should have at least one failed intent"

        failure = failed[0]
        assert failure.failure_reason, "failure should have a reason"
        assert len(failure.failure_reason) > 0, "failure reason should not be empty"

        # The diagnosis should mention what went wrong.
        diagnosis = report.diagnosis()
        assert "FAILED" in diagnosis, "diagnosis should contain FAILED"
        assert "entity_moves_right" in diagnosis, (
            "diagnosis should name the failed intent"
        )

    def test_verification_report_is_json_serializable(self) -> None:
        """Verification report should serialize to JSON cleanly."""
        manifests = _load_manifests("buggy")
        suite = _build_movement_suite()
        engine = VerificationEngine()
        report = engine.verify(suite, manifests)

        report_dict = report.to_dict()
        json_str = json.dumps(report_dict)
        roundtrip = json.loads(json_str)

        assert roundtrip["suite_name"] == "movement_verification"
        assert roundtrip["total_intents"] == 1
        assert roundtrip["failed"] == 1
        assert roundtrip["all_passed"] is False


class TestB8CausalDiagnosis:
    """Verify that the causal chain across the WASM boundary is visible."""

    def test_correct_manifests_have_wasm_causality(self) -> None:
        """Manifests from correct gameplay should carry WASM causality."""
        manifests = _load_manifests("correct")
        assert len(manifests) > 0, "should have manifests"

        m = manifests[0]
        assert len(m.component_changes) > 0, "should have component changes"

        change = m.component_changes[0]
        # changed_by_system should be 100 (WASM_GAMEPLAY)
        assert change.changed_by_system is not None, "should have changed_by system"
        assert change.changed_by_system == 100, (
            f"should be WASM_GAMEPLAY (100), got {change.changed_by_system}"
        )

    def test_buggy_manifests_have_wasm_causality(self) -> None:
        """Manifests from buggy gameplay should also carry WASM causality."""
        manifests = _load_manifests("buggy")
        m = manifests[0]
        change = m.component_changes[0]

        assert change.changed_by_system == 100
        assert change.reason_type == "GameRule"

    def test_correct_and_buggy_have_different_reasons(self) -> None:
        """Correct and buggy modules should have different causal reasons."""
        correct = _load_manifests("correct")
        buggy = _load_manifests("buggy")

        correct_reason = correct[0].component_changes[0].reason_detail
        buggy_reason = buggy[0].component_changes[0].reason_detail

        assert correct_reason != buggy_reason, (
            f"correct ('{correct_reason}') and buggy ('{buggy_reason}') "
            "should have different causal reasons"
        )

    def test_all_ticks_carry_causality(self) -> None:
        """Every tick in both scenarios should have causality metadata."""
        for scenario in ("correct", "buggy"):
            manifests = _load_manifests(scenario)
            for i, m in enumerate(manifests):
                for change in m.component_changes:
                    assert change.changed_by_system == 100, (
                        f"{scenario} tick {i + 1}: changed_by should be WASM_GAMEPLAY"
                    )
                    assert change.reason_type == "GameRule", (
                        f"{scenario} tick {i + 1}: reason_type should be GameRule"
                    )
                    assert len(change.reason_detail) > 0, (
                        f"{scenario} tick {i + 1}: reason_detail should not be empty"
                    )
