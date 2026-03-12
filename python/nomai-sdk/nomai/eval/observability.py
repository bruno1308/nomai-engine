"""Observability dimension evaluation -- manifest fidelity.

Answers: "Can AI reconstruct full game state from manifest output?"

Metrics:
- ``manifest_change_recall``: Are all meaningful state changes captured?
- ``state_reconstruction_fidelity``: Can entity state be reconstructed
  from manifests alone?
- ``root_cause_recoverability_at_k``: Do causal chains surface the true
  root cause within the first *k* steps?
"""

from __future__ import annotations

import logging

from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.manifest import CausalChain, ComponentChange, TickManifest

logger = logging.getLogger(__name__)


def manifest_change_recall(
    manifests: list[TickManifest],
    ground_truth_changes: list[ComponentChange],
) -> MetricResult:
    """Compute recall of component changes captured by the manifest.

    For each ground-truth change, checks whether a matching entry exists
    in the manifest sequence (same entity, component, tick).

    Args:
        manifests: Manifests produced by the engine.
        ground_truth_changes: Known-correct list of changes that should
            appear in the manifests.

    Returns:
        MetricResult with recall value (0.0-1.0).  Target >= 0.995.
    """
    if not ground_truth_changes:
        return MetricResult(
            name="manifest_change_recall",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.995,
            passed=True,
            detail="No ground truth changes to compare (vacuously correct).",
        )

    # Build lookup set from manifest changes.
    manifest_keys: set[tuple[int, str, int]] = set()
    for m in manifests:
        for c in m.component_changes:
            manifest_keys.add((c.entity_id, c.component_type_name, c.tick))

    matched = 0
    for gt in ground_truth_changes:
        key = (gt.entity_id, gt.component_type_name, gt.tick)
        if key in manifest_keys:
            matched += 1

    recall = matched / len(ground_truth_changes)
    target = 0.995
    return MetricResult(
        name="manifest_change_recall",
        dimension=EvalDimension.OBSERVABILITY,
        value=recall,
        target=target,
        passed=recall >= target,
        detail=f"{matched}/{len(ground_truth_changes)} ground-truth changes found in manifests.",
    )


def state_reconstruction_fidelity(
    manifests: list[TickManifest],
    ground_truth_states: dict[int, dict[str, object]],
) -> MetricResult:
    """Check whether entity state can be reconstructed from manifests.

    Reconstructs current component values for each entity by replaying
    ``component_changes`` from the manifest sequence, then compares
    against a ground-truth snapshot of entity states.

    Args:
        manifests: Ordered manifest sequence.
        ground_truth_states: Mapping of ``entity_id`` to
            ``{component_name: expected_value}`` representing the
            expected state after all manifests have been applied.

    Returns:
        MetricResult with fidelity fraction (0.0-1.0).  Target = 1.0.
    """
    if not ground_truth_states:
        return MetricResult(
            name="state_reconstruction_fidelity",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=1.0,
            passed=True,
            detail="No ground truth states provided (vacuously correct).",
        )

    # Reconstruct state by replaying changes.
    reconstructed: dict[int, dict[str, object]] = {}
    for m in manifests:
        for c in m.component_changes:
            if c.entity_id not in reconstructed:
                reconstructed[c.entity_id] = {}
            reconstructed[c.entity_id][c.component_type_name] = c.new_value

    matching = 0
    total = len(ground_truth_states)
    mismatches: list[str] = []

    for eid, expected_components in ground_truth_states.items():
        actual = reconstructed.get(eid, {})
        if all(
            actual.get(comp) == val
            for comp, val in expected_components.items()
        ):
            matching += 1
        else:
            mismatches.append(f"entity {eid}")

    fidelity = matching / total
    mismatch_detail = f" Mismatches: {', '.join(mismatches[:5])}." if mismatches else ""
    return MetricResult(
        name="state_reconstruction_fidelity",
        dimension=EvalDimension.OBSERVABILITY,
        value=fidelity,
        target=1.0,
        passed=fidelity >= 1.0,
        detail=f"{matching}/{total} entities reconstructed correctly.{mismatch_detail}",
    )


def root_cause_recoverability_at_k(
    causal_chains: list[CausalChain],
    ground_truth_causes: dict[str, str],
    k: int = 3,
) -> MetricResult:
    """Check if true root causes appear in the first *k* causal steps.

    For each causal chain, checks whether the known ground-truth root
    cause string appears in the ``reason_detail`` or ``description`` of
    the first *k* steps.

    Args:
        causal_chains: Causal chains produced by the engine for failures.
        ground_truth_causes: Mapping of ``component`` name to the
            expected root-cause substring.
        k: How many steps deep to search.  Default 3.

    Returns:
        MetricResult with recoverability rate.  Target >= 0.9 at k=3.
    """
    if not causal_chains:
        return MetricResult(
            name=f"root_cause_recoverability_at_{k}",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.9,
            passed=True,
            detail="No causal chains to evaluate (vacuously correct).",
        )

    recoverable = 0
    for chain in causal_chains:
        expected = ground_truth_causes.get(chain.component, "")
        if not expected:
            recoverable += 1  # No ground truth -> skip.
            continue
        found = False
        for step in chain.steps[:k]:
            if expected in step.reason_detail or expected in step.description:
                found = True
                break
        if found:
            recoverable += 1

    rate = recoverable / len(causal_chains)
    target = 0.9
    return MetricResult(
        name=f"root_cause_recoverability_at_{k}",
        dimension=EvalDimension.OBSERVABILITY,
        value=rate,
        target=target,
        passed=rate >= target,
        detail=f"{recoverable}/{len(causal_chains)} chains have true cause in first {k} steps.",
    )
