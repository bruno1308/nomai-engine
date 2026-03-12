"""Action prediction accuracy metric (Tier 2).

Evaluates an LLM's ability to predict the next game state given the current
scene snapshot and game rules.  This measures how well the engine's text
representation conveys physics and game logic to AI.

Metric:
- ``action_prediction_accuracy``: Fraction of prediction cases where the LLM
  correctly predicts all entity positions within a configurable tolerance.
"""

from __future__ import annotations

import json
import logging
import re
from dataclasses import dataclass
from typing import Self

from nomai.eval.llm_client import LLMClient
from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.scene import SceneSnapshot

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# PredictionCase data type
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class PredictionCase:
    """A single prediction test case.

    Attributes:
        current: The current scene snapshot.
        next_state: The expected scene snapshot after one tick.
        game_rules: Natural-language description of the game rules.
    """

    current: SceneSnapshot
    next_state: SceneSnapshot
    game_rules: str

    def to_dict(self) -> dict[str, object]:
        return {
            "current": self.current.to_dict(),
            "next_state": self.next_state.to_dict(),
            "game_rules": self.game_rules,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        return cls(
            current=SceneSnapshot.from_dict(data["current"]),  # type: ignore[arg-type]
            next_state=SceneSnapshot.from_dict(data["next_state"]),  # type: ignore[arg-type]
            game_rules=str(data["game_rules"]),
        )


# ---------------------------------------------------------------------------
# Ground truth extraction
# ---------------------------------------------------------------------------


def _build_ground_truth(snapshot: SceneSnapshot) -> dict[str, float]:
    """Extract ground-truth positions keyed by entity_id + axis."""
    truth: dict[str, float] = {}
    for e in snapshot.entities:
        if e.position is not None:
            truth[f"{e.entity_id}_x"] = e.position[0]
            truth[f"{e.entity_id}_y"] = e.position[1]
    return truth


# ---------------------------------------------------------------------------
# System prompt
# ---------------------------------------------------------------------------

_PREDICTION_SYSTEM = (
    "You are predicting the next game state. Given the current scene and game rules, "
    "predict the positions of key entities after one tick. "
    "Respond with ONLY a JSON object mapping entity_id_axis to predicted value. "
    'Example: {"1_x": 400.0, "1_y": 560.0, "2_x": 403.3, "2_y": 295.0}'
)


# ---------------------------------------------------------------------------
# Response parsing
# ---------------------------------------------------------------------------

_FENCE_RE = re.compile(r"```(?:json)?\s*(.*?)\s*```", re.DOTALL)


def _extract_predictions(response: str) -> dict[str, float] | None:
    """Parse JSON predictions from an LLM response.

    Handles responses wrapped in markdown code fences.  Returns ``None``
    if the response cannot be parsed as a JSON object mapping strings
    to numbers.
    """
    text = response.strip()

    # Try to extract from code fence first.
    fence_match = _FENCE_RE.search(text)
    if fence_match:
        text = fence_match.group(1).strip()

    try:
        parsed = json.loads(text)
    except (json.JSONDecodeError, ValueError):
        logger.debug("Failed to parse prediction JSON: %s", response[:200])
        return None

    if not isinstance(parsed, dict):
        return None

    result: dict[str, float] = {}
    for key, value in parsed.items():
        try:
            result[str(key)] = float(value)
        except (TypeError, ValueError):
            return None
    return result


# ---------------------------------------------------------------------------
# Metric
# ---------------------------------------------------------------------------


def action_prediction_accuracy(
    cases: list[PredictionCase],
    llm_client: LLMClient,
    tolerance: float = 1.0,
) -> MetricResult:
    """Evaluate LLM action-prediction accuracy on a set of prediction cases.

    For each case, feeds ``current.summary()`` and ``game_rules`` to the LLM,
    then compares each predicted position to the ground truth within
    *tolerance*.  ALL positions must be within tolerance for a case to count
    as correct.  Malformed JSON counts as an incorrect prediction.

    Args:
        cases: Prediction cases with current state and expected next state.
        llm_client: LLM client (real or mock) for completions.
        tolerance: Maximum allowed absolute difference per axis (default 1.0).

    Returns:
        MetricResult with accuracy (0.0-1.0).  Target >= 0.7.
        Empty cases list is vacuously correct (1.0).
    """
    if not cases:
        return MetricResult(
            name="action_prediction_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.7,
            passed=True,
            detail="No cases provided (vacuously correct).",
        )

    correct = 0
    total = len(cases)
    errors: list[str] = []

    for i, case in enumerate(cases):
        prompt = (
            f"Current scene:\n{case.current.summary()}\n\n"
            f"Game rules: {case.game_rules}\n\n"
            "Predict the positions of all entities after one tick."
        )
        response = llm_client.complete(_PREDICTION_SYSTEM, prompt)
        predictions = _extract_predictions(response)

        if predictions is None:
            errors.append(f"Case {i}: malformed JSON response")
            continue

        ground_truth = _build_ground_truth(case.next_state)
        case_correct = True
        for key, expected_val in ground_truth.items():
            predicted_val = predictions.get(key)
            if predicted_val is None or abs(predicted_val - expected_val) > tolerance:
                case_correct = False
                break

        if case_correct:
            correct += 1
        else:
            errors.append(f"Case {i}: position mismatch")

    accuracy = correct / total
    target = 0.7
    error_detail = ""
    if errors:
        error_detail = " Errors: " + "; ".join(errors[:5])
    return MetricResult(
        name="action_prediction_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=target,
        passed=accuracy >= target,
        detail=f"{correct}/{total} cases predicted correctly.{error_detail}",
    )
