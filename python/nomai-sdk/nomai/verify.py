"""Verification engine that checks intent specs against manifest data.

The verification engine is the core of the Nomai AI verification thesis:
behavioral correctness can be determined from manifest data alone, without
pixel peeking.  Given a :class:`VerificationSuite` and a sequence of
:class:`TickManifest` objects, it produces a structured
:class:`VerificationReport` with per-intent pass/fail results, failure
evidence, causal chains, and heuristic fix suggestions.

All output types are JSON-serializable for regression test storage.
"""

from __future__ import annotations

import json
import logging
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from nomai.physics_sanity import PhysicsEntityInfo

from nomai.intents import (
    Expected,
    ExpectedType,
    IntentKind,
    IntentSpec,
    Trigger,
    TriggerType,
    VerificationSuite,
)
from nomai.manifest import (
    CausalChain,
    ComponentChange,
    TickManifest,
)

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# SuggestedFix
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class SuggestedFix:
    """A heuristic fix suggestion for the AI to act on."""
    intent_name: str
    fix_type: str  # "entity_not_found", "trigger_never_fired", "wrong_value", "timeout", "unknown"
    description: str
    priority: str = "medium"  # "high", "medium", "low"

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "intent_name": self.intent_name,
            "fix_type": self.fix_type,
            "description": self.description,
            "priority": self.priority,
        }


# ---------------------------------------------------------------------------
# IntentResult
# ---------------------------------------------------------------------------

@dataclass
class IntentResult:
    """The verification result for a single intent spec.

    Attributes:
        intent_name: Name of the intent spec that was verified.
        passed: Whether the intent was satisfied.
        failure_reason: Human-readable explanation when ``passed`` is False.
        trigger_tick: The tick at which the trigger fired (behavior intents).
        evidence: Component changes that serve as evidence for the result.
        causal_chain: Optional causal chain tracing the root cause.
        suggestion: Heuristic fix suggestion for the AI to act on.
    """
    intent_name: str
    passed: bool
    failure_reason: str = ""
    trigger_tick: int | None = None
    evidence: list[ComponentChange] = field(default_factory=list)
    causal_chain: CausalChain | None = None
    suggestion: str = ""

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        result: dict[str, object] = {
            "intent_name": self.intent_name,
            "passed": self.passed,
            "failure_reason": self.failure_reason,
            "trigger_tick": self.trigger_tick,
            "evidence": [e.to_dict() for e in self.evidence],
            "causal_chain": self.causal_chain.to_dict() if self.causal_chain else None,
            "suggestion": self.suggestion,
        }
        return result

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> IntentResult:
        """Deserialize from a plain dict (inverse of to_dict)."""
        evidence: list[ComponentChange] = []
        raw_evidence = data.get("evidence", [])
        if isinstance(raw_evidence, list):
            evidence = [ComponentChange.from_dict(e) for e in raw_evidence]  # type: ignore[arg-type]

        raw_chain = data.get("causal_chain")
        causal_chain: CausalChain | None = None
        if isinstance(raw_chain, dict):
            causal_chain = CausalChain.from_dict(raw_chain)

        raw_tick = data.get("trigger_tick")
        trigger_tick: int | None = int(raw_tick) if raw_tick is not None else None  # type: ignore[arg-type]

        return cls(
            intent_name=str(data.get("intent_name", "")),
            passed=bool(data.get("passed", False)),
            failure_reason=str(data.get("failure_reason", "")),
            trigger_tick=trigger_tick,
            evidence=evidence,
            causal_chain=causal_chain,
            suggestion=str(data.get("suggestion", "")),
        )


# ---------------------------------------------------------------------------
# VerificationReport
# ---------------------------------------------------------------------------

@dataclass
class VerificationReport:
    """Aggregate verification report for a complete suite.

    Attributes:
        suite_name: Name of the verification suite.
        total_intents: Total number of intents in the suite.
        passed: Number of intents that passed.
        failed: Number of intents that failed.
        results: Per-intent verification results.
        wall_time_ms: Wall-clock time spent on verification in milliseconds.
        ticks_examined: Number of tick manifests that were examined.
    """
    suite_name: str
    total_intents: int
    passed: int
    failed: int
    results: list[IntentResult]
    wall_time_ms: float = 0.0
    ticks_examined: int = 0

    @property
    def all_passed(self) -> bool:
        """True if every intent in the suite passed."""
        return self.failed == 0

    def summary(self) -> str:
        """Generate a human-readable summary of the verification report.

        Returns a multi-line string suitable for logging or display.
        """
        lines: list[str] = []
        lines.append(f"Verification Report: {self.suite_name}")
        lines.append(f"  Total: {self.total_intents}  Passed: {self.passed}  Failed: {self.failed}")
        lines.append(f"  Ticks examined: {self.ticks_examined}  Wall time: {self.wall_time_ms:.1f}ms")
        lines.append("")

        for r in self.results:
            status = "PASS" if r.passed else "FAIL"
            lines.append(f"  [{status}] {r.intent_name}")
            if not r.passed:
                lines.append(f"         Reason: {r.failure_reason}")
                if r.trigger_tick is not None:
                    lines.append(f"         Trigger tick: {r.trigger_tick}")
                if r.suggestion:
                    lines.append(f"         Suggestion: {r.suggestion}")

        status_line = "ALL PASSED" if self.all_passed else f"{self.failed} FAILED"
        lines.append("")
        lines.append(f"  Result: {status_line}")
        return "\n".join(lines)

    def failures(self) -> list[IntentResult]:
        """Return only the failed intent results."""
        return [r for r in self.results if not r.passed]

    def diagnosis(self) -> str:
        """Produce an AI-readable failure summary for autonomous fixing.

        This is the critical bridge between verification failure and
        the AI's ability to generate a code fix without human help.
        """
        if self.all_passed:
            return "All intents passed. No issues detected."

        parts: list[str] = []
        parts.append(f"VERIFICATION FAILED: {self.failed}/{self.total_intents} intents failed.")
        parts.append("")

        for r in self.failures():
            parts.append(f"FAILED: {r.intent_name}")
            parts.append(f"  Reason: {r.failure_reason}")
            if r.trigger_tick is not None:
                parts.append(f"  Trigger tick: {r.trigger_tick}")
            if r.evidence:
                parts.append(f"  Evidence: {len(r.evidence)} component change(s)")
                for ev in r.evidence[:3]:  # Limit to first 3 for readability
                    parts.append(
                        f"    - entity {ev.entity_id} {ev.component_type_name}: "
                        f"{ev.old_value} -> {ev.new_value} "
                        f"(reason: {ev.reason_type}/{ev.reason_detail})"
                    )
            if r.causal_chain:
                parts.append(f"  Causal chain ({len(r.causal_chain.steps)} steps):")
                for step in r.causal_chain.steps[:5]:
                    parts.append(
                        f"    tick {step.tick}: {step.description} "
                        f"({step.reason_type}/{step.reason_detail})"
                    )
            if r.suggestion:
                parts.append(f"  Suggested fix: {r.suggestion}")
            parts.append("")

        return "\n".join(parts)

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "suite_name": self.suite_name,
            "total_intents": self.total_intents,
            "passed": self.passed,
            "failed": self.failed,
            "results": [r.to_dict() for r in self.results],
            "wall_time_ms": self.wall_time_ms,
            "ticks_examined": self.ticks_examined,
            "all_passed": self.all_passed,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> VerificationReport:
        """Deserialize from a plain dict (inverse of to_dict)."""
        raw_results = data.get("results", [])
        results: list[IntentResult] = []
        if isinstance(raw_results, list):
            results = [IntentResult.from_dict(r) for r in raw_results]  # type: ignore[arg-type]
        return cls(
            suite_name=str(data.get("suite_name", "")),
            total_intents=int(data.get("total_intents", 0)),  # type: ignore[arg-type]
            passed=int(data.get("passed", 0)),  # type: ignore[arg-type]
            failed=int(data.get("failed", 0)),  # type: ignore[arg-type]
            results=results,
            wall_time_ms=float(data.get("wall_time_ms", 0.0)),  # type: ignore[arg-type]
            ticks_examined=int(data.get("ticks_examined", 0)),  # type: ignore[arg-type]
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> VerificationReport:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)

    def suggested_fixes(self) -> list[SuggestedFix]:
        """Generate heuristic fix suggestions for all failures.

        Categorizes failures and produces actionable suggestions.
        """
        fixes: list[SuggestedFix] = []
        for r in self.failures():
            fix_type = self._classify_failure(r)
            fixes.append(SuggestedFix(
                intent_name=r.intent_name,
                fix_type=fix_type,
                description=r.suggestion or r.failure_reason,
                priority="high" if fix_type == "entity_not_found" else "medium",
            ))
        return fixes

    @staticmethod
    def _classify_failure(result: IntentResult) -> str:
        """Classify a failure into a fix type based on heuristics."""
        reason = result.failure_reason.lower()
        if "no entity found" in reason or "not found" in reason:
            return "entity_not_found"
        if "never fired" in reason:
            return "trigger_never_fired"
        if "not met" in reason and ("timeout" in reason or "within" in reason):
            return "timeout"
        if "out of range" in reason or "expected value" in reason:
            return "wrong_value"
        return "unknown"


# ---------------------------------------------------------------------------
# ReplayResult
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class ReplayResult:
    """Result of replaying a regression test."""
    passed: bool
    reason: str
    expected_passed: int
    expected_failed: int
    actual_passed: int
    actual_failed: int

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "passed": self.passed,
            "reason": self.reason,
            "expected_passed": self.expected_passed,
            "expected_failed": self.expected_failed,
            "actual_passed": self.actual_passed,
            "actual_failed": self.actual_failed,
        }


# ---------------------------------------------------------------------------
# RegressionTest
# ---------------------------------------------------------------------------

@dataclass
class RegressionTest:
    """A regression test: suite + manifest snapshots + expected result counts."""
    name: str
    suite: VerificationSuite
    manifests: list[TickManifest]
    expected_pass_count: int
    expected_fail_count: int

    @classmethod
    def create(
        cls,
        name: str,
        suite: VerificationSuite,
        manifests: list[TickManifest],
        report: VerificationReport,
    ) -> RegressionTest:
        """Create a regression test from a verification run."""
        return cls(
            name=name,
            suite=suite,
            manifests=manifests,
            expected_pass_count=report.passed,
            expected_fail_count=report.failed,
        )

    def replay(
        self,
        engine: VerificationEngine,
        manifests_override: list[TickManifest] | None = None,
    ) -> ReplayResult:
        """Replay the regression test and compare results to expectations."""
        manifests = manifests_override if manifests_override is not None else self.manifests
        report = engine.verify(self.suite, manifests)

        if report.passed == self.expected_pass_count and report.failed == self.expected_fail_count:
            return ReplayResult(
                passed=True,
                reason="Regression test passed: results match expected counts",
                expected_passed=self.expected_pass_count,
                expected_failed=self.expected_fail_count,
                actual_passed=report.passed,
                actual_failed=report.failed,
            )
        return ReplayResult(
            passed=False,
            reason=(
                f"Regression drift detected: expected {self.expected_pass_count} pass / "
                f"{self.expected_fail_count} fail, got {report.passed} pass / {report.failed} fail"
            ),
            expected_passed=self.expected_pass_count,
            expected_failed=self.expected_fail_count,
            actual_passed=report.passed,
            actual_failed=report.failed,
        )

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "suite": self.suite.to_dict(),
            "manifests": [m.to_dict() for m in self.manifests],
            "expected_pass_count": self.expected_pass_count,
            "expected_fail_count": self.expected_fail_count,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> RegressionTest:
        """Deserialize from a plain dict (inverse of to_dict)."""
        raw_suite = data.get("suite", {})
        suite = VerificationSuite.from_dict(raw_suite)  # type: ignore[arg-type]
        raw_manifests = data.get("manifests", [])
        manifests: list[TickManifest] = []
        if isinstance(raw_manifests, list):
            manifests = [TickManifest.from_dict(m) for m in raw_manifests]  # type: ignore[arg-type]
        return cls(
            name=str(data.get("name", "")),
            suite=suite,
            manifests=manifests,
            expected_pass_count=int(data.get("expected_pass_count", 0)),  # type: ignore[arg-type]
            expected_fail_count=int(data.get("expected_fail_count", 0)),  # type: ignore[arg-type]
        )

    def save(self, path: str | Path) -> None:
        """Save this regression test to a JSON file."""
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(self.to_dict(), indent=2), encoding="utf-8")

    @classmethod
    def load(cls, path: str | Path) -> RegressionTest:
        """Load a regression test from a JSON file."""
        p = Path(path)
        if not p.exists():
            msg = f"Regression test file not found: {p}"
            raise FileNotFoundError(msg)
        data: dict[str, object] = json.loads(p.read_text(encoding="utf-8"))
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# VerificationEngine
# ---------------------------------------------------------------------------

class VerificationEngine:
    """Engine that verifies intent specs against tick manifest data.

    The verification engine evaluates each intent in a
    :class:`VerificationSuite` against a sequence of tick manifests and
    produces a structured :class:`VerificationReport`.

    Usage::

        engine = VerificationEngine()
        report = engine.verify(suite, manifests, entity_index)
        if not report.all_passed:
            print(report.diagnosis())
    """

    def verify(
        self,
        suite: VerificationSuite,
        manifests: list[TickManifest],
        entity_index: dict[str, dict[str, str]] | None = None,
        physics_registry: dict[int, PhysicsEntityInfo] | None = None,
    ) -> VerificationReport:
        """Verify all intents in a suite against the given manifests.

        Args:
            suite: The verification suite containing intent specs.
            manifests: Ordered list of tick manifests from the simulation.
            entity_index: Optional mapping of entity name/role to metadata.
                Keys are entity roles (e.g. ``"paddle"``), values are dicts
                with keys like ``"entity_type"``, ``"role"``, ``"tier"``.
            physics_registry: Optional mapping of entity ID to
                :class:`~nomai.physics_sanity.PhysicsEntityInfo`. When
                provided, automatic physics sanity checks run alongside
                intent verification.

        Returns:
            A :class:`VerificationReport` with per-intent results.
        """
        start_time = time.monotonic()
        results: list[IntentResult] = []

        if entity_index is None:
            entity_index = {}

        for intent in suite.intents:
            if intent.kind == IntentKind.ENTITY:
                result = self._verify_entity(intent, manifests, entity_index)
            elif intent.kind == IntentKind.BEHAVIOR:
                result = self._verify_behavior(intent, manifests)
            elif intent.kind == IntentKind.METRIC:
                result = self._verify_metric(intent, manifests)
            elif intent.kind == IntentKind.INVARIANT:
                result = self._verify_invariant(intent, manifests)
            else:
                result = IntentResult(
                    intent_name=intent.name,
                    passed=False,
                    failure_reason=f"Unknown intent kind: {intent.kind}",
                )
            results.append(result)

        # Run physics sanity checks if registry provided
        if physics_registry is not None:
            from nomai.physics_sanity import PhysicsSanityChecker

            checker = PhysicsSanityChecker(physics_registry)
            sanity_results = checker.check_collision_responses(manifests)
            sanity_results.extend(checker.check_static_immobility(manifests))
            sanity_results.extend(checker.check_no_tunneling(manifests))
            results.extend(sanity_results)

        elapsed_ms = (time.monotonic() - start_time) * 1000.0
        passed_count = sum(1 for r in results if r.passed)
        failed_count = len(results) - passed_count

        report = VerificationReport(
            suite_name=suite.name,
            total_intents=len(results),
            passed=passed_count,
            failed=failed_count,
            results=results,
            wall_time_ms=elapsed_ms,
            ticks_examined=len(manifests),
        )

        logger.info(
            "Verification complete: %s -- %d/%d passed in %.1fms",
            suite.name,
            passed_count,
            len(results),
            elapsed_ms,
        )

        return report

    # -- Entity verification ------------------------------------------------

    def _verify_entity(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
        entity_index: dict[str, dict[str, str]],
    ) -> IntentResult:
        """Verify that an entity with the given role exists.

        First checks the entity index for a direct match. If not found,
        scans manifest component changes for identity changes that match
        the expected role.
        """
        role = intent.entity_role or ""

        # Check entity index first
        if role in entity_index:
            entry = entity_index[role]
            # If entity_type is specified, check it matches
            if intent.entity_type is not None:
                idx_type = entry.get("entity_type", "")
                if idx_type and idx_type != intent.entity_type:
                    return IntentResult(
                        intent_name=intent.name,
                        passed=False,
                        failure_reason=(
                            f"Entity with role '{role}' found but type "
                            f"'{idx_type}' does not match expected '{intent.entity_type}'"
                        ),
                        suggestion=(
                            f"Change the entity type for '{role}' from "
                            f"'{idx_type}' to '{intent.entity_type}'."
                        ),
                    )
            return IntentResult(
                intent_name=intent.name,
                passed=True,
            )

        # Fallback: scan manifests for identity component changes with matching role
        for manifest in manifests:
            for change in manifest.component_changes:
                if change.component_type_name == "identity":
                    new_val = change.new_value
                    if isinstance(new_val, dict) and new_val.get("role") == role:
                        return IntentResult(
                            intent_name=intent.name,
                            passed=True,
                            evidence=[change],
                        )

        return IntentResult(
            intent_name=intent.name,
            passed=False,
            failure_reason=f"No entity found with role '{role}'",
            suggestion=(
                f"Add a spawn command for an entity with role '{role}' "
                f"and type '{intent.entity_type or 'unknown'}' to the gameplay module."
            ),
        )

    # -- Behavior verification ----------------------------------------------

    def _verify_behavior(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Verify a behavior: find trigger tick, then check expected outcome.

        Scans manifests sequentially for the trigger condition. Once the
        trigger fires, scans the remaining manifests (up to
        ``timeout_ticks``) for the expected outcome.
        """
        if intent.trigger is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason="Behavior intent has no trigger defined",
                suggestion="Define a trigger for this behavior intent.",
            )

        if intent.expected is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason="Behavior intent has no expected outcome defined",
                suggestion="Define an expected outcome for this behavior intent.",
            )

        # Handle AFTER triggers with two-phase resolution
        if intent.trigger.type == TriggerType.AFTER:
            resolved_idx, after_reason = self._resolve_after_trigger(
                intent.trigger, manifests
            )
            if resolved_idx is None:
                return IntentResult(
                    intent_name=intent.name,
                    passed=False,
                    failure_reason=f"After trigger failed: {after_reason}",
                    suggestion="Ensure the child trigger condition occurs during simulation and the delay does not exceed the simulation length.",
                )
            trigger_tick_idx = resolved_idx
        else:
            # Phase 1: Find trigger tick
            trigger_tick_idx_found: int | None = None
            for idx, manifest in enumerate(manifests):
                if self._check_trigger(intent.trigger, manifest):
                    trigger_tick_idx_found = idx
                    break

            if trigger_tick_idx_found is None:
                return IntentResult(
                    intent_name=intent.name,
                    passed=False,
                    failure_reason=(
                        f"Trigger '{intent.trigger.type.value}' never fired "
                        f"across {len(manifests)} ticks"
                    ),
                    suggestion=(
                        "Ensure the game state produces the trigger condition. "
                        "Check that the relevant entities exist and interact correctly."
                    ),
                )
            trigger_tick_idx = trigger_tick_idx_found

        trigger_tick = manifests[trigger_tick_idx].tick

        # Phase 2: Check expected outcome after trigger
        timeout = intent.timeout_ticks
        end_idx = min(trigger_tick_idx + timeout, len(manifests))

        for idx in range(trigger_tick_idx, end_idx):
            manifest = manifests[idx]
            if self._check_expected(intent.expected, manifest):
                # Collect evidence from the matching manifest
                evidence = list(manifest.component_changes)
                return IntentResult(
                    intent_name=intent.name,
                    passed=True,
                    trigger_tick=trigger_tick,
                    evidence=evidence,
                )

        return IntentResult(
            intent_name=intent.name,
            passed=False,
            trigger_tick=trigger_tick,
            failure_reason=(
                f"Expected outcome '{intent.expected.type.value}' not met "
                f"within {timeout} ticks after trigger at tick {trigger_tick}"
            ),
            suggestion=(
                "The trigger fired but the expected outcome was not observed. "
                "Check the gameplay logic that should respond to the trigger event."
            ),
        )

    # -- After trigger resolution -------------------------------------------

    def _resolve_after_trigger(
        self,
        trigger: Trigger,
        manifests: list[TickManifest],
    ) -> tuple[int | None, str]:
        """Resolve an AFTER trigger: find child trigger tick + delay.

        Returns ``(resolved_index, failure_reason)`` where *resolved_index*
        is the manifest index the After trigger resolves to, or ``None`` on
        failure.  *failure_reason* is empty on success and describes the
        specific failure mode otherwise.
        """
        if trigger.type != TriggerType.AFTER or not trigger.children:
            return None, "trigger is not AFTER or has no children"
        child = trigger.children[0]
        raw_delay = trigger.params.get("delay_ticks", 0)
        delay = int(raw_delay) if raw_delay is not None else 0  # type: ignore[arg-type]

        # Find when child fires
        child_idx: int | None = None
        for idx, manifest in enumerate(manifests):
            if self._check_trigger(child, manifest):
                child_idx = idx
                break

        if child_idx is None:
            return None, "child trigger never fired"

        resolved_idx = child_idx + delay
        if resolved_idx >= len(manifests):
            return None, (
                f"child trigger fired at tick index {child_idx} but "
                f"delay of {delay} ticks exceeds available manifests "
                f"(need index {resolved_idx}, have {len(manifests)})"
            )
        return resolved_idx, ""

    # -- Metric verification ------------------------------------------------

    def _verify_metric(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Verify a metric: check component values stay within range.

        Scans all component changes across all manifests for values of
        the specified component/field on the specified entity. If any
        value falls outside the metric range, the intent fails.
        """
        if intent.metric_range is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason="Metric intent has no metric_range defined",
            )

        range_min, range_max = intent.metric_range
        entity_name = intent.metric_entity or ""
        component = intent.metric_component or ""
        field_name = intent.metric_field or ""

        for manifest in manifests:
            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue

                # Extract the field value from new_value
                value = self._extract_field_value(change.new_value, field_name)
                if value is None:
                    continue

                if not isinstance(value, (int, float)):
                    continue

                if value < range_min or value > range_max:
                    return IntentResult(
                        intent_name=intent.name,
                        passed=False,
                        trigger_tick=manifest.tick,
                        failure_reason=(
                            f"Metric '{field_name}' on '{entity_name}.{component}' "
                            f"value {value} out of range [{range_min}, {range_max}] "
                            f"at tick {manifest.tick}"
                        ),
                        evidence=[change],
                        suggestion=(
                            f"Clamp or limit the '{field_name}' value of "
                            f"'{component}' to stay within [{range_min}, {range_max}]."
                        ),
                    )

        return IntentResult(
            intent_name=intent.name,
            passed=True,
        )

    # -- Invariant verification ---------------------------------------------

    def _verify_invariant(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Verify an invariant: check condition holds every tick.

        For the spike implementation, invariant conditions are evaluated
        as simple manifest queries. The condition string is matched
        against manifest aggregates and component changes.

        Supported condition formats:
        - ``"aggregate:<entity_type> <op> <value>"`` -- e.g.
          ``"aggregate:brick > 0"``
        - ``"entity_count > <N>"`` -- total entity count check
        - ``"component_range:<entity>.<component>.<field> in [<min>, <max>]"``
          -- checks that a component field stays within the given range
        - Free-form conditions are stored but pass trivially in the spike
          (full expression evaluation is post-MVP).
        """
        condition = intent.condition or ""

        # Parse aggregate conditions
        if condition.startswith("aggregate:"):
            return self._verify_aggregate_invariant(intent.name, condition, manifests)

        # Parse entity_count conditions
        if condition.startswith("entity_count"):
            return self._verify_entity_count_invariant(intent.name, condition, manifests)

        # Parse component_range conditions
        # Format: "component_range:<entity>.<component>.<field> in [<min>, <max>]"
        if condition.startswith("component_range:"):
            return self._verify_component_range_invariant(
                intent.name, condition, manifests
            )

        # For the spike, free-form conditions pass trivially with a warning
        logger.warning(
            "Invariant '%s' has free-form condition that cannot be evaluated "
            "in the spike; treating as pass. Condition: %s",
            intent.name,
            condition,
        )
        return IntentResult(
            intent_name=intent.name,
            passed=True,
            suggestion=(
                "This invariant uses a free-form condition string that "
                "is not evaluated in the spike. Convert to a structured "
                "condition (aggregate:, entity_count, or component_range:) "
                "for full evaluation."
            ),
        )

    def _verify_aggregate_invariant(
        self,
        intent_name: str,
        condition: str,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Evaluate an aggregate invariant condition.

        Condition format: ``"aggregate:<entity_type> <op> <value>"``
        """
        # Parse: "aggregate:brick > 0"
        try:
            rest = condition[len("aggregate:"):]
            parts = rest.split()
            if len(parts) < 3:
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    failure_reason=f"Malformed aggregate condition: '{condition}'",
                )
            entity_type = parts[0]
            op = parts[1]
            target_value = float(parts[2])
        except (ValueError, IndexError) as exc:
            return IntentResult(
                intent_name=intent_name,
                passed=False,
                failure_reason=f"Failed to parse aggregate condition '{condition}': {exc}",
            )

        for manifest in manifests:
            actual = manifest.aggregates.entity_count_by_type.get(entity_type, 0)
            if not self._compare(float(actual), op, target_value):
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    trigger_tick=manifest.tick,
                    failure_reason=(
                        f"Aggregate invariant violated at tick {manifest.tick}: "
                        f"{entity_type} count is {actual}, expected {op} {target_value}"
                    ),
                    suggestion=(
                        f"Ensure '{entity_type}' count satisfies "
                        f"'{op} {target_value}' on every tick."
                    ),
                )

        return IntentResult(
            intent_name=intent_name,
            passed=True,
        )

    def _verify_entity_count_invariant(
        self,
        intent_name: str,
        condition: str,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Evaluate an entity_count invariant condition.

        Condition format: ``"entity_count <op> <value>"``
        """
        try:
            parts = condition.split()
            if len(parts) < 3:
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    failure_reason=f"Malformed entity_count condition: '{condition}'",
                )
            op = parts[1]
            target_value = float(parts[2])
        except (ValueError, IndexError) as exc:
            return IntentResult(
                intent_name=intent_name,
                passed=False,
                failure_reason=f"Failed to parse entity_count condition '{condition}': {exc}",
            )

        for manifest in manifests:
            actual = manifest.aggregates.total_entity_count
            if not self._compare(float(actual), op, target_value):
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    trigger_tick=manifest.tick,
                    failure_reason=(
                        f"Entity count invariant violated at tick {manifest.tick}: "
                        f"total is {actual}, expected {op} {target_value}"
                    ),
                    suggestion=(
                        f"Ensure total entity count satisfies "
                        f"'{op} {target_value}' on every tick."
                    ),
                )

        return IntentResult(
            intent_name=intent_name,
            passed=True,
        )

    def _verify_component_range_invariant(
        self,
        intent_name: str,
        condition: str,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Evaluate a component_range invariant.

        Format: ``"component_range:<entity>.<component>.<field> in [<min>, <max>]"``

        Scans all tick manifests for component changes matching the
        entity/component, extracts the field value from new_value, and
        checks it falls within [min, max].
        """
        try:
            rest = condition[len("component_range:"):]
            path_part, range_part = rest.split(" in ")
            path_parts = path_part.strip().split(".")
            if len(path_parts) != 3:
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    failure_reason=(
                        f"Malformed component_range: need entity.component.field, "
                        f"got '{path_part.strip()}'"
                    ),
                )
            entity_name, component, field_name = path_parts
            range_trimmed = range_part.strip()
            if not (range_trimmed.startswith("[") and range_trimmed.endswith("]")):
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    failure_reason=(
                        f"Malformed component_range: range must be enclosed in "
                        f"brackets, e.g. [0, 800], got '{range_trimmed}'"
                    ),
                )
            range_str = range_trimmed[1:-1]
            range_min, range_max = [float(x.strip()) for x in range_str.split(",")]
            if range_min > range_max:
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    failure_reason=(
                        f"Malformed component_range: min ({range_min}) > max "
                        f"({range_max}) in '{condition}'"
                    ),
                )
        except Exception as exc:
            return IntentResult(
                intent_name=intent_name,
                passed=False,
                failure_reason=f"Failed to parse component_range '{condition}': {exc}",
            )

        for manifest in manifests:
            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue
                if not self._matches_entity(change, entity_name):
                    continue
                value = self._extract_field_value(change.new_value, field_name)
                if not isinstance(value, (int, float)):
                    continue
                if value < range_min or value > range_max:
                    return IntentResult(
                        intent_name=intent_name,
                        passed=False,
                        trigger_tick=manifest.tick,
                        failure_reason=(
                            f"Entity '{entity_name}' {component}.{field_name} = {value} "
                            f"out of range [{range_min}, {range_max}] at tick {manifest.tick}"
                        ),
                        evidence=[change],
                        suggestion=(
                            f"Clamp '{entity_name}' {component}.{field_name} "
                            f"to stay within [{range_min}, {range_max}]."
                        ),
                    )

        return IntentResult(intent_name=intent_name, passed=True)

    # -- Trigger evaluation -------------------------------------------------

    def _check_trigger(self, trigger: Trigger, manifest: TickManifest) -> bool:
        """Evaluate a trigger condition against a single tick manifest.

        Returns True if the trigger condition is satisfied.
        """
        if trigger.type == TriggerType.TICK_REACHED:
            target_tick = trigger.params.get("tick", 0)
            if isinstance(target_tick, (int, float)):
                return manifest.tick >= int(target_tick)
            return False

        if trigger.type == TriggerType.EVENT_OCCURRED:
            event_type = str(trigger.params.get("event_type", ""))
            involving = trigger.params.get("involving")
            for event in manifest.events:
                if event.event_type == event_type:
                    if involving is None:
                        return True
                    if isinstance(involving, list):
                        detail = event.reason_detail.lower()
                        desc = event.description.lower()
                        search_text = f"{detail} {desc}"
                        if all(name.lower() in search_text for name in involving):
                            return True
            return False

        if trigger.type == TriggerType.COMPONENT_CONDITION:
            component = str(trigger.params.get("component", ""))
            field_name = str(trigger.params.get("field", ""))
            op = str(trigger.params.get("comparison", ""))
            expected_value = trigger.params.get("value")

            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue
                value = self._extract_field_value(change.new_value, field_name)
                if value is None:
                    continue
                if isinstance(value, (int, float)) and isinstance(expected_value, (int, float)):
                    if self._compare(float(value), op, float(expected_value)):
                        return True
                elif isinstance(value, str) and isinstance(expected_value, str):
                    if self._compare_str(value, op, expected_value):
                        return True
            return False

        if trigger.type == TriggerType.AGGREGATE_CONDITION:
            entity_type = str(trigger.params.get("entity_type", ""))
            op = str(trigger.params.get("comparison", ""))
            target = trigger.params.get("value", 0)
            if not isinstance(target, (int, float)):
                return False
            actual = manifest.aggregates.entity_count_by_type.get(entity_type, 0)
            return self._compare(float(actual), op, float(target))

        if trigger.type == TriggerType.COLLISION:
            entity_a = str(trigger.params.get("entity_a", ""))
            entity_b = str(trigger.params.get("entity_b", ""))
            for event in manifest.events:
                if event.event_type == "collision":
                    detail = event.reason_detail.lower()
                    if entity_a.lower() in detail and entity_b.lower() in detail:
                        return True
            return False

        if trigger.type == TriggerType.AND:
            return all(
                self._check_trigger(child, manifest)
                for child in trigger.children
            )

        if trigger.type == TriggerType.OR:
            return any(
                self._check_trigger(child, manifest)
                for child in trigger.children
            )

        if trigger.type == TriggerType.STATE_TRANSITION:
            entity_name = str(trigger.params.get("entity", ""))
            from_state = str(trigger.params.get("from_state", ""))
            to_state = str(trigger.params.get("to_state", ""))
            for change in manifest.component_changes:
                if change.old_value == from_state and change.new_value == to_state:
                    detail = change.reason_detail.lower()
                    if entity_name.lower() in detail:
                        return True
            return False

        if trigger.type == TriggerType.AFTER:
            # AFTER triggers are evaluated at the behavior level via _resolve_after_trigger
            return False

        logger.warning("Unknown trigger type: %s", trigger.type)
        return False

    # -- Expected evaluation ------------------------------------------------

    def _check_expected(self, expected: Expected, manifest: TickManifest) -> bool:
        """Evaluate an expected outcome against a single tick manifest.

        Returns True if the expected outcome is satisfied.
        """
        if expected.type == ExpectedType.COMPONENT_CHANGED:
            component = str(expected.params.get("component", ""))
            field_name = expected.params.get("field")
            expected_value = expected.params.get("expected_value")
            entity_name = expected.params.get("entity")

            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue
                # Filter by entity name if specified
                if entity_name and not self._matches_entity(change, str(entity_name)):
                    continue
                # If a specific field is required, check it exists and changed
                if field_name is not None:
                    new_val = self._extract_field_value(change.new_value, str(field_name))
                    if new_val is None:
                        continue
                    # Verify the value actually changed (old != new for this field)
                    old_val = self._extract_field_value(change.old_value, str(field_name))
                    if old_val is not None and old_val == new_val:
                        continue
                    if expected_value is not None and new_val != expected_value:
                        continue
                elif expected_value is not None:
                    if change.new_value != expected_value:
                        continue
                else:
                    # No field or expected_value: require old and new to differ
                    if change.old_value is not None and change.old_value == change.new_value:
                        continue
                return True
            return False

        if expected.type == ExpectedType.ENTITY_DESPAWNED:
            entity_name = str(expected.params.get("entity", ""))
            if not manifest.entity_despawns:
                return False

            # Try to match the entity name against evidence in the manifest.
            # entity_despawns contains integer entity IDs; we correlate them
            # with events and component changes that reference the entity name.
            despawn_set = set(manifest.entity_despawns)

            # Check events: if an event's description or reason_detail mentions
            # the entity name AND involves a despawned entity, it's a match.
            for event in manifest.events:
                search_text = f"{event.description} {event.reason_detail}".lower()
                if entity_name.lower() in search_text:
                    if any(eid in despawn_set for eid in event.involved_entities):
                        return True

            # Check component changes on despawned entities for identity match.
            for change in manifest.component_changes:
                if change.entity_id not in despawn_set:
                    continue
                if change.component_type_name == "identity":
                    if isinstance(change.new_value, dict):
                        role = str(change.new_value.get("role", ""))
                        etype = str(change.new_value.get("entity_type", ""))
                        if entity_name.lower() in (role.lower(), etype.lower()):
                            return True
                # Also check reason_detail for entity name
                if entity_name.lower() in change.reason_detail.lower():
                    return True

            # Fallback: check if the entity_id string matches the entity name
            for eid in manifest.entity_despawns:
                if str(eid) == entity_name:
                    return True

            return False

        if expected.type == ExpectedType.EVENT_EMITTED:
            event_type = str(expected.params.get("event_type", ""))
            for event in manifest.events:
                if event.event_type == event_type:
                    return True
            return False

        if expected.type == ExpectedType.AGGREGATE_CHANGED:
            entity_type = str(expected.params.get("entity_type", ""))
            op = str(expected.params.get("comparison", ""))
            target = expected.params.get("value", 0)
            if not isinstance(target, (int, float)):
                return False
            actual = manifest.aggregates.entity_count_by_type.get(entity_type, 0)
            return self._compare(float(actual), op, float(target))

        if expected.type == ExpectedType.IN_STATE:
            component = str(expected.params.get("component", ""))
            state = str(expected.params.get("state", ""))
            for change in manifest.component_changes:
                if change.component_type_name == component:
                    if change.new_value == state:
                        return True
            return False

        if expected.type == ExpectedType.VALUE_RELATION:
            entity_name = expected.params.get("entity")
            component = str(expected.params.get("component", ""))
            field_name = str(expected.params.get("field", ""))
            relation = str(expected.params.get("relation", ""))
            tolerance = float(expected.params.get("tolerance", 0.1))  # type: ignore[arg-type]

            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue
                # Filter by entity name if specified
                if entity_name and not self._matches_entity(change, str(entity_name)):
                    continue
                old_val = self._extract_field_value(change.old_value, field_name)
                new_val = self._extract_field_value(change.new_value, field_name)
                if old_val is None or new_val is None:
                    continue
                if not isinstance(old_val, (int, float)) or not isinstance(new_val, (int, float)):
                    continue

                old_f = float(old_val)
                new_f = float(new_val)

                if relation == "sign_flipped":
                    if old_f * new_f < 0:  # opposite signs, neither zero
                        return True
                elif relation == "magnitude_preserved":
                    if abs(old_f) > 0:
                        if abs(abs(new_f) - abs(old_f)) / abs(old_f) <= tolerance:
                            return True
                elif relation == "increased":
                    if new_f > old_f:
                        return True
                elif relation == "decreased":
                    if new_f < old_f:
                        return True
                elif relation == "changed_by_more_than":
                    if abs(new_f - old_f) > tolerance:
                        return True
            return False

        if expected.type == ExpectedType.ALL:
            return all(
                self._check_expected(child, manifest)
                for child in expected.children
            )

        if expected.type == ExpectedType.ANY:
            return any(
                self._check_expected(child, manifest)
                for child in expected.children
            )

        logger.warning("Unknown expected type: %s", expected.type)
        return False

    # -- Comparison helpers -------------------------------------------------

    def _compare(self, actual: float, op: str, expected: float) -> bool:
        """Evaluate a numeric comparison.

        Supported operators: ``==``, ``!=``, ``<``, ``<=``, ``>``, ``>=``.
        """
        if op == "==":
            return actual == expected
        if op == "!=":
            return actual != expected
        if op == "<":
            return actual < expected
        if op == "<=":
            return actual <= expected
        if op == ">":
            return actual > expected
        if op == ">=":
            return actual >= expected
        logger.warning("Unknown comparison operator: %s", op)
        return False

    def _compare_str(self, actual: str, op: str, expected: str) -> bool:
        """Evaluate a string comparison (equality/inequality only)."""
        if op == "==":
            return actual == expected
        if op == "!=":
            return actual != expected
        logger.warning("String comparison with operator '%s' not supported", op)
        return False

    @staticmethod
    def _matches_entity(change: ComponentChange, entity_name: str) -> bool:
        """Check if a component change belongs to the named entity.

        Uses a dual strategy:
        - If ``entity_name`` is numeric, matches against ``change.entity_id``.
        - If ``entity_name`` appears in ``change.reason_detail``, matches.
        - If neither can be checked (non-numeric name with generic
          reason_detail), returns True (permissive -- avoids false negatives).
        """
        # Try numeric entity ID match
        try:
            return change.entity_id == int(entity_name)
        except (ValueError, TypeError):
            pass
        # Try role/type match via reason_detail
        detail = change.reason_detail.lower()
        name_lower = entity_name.lower()
        if name_lower in detail:
            return True
        # If reason_detail contains a colon-separated role format (e.g.
        # "ball:brick"), we can check if the entity name is NOT present
        # to reject mismatches.  Only reject when we have clear negative
        # evidence.
        if ":" in detail:
            return False
        # No evidence to filter on -- include the change (permissive).
        return True

    def _extract_field_value(
        self,
        value: object,
        field_name: str,
    ) -> object:
        """Extract a named field from a component value.

        If ``value`` is a dict, returns ``value[field_name]``.
        If ``value`` is a scalar and ``field_name`` is empty, returns value.
        Otherwise returns None.
        """
        if not field_name:
            return value
        if isinstance(value, dict):
            return value.get(field_name)
        return None
