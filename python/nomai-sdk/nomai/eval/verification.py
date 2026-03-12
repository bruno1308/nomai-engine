"""Verification dimension evaluation -- reasoning quality over manifest.

Answers: "Can AI verify game behavior from manifest alone?"

Metrics:
- ``intent_expressibility_coverage``: What fraction of game rules can be
  expressed as intent specs?
- ``bug_detection_precision`` / ``bug_detection_recall``: Accuracy of
  the verification engine on a seeded bug corpus.
- ``diagnosis_to_fix_success_at_k``: Can AI fix bugs within *k* attempts
  using only the verification report?
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

from nomai.eval.metrics import EvalDimension, MetricResult

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class BugCorpusResult:
    """Outcome of running verification against a single seeded bug.

    Attributes:
        bug_id: Unique identifier for the seeded bug.
        detected: Whether the verification engine flagged this bug.
        is_true_bug: Whether this entry is a genuine bug (True) or a
            clean scenario that should NOT be flagged (False).
        attempts_to_fix: Number of write-verify-fix iterations needed
            to resolve the bug.  ``-1`` if not fixed.
    """

    bug_id: str
    detected: bool
    is_true_bug: bool
    attempts_to_fix: int = -1


def intent_expressibility_coverage(
    total_rules: int,
    expressible_rules: int,
) -> MetricResult:
    """Fraction of benchmark game rules expressible as intent specs.

    Args:
        total_rules: Total rules in the benchmark suite.
        expressible_rules: Rules that can be written as ``IntentSpec``.

    Returns:
        MetricResult with coverage rate.  Target >= 0.9.
    """
    if total_rules == 0:
        return MetricResult(
            name="intent_expressibility_coverage",
            dimension=EvalDimension.VERIFICATION,
            value=1.0,
            target=0.9,
            passed=True,
            detail="No rules to evaluate (vacuously correct).",
        )

    rate = expressible_rules / total_rules
    target = 0.9
    return MetricResult(
        name="intent_expressibility_coverage",
        dimension=EvalDimension.VERIFICATION,
        value=rate,
        target=target,
        passed=rate >= target,
        detail=f"{expressible_rules}/{total_rules} rules expressible as intent specs.",
    )


def bug_detection_precision(
    results: list[BugCorpusResult],
) -> MetricResult:
    """Precision of the verification engine on a seeded bug corpus.

    Precision = true positives / (true positives + false positives).

    Args:
        results: Outcomes from running the bug corpus.

    Returns:
        MetricResult with precision.  Target >= 0.95.
    """
    true_positives = sum(1 for r in results if r.is_true_bug and r.detected)
    false_positives = sum(1 for r in results if not r.is_true_bug and r.detected)
    denominator = true_positives + false_positives

    if denominator == 0:
        return MetricResult(
            name="bug_detection_precision",
            dimension=EvalDimension.VERIFICATION,
            value=1.0,
            target=0.95,
            passed=True,
            detail="No detections made (vacuously correct).",
        )

    precision = true_positives / denominator
    target = 0.95
    return MetricResult(
        name="bug_detection_precision",
        dimension=EvalDimension.VERIFICATION,
        value=precision,
        target=target,
        passed=precision >= target,
        detail=f"Precision: {true_positives} TP, {false_positives} FP.",
    )


def bug_detection_recall(
    results: list[BugCorpusResult],
) -> MetricResult:
    """Recall of the verification engine on a seeded bug corpus.

    Recall = true positives / (true positives + false negatives).

    Args:
        results: Outcomes from running the bug corpus.

    Returns:
        MetricResult with recall.  Target >= 0.95.
    """
    true_positives = sum(1 for r in results if r.is_true_bug and r.detected)
    false_negatives = sum(1 for r in results if r.is_true_bug and not r.detected)
    denominator = true_positives + false_negatives

    if denominator == 0:
        return MetricResult(
            name="bug_detection_recall",
            dimension=EvalDimension.VERIFICATION,
            value=1.0,
            target=0.95,
            passed=True,
            detail="No true bugs in corpus (vacuously correct).",
        )

    recall = true_positives / denominator
    target = 0.95
    return MetricResult(
        name="bug_detection_recall",
        dimension=EvalDimension.VERIFICATION,
        value=recall,
        target=target,
        passed=recall >= target,
        detail=f"Recall: {true_positives} TP, {false_negatives} FN.",
    )


def diagnosis_to_fix_success_at_k(
    results: list[BugCorpusResult],
    k: int = 2,
) -> MetricResult:
    """Fraction of bugs fixed within *k* write-verify-fix attempts.

    Only considers true bugs that were detected.

    Args:
        results: Outcomes from running the bug corpus.
        k: Maximum number of fix attempts.  Default 2.

    Returns:
        MetricResult with success rate.  Target >= 0.7 at k=2.
    """
    detected_bugs = [r for r in results if r.is_true_bug and r.detected]
    if not detected_bugs:
        return MetricResult(
            name=f"diagnosis_to_fix_success_at_{k}",
            dimension=EvalDimension.VERIFICATION,
            value=1.0,
            target=0.7,
            passed=True,
            detail="No detected bugs to fix (vacuously correct).",
        )

    fixed_in_k = sum(
        1 for r in detected_bugs if 0 < r.attempts_to_fix <= k
    )
    rate = fixed_in_k / len(detected_bugs)
    target = 0.7
    return MetricResult(
        name=f"diagnosis_to_fix_success_at_{k}",
        dimension=EvalDimension.VERIFICATION,
        value=rate,
        target=target,
        passed=rate >= target,
        detail=f"{fixed_in_k}/{len(detected_bugs)} bugs fixed in <={k} attempts.",
    )
