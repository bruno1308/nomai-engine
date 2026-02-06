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

import logging
import time
from dataclasses import dataclass, field

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
    CausalStep,
    ComponentChange,
    GameEvent,
    TickManifest,
)

logger = logging.getLogger(__name__)


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
    ) -> VerificationReport:
        """Verify all intents in a suite against the given manifests.

        Args:
            suite: The verification suite containing intent specs.
            manifests: Ordered list of tick manifests from the simulation.
            entity_index: Optional mapping of entity name/role to metadata.
                Keys are entity roles (e.g. ``"paddle"``), values are dicts
                with keys like ``"entity_type"``, ``"role"``, ``"tier"``.

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

        # Phase 1: Find trigger tick
        trigger_tick_idx: int | None = None
        for idx, manifest in enumerate(manifests):
            if self._check_trigger(intent.trigger, manifest):
                trigger_tick_idx = idx
                break

        if trigger_tick_idx is None:
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
                "condition (aggregate: or entity_count) for full evaluation."
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
                    # If involving is specified, all listed entities must appear
                    # (simplified: check event description or involved_entities)
                    if isinstance(involving, list):
                        # For spike: just check event type match is sufficient
                        return True
            return False

        if trigger.type == TriggerType.COMPONENT_CONDITION:
            entity = str(trigger.params.get("entity", ""))
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
            # Check for collision events in the manifest
            for event in manifest.events:
                if event.event_type == "collision":
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

        # STATE_TRANSITION: check component changes for state transitions
        if trigger.type == TriggerType.STATE_TRANSITION:
            entity = str(trigger.params.get("entity", ""))
            from_state = str(trigger.params.get("from_state", ""))
            to_state = str(trigger.params.get("to_state", ""))
            for change in manifest.component_changes:
                if change.old_value == from_state and change.new_value == to_state:
                    return True
            return False

        logger.warning("Unknown trigger type: %s", trigger.type)
        return False

    # -- Expected evaluation ------------------------------------------------

    def _check_expected(self, expected: Expected, manifest: TickManifest) -> bool:
        """Evaluate an expected outcome against a single tick manifest.

        Returns True if the expected outcome is satisfied.
        """
        if expected.type == ExpectedType.COMPONENT_CHANGED:
            entity = str(expected.params.get("entity", ""))
            component = str(expected.params.get("component", ""))
            field_name = expected.params.get("field")
            expected_value = expected.params.get("expected_value")

            for change in manifest.component_changes:
                if change.component_type_name != component:
                    continue
                # If a specific field is required, check the field exists in new_value
                if field_name is not None:
                    value = self._extract_field_value(change.new_value, str(field_name))
                    if value is None:
                        continue
                    if expected_value is not None and value != expected_value:
                        continue
                elif expected_value is not None:
                    if change.new_value != expected_value:
                        continue
                return True
            return False

        if expected.type == ExpectedType.ENTITY_DESPAWNED:
            # Simplified: check that entity_despawns is non-empty
            return len(manifest.entity_despawns) > 0

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
