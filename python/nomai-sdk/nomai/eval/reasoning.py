"""G-Eval scene scoring and multi-hop spatial reasoning metrics (Tier 3).

Provides LLM-as-judge evaluation of scene description quality via the
G-Eval framework, and multi-hop spatial reasoning question generation
with accuracy measurement.

Metrics:
- ``geval_{criterion}``: LLM-judged 1-5 score normalized to 0.0-1.0 for
  completeness, clarity, spatial_accuracy, and actionability.
- ``multihop_spatial_accuracy``: Fraction of multi-hop spatial questions
  the LLM answers correctly given the scene summary.
"""

from __future__ import annotations

import logging
import math
import re
from dataclasses import dataclass
from typing import Self

from nomai.eval.llm_client import LLMClient
from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.scene import SceneSnapshot

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# G-Eval criteria
# ---------------------------------------------------------------------------

GEVAL_CRITERIA: dict[str, str] = {
    "completeness": (
        "Does this scene description include ALL entities, their positions, "
        "sizes, types, and velocities? Are there any missing details that "
        "would be needed to understand the full game state?"
    ),
    "clarity": (
        "Is this scene description clear and unambiguous? Could a reader "
        "reconstruct a mental picture of the game state without confusion? "
        "Is the formatting helpful for quick comprehension?"
    ),
    "spatial_accuracy": (
        "Are the spatial relationships in this description accurate and useful? "
        "Can you determine where entities are relative to each other and "
        "relative to the scene bounds? Are positions and sizes meaningful?"
    ),
    "actionability": (
        "Does this description provide enough information to decide what "
        "actions to take? Can you predict what will happen next? Does it "
        "imply what game moves are possible or advisable?"
    ),
}


# ---------------------------------------------------------------------------
# G-Eval helpers
# ---------------------------------------------------------------------------

_SCORE_RE = re.compile(r"\b([1-5])\b")

_GEVAL_SYSTEM_PROMPT = (
    "You are evaluating the quality of a game scene description. "
    "Score the description on a scale of 1 to 5 based on the given criterion. "
    "Respond with ONLY a single integer from 1 to 5."
)


def _parse_score(response: str) -> int | None:
    """Extract a 1-5 score from an LLM response.

    Returns the first integer 1-5 found, or ``None`` if no valid score.
    """
    match = _SCORE_RE.search(response)
    if match:
        return int(match.group(1))
    return None


def geval_score(
    snapshot: SceneSnapshot,
    llm_client: LLMClient,
    criterion: str,
) -> MetricResult:
    """Score a scene description on a single G-Eval criterion.

    Feeds ``snapshot.summary()`` and the criterion description to the LLM,
    which responds with a 1-5 score. The raw score is normalized to 0.0-1.0
    via ``(raw - 1) / 4.0``.

    Args:
        snapshot: The scene snapshot to evaluate.
        llm_client: LLM client for completions.
        criterion: One of the keys in ``GEVAL_CRITERIA``.

    Returns:
        MetricResult with normalized score. Always passes (tracking-only).
    """
    criterion_text = GEVAL_CRITERIA[criterion]
    scene_text = snapshot.summary()
    prompt = (
        f"Scene description:\n{scene_text}\n\n"
        f"Criterion: {criterion_text}\n\n"
        f"Score (1-5):"
    )

    response = llm_client.complete(_GEVAL_SYSTEM_PROMPT, prompt)
    raw_score = _parse_score(response)

    if raw_score is not None:
        normalized = (raw_score - 1) / 4.0
    else:
        normalized = 0.0

    return MetricResult(
        name=f"geval_{criterion}",
        dimension=EvalDimension.OBSERVABILITY,
        value=normalized,
        target=None,
        passed=True,
        detail=f"G-Eval {criterion}: raw={raw_score}, normalized={normalized:.2f}",
    )


def geval_all(
    snapshot: SceneSnapshot,
    llm_client: LLMClient,
) -> list[MetricResult]:
    """Run all G-Eval criteria and return a list of MetricResults.

    Args:
        snapshot: The scene snapshot to evaluate.
        llm_client: LLM client for completions.

    Returns:
        List of 4 MetricResults, one per criterion.
    """
    results: list[MetricResult] = []
    for criterion in GEVAL_CRITERIA:
        results.append(geval_score(snapshot, llm_client, criterion))
    return results


# ---------------------------------------------------------------------------
# Multi-hop spatial reasoning
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SpatialQuestion:
    """A spatial reasoning question with expected answer and hop count.

    Attributes:
        question: The natural-language question.
        expected_answer: The canonical correct answer (entity role name).
        reasoning_steps: Number of reasoning steps needed to answer.
    """

    question: str
    expected_answer: str
    reasoning_steps: int

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "question": self.question,
            "expected_answer": self.expected_answer,
            "reasoning_steps": self.reasoning_steps,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            question=str(data["question"]),
            expected_answer=str(data["expected_answer"]),
            reasoning_steps=int(data["reasoning_steps"]),  # type: ignore[arg-type]
        )


def _distance(a: tuple[float, float], b: tuple[float, float]) -> float:
    """Euclidean distance between two (x, y) tuples."""
    return math.sqrt((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2)


def generate_spatial_questions(snapshot: SceneSnapshot) -> list[SpatialQuestion]:
    """Generate multi-hop spatial reasoning questions from a scene snapshot.

    Produces three types of questions:
    1. Distance comparison (requires >= 3 entities with positions)
    2. Closest to boundary (requires >= 1 entity with position)
    3. Furthest from center (requires >= 2 entities with positions)

    Args:
        snapshot: The scene snapshot to generate questions from.

    Returns:
        List of ``SpatialQuestion`` instances.
    """
    questions: list[SpatialQuestion] = []
    entities_with_pos = [e for e in snapshot.entities if e.position is not None]

    # 1. Distance comparison — all valid (i, j, k) triples where i < j < k.
    if len(entities_with_pos) >= 3:
        for i in range(len(entities_with_pos)):
            for j in range(i + 1, len(entities_with_pos)):
                for k in range(j + 1, len(entities_with_pos)):
                    a = entities_with_pos[i]
                    b = entities_with_pos[j]
                    c = entities_with_pos[k]
                    assert a.position is not None
                    assert b.position is not None
                    assert c.position is not None

                    d_ab = _distance(a.position, b.position)
                    d_ac = _distance(a.position, c.position)
                    if abs(d_ab - d_ac) < 1.0:
                        continue  # skip ties
                    if d_ab < d_ac:
                        answer = b.role
                    else:
                        answer = c.role
                    questions.append(
                        SpatialQuestion(
                            question=f"Is the {a.role} closer to the {b.role} or the {c.role}?",
                            expected_answer=answer,
                            reasoning_steps=2,
                        )
                    )

    # 2. Closest to boundary.
    if len(entities_with_pos) >= 1:
        # Closest to top wall (min y).
        top_entity = min(entities_with_pos, key=lambda e: e.position[1])  # type: ignore[index]
        questions.append(
            SpatialQuestion(
                question="Which entity is closest to the top wall?",
                expected_answer=top_entity.role,
                reasoning_steps=1,
            )
        )

        # Closest to right wall (min distance to max_x).
        max_x = snapshot.bounds.max_x
        right_entity = min(
            entities_with_pos,
            key=lambda e: max_x - e.position[0],  # type: ignore[index]
        )
        questions.append(
            SpatialQuestion(
                question="Which entity is closest to the right wall?",
                expected_answer=right_entity.role,
                reasoning_steps=1,
            )
        )

    # 3. Furthest from center.
    if len(entities_with_pos) >= 2:
        center_x = (snapshot.bounds.min_x + snapshot.bounds.max_x) / 2.0
        center_y = (snapshot.bounds.min_y + snapshot.bounds.max_y) / 2.0
        center = (center_x, center_y)
        furthest = max(
            entities_with_pos,
            key=lambda e: _distance(e.position, center),  # type: ignore[arg-type]
        )
        questions.append(
            SpatialQuestion(
                question="Which entity is furthest from the center of the scene?",
                expected_answer=furthest.role,
                reasoning_steps=2,
            )
        )

    return questions


# ---------------------------------------------------------------------------
# Multi-hop spatial accuracy metric
# ---------------------------------------------------------------------------

_SPATIAL_SYSTEM_PROMPT = (
    "You are answering spatial reasoning questions about a game scene. "
    "Answer with ONLY the entity name or role. Do not explain."
)


def multihop_spatial_accuracy(
    snapshot: SceneSnapshot,
    questions: list[SpatialQuestion],
    llm_client: LLMClient,
) -> MetricResult:
    """Evaluate LLM multi-hop spatial reasoning accuracy.

    Feeds ``snapshot.summary()`` and each spatial question to the LLM,
    then checks whether the expected answer appears in the response.

    Args:
        snapshot: The scene snapshot to describe to the LLM.
        questions: Pre-generated spatial questions with expected answers.
        llm_client: LLM client (real or mock) for completions.

    Returns:
        MetricResult with accuracy (0.0-1.0). Always passes (tracking-only).
        Empty questions list is vacuously correct (1.0).
    """
    if not questions:
        return MetricResult(
            name="multihop_spatial_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=None,
            passed=True,
            detail="No questions provided (vacuously correct).",
        )

    scene_text = snapshot.summary()
    correct = 0
    total = len(questions)

    for q in questions:
        prompt = f"Scene:\n{scene_text}\n\nQuestion: {q.question}"
        answer = llm_client.complete(_SPATIAL_SYSTEM_PROMPT, prompt)
        if q.expected_answer.lower() in answer.lower():
            correct += 1

    accuracy = correct / total
    return MetricResult(
        name="multihop_spatial_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=None,
        passed=True,
        detail=f"{correct}/{total} spatial questions answered correctly.",
    )
