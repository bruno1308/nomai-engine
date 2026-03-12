"""Scene QA question generation and accuracy metric (Tier 2).

Generates template-based questions from a ``SceneSnapshot`` and evaluates
an LLM's ability to answer them correctly, measuring how well the engine's
text scene representation conveys game state to AI.

Metric:
- ``scene_qa_accuracy``: Fraction of generated questions answered correctly
  by the LLM, using type-specific answer matching.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass
from typing import Self

from nomai.eval.llm_client import LLMClient
from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.scene import SceneSnapshot

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# SceneQuestion data type
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class SceneQuestion:
    """A single QA pair generated from a scene snapshot.

    Attributes:
        question: The natural-language question.
        expected_answer: The canonical correct answer.
        question_type: Category — one of count, existence, position, type,
            size, relative_position, velocity.
    """

    question: str
    expected_answer: str
    question_type: str  # count, existence, position, type, size, relative_position, velocity

    def to_dict(self) -> dict[str, str]:
        return {
            "question": self.question,
            "expected_answer": self.expected_answer,
            "question_type": self.question_type,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        return cls(
            question=str(data["question"]),
            expected_answer=str(data["expected_answer"]),
            question_type=str(data["question_type"]),
        )


# ---------------------------------------------------------------------------
# Question generation
# ---------------------------------------------------------------------------

_ABSENT_ROLE_POOL = {"enemy", "powerup", "wall", "portal"}


def _velocity_direction(vx: float, vy: float) -> str:
    """Convert a velocity vector to a human-readable direction string.

    Uses screen coordinates where lower y = higher on screen (above).
    """
    parts: list[str] = []
    if vx > 0:
        parts.append("right")
    elif vx < 0:
        parts.append("left")
    # Screen coords: negative vy means moving up.
    if vy < 0:
        parts.append("up")
    elif vy > 0:
        parts.append("down")
    if not parts:
        return "stationary"
    return " and ".join(parts)


def generate_scene_questions(snapshot: SceneSnapshot) -> list[SceneQuestion]:
    """Generate template-based QA pairs from a scene snapshot.

    Produces questions covering: count, existence (positive & negative),
    position, size, type, relative_position, and velocity.

    Args:
        snapshot: The scene snapshot to generate questions from.

    Returns:
        List of ``SceneQuestion`` instances.
    """
    questions: list[SceneQuestion] = []

    # 1. Count question.
    questions.append(
        SceneQuestion(
            question="How many entities are in the scene?",
            expected_answer=str(snapshot.entity_count),
            question_type="count",
        )
    )

    # Collect unique roles.
    seen_roles: set[str] = set()
    for entity in snapshot.entities:
        seen_roles.add(entity.role)

    # 2. Existence (positive) — one per unique role.
    for role in sorted(seen_roles):
        questions.append(
            SceneQuestion(
                question=f"Is there a {role} in the scene?",
                expected_answer="yes",
                question_type="existence",
            )
        )

    # 3. Existence (negative) — pick from absent roles.
    absent_roles = _ABSENT_ROLE_POOL - seen_roles
    for role in sorted(absent_roles):
        questions.append(
            SceneQuestion(
                question=f"Is there a {role} in the scene?",
                expected_answer="no",
                question_type="existence",
            )
        )

    # Per-entity questions.
    for entity in snapshot.entities:
        # 4. Position question.
        if entity.position is not None:
            questions.append(
                SceneQuestion(
                    question=f"What is the approximate x-position of the {entity.role}?",
                    expected_answer=str(round(entity.position[0])),
                    question_type="position",
                )
            )

        # 5. Size question.
        if entity.size is not None:
            questions.append(
                SceneQuestion(
                    question=f"What is the width of the {entity.role}?",
                    expected_answer=str(round(entity.size[0])),
                    question_type="size",
                )
            )

        # 6. Type question.
        questions.append(
            SceneQuestion(
                question=f"What type of entity is the {entity.role}?",
                expected_answer=entity.entity_type,
                question_type="type",
            )
        )

        # 8. Velocity question.
        if entity.velocity is not None:
            direction = _velocity_direction(entity.velocity[0], entity.velocity[1])
            questions.append(
                SceneQuestion(
                    question=f"What direction is the {entity.role} moving?",
                    expected_answer=direction,
                    question_type="velocity",
                )
            )

    # 7. Relative position — pairwise, only if |y diff| > 10.0.
    entities_with_pos = [e for e in snapshot.entities if e.position is not None]
    for i, a in enumerate(entities_with_pos):
        for b in entities_with_pos[i + 1 :]:
            assert a.position is not None  # for type checker
            assert b.position is not None
            y_diff = a.position[1] - b.position[1]
            if abs(y_diff) > 10.0:
                # Lower y = above in screen coords.
                if y_diff < 0:
                    answer = "above"
                else:
                    answer = "below"
                questions.append(
                    SceneQuestion(
                        question=(
                            f"Is the {a.role} above or below the {b.role}?"
                        ),
                        expected_answer=answer,
                        question_type="relative_position",
                    )
                )

    return questions


# ---------------------------------------------------------------------------
# Answer matching
# ---------------------------------------------------------------------------

_NUMBER_RE = re.compile(r"-?\d+(?:\.\d+)?")


def _answer_matches(expected: str, actual: str, question_type: str) -> bool:
    """Check whether *actual* matches *expected* using type-specific logic.

    Args:
        expected: The canonical expected answer.
        actual: The LLM's response text.
        question_type: The question category, controlling match strategy.

    Returns:
        ``True`` if the answer is considered correct.
    """
    expected_lower = expected.strip().lower()
    actual_lower = actual.strip().lower()

    if question_type in ("count", "position", "size"):
        # Extract numbers and compare with tolerance.
        expected_nums = _NUMBER_RE.findall(expected_lower)
        actual_nums = _NUMBER_RE.findall(actual_lower)
        if not expected_nums:
            return expected_lower == actual_lower
        exp_val = float(expected_nums[0])
        tolerance = 5.0 if question_type == "position" else 1.0
        for num_str in actual_nums:
            if abs(float(num_str) - exp_val) <= tolerance:
                return True
        return False

    if question_type == "existence":
        # Determine polarity of actual answer.
        has_negative = bool(re.search(r"\bno\b|\bnot\b|\bnone\b", actual_lower))
        has_positive = bool(re.search(r"\byes\b", actual_lower))
        expected_is_yes = expected_lower == "yes"
        if expected_is_yes:
            # Must contain "yes" and not be negated, OR at least not negative.
            return has_positive and not has_negative
        else:
            return has_negative

    if question_type in ("type", "velocity", "relative_position"):
        # Word-boundary regex match.
        pattern = r"\b" + re.escape(expected_lower) + r"\b"
        return bool(re.search(pattern, actual_lower))

    # Fallback: exact match (case-insensitive).
    return expected_lower == actual_lower


# ---------------------------------------------------------------------------
# Scene QA accuracy metric
# ---------------------------------------------------------------------------

_QA_SYSTEM_PROMPT = (
    "You are evaluating a game engine's text scene representation. "
    "Answer the question about the scene based ONLY on the text provided. "
    "Be concise — answer with just the relevant value or short phrase."
)


def scene_qa_accuracy(
    snapshot: SceneSnapshot,
    questions: list[SceneQuestion],
    llm_client: LLMClient,
) -> MetricResult:
    """Evaluate LLM QA accuracy on a scene snapshot.

    Feeds ``snapshot.summary()`` and each question to the LLM, then
    compares responses against expected answers using type-specific
    matching.

    Args:
        snapshot: The scene snapshot to describe to the LLM.
        questions: Pre-generated questions with expected answers.
        llm_client: LLM client (real or mock) for completions.

    Returns:
        MetricResult with accuracy (0.0-1.0).  Target >= 0.8.
        Empty questions list is vacuously correct (1.0).
    """
    if not questions:
        return MetricResult(
            name="scene_qa_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.8,
            passed=True,
            detail="No questions provided (vacuously correct).",
        )

    scene_text = snapshot.summary()
    correct = 0
    total = len(questions)
    mismatches: list[str] = []

    for q in questions:
        prompt = f"Scene:\n{scene_text}\n\nQuestion: {q.question}"
        answer = llm_client.complete(_QA_SYSTEM_PROMPT, prompt)
        if _answer_matches(q.expected_answer, answer, q.question_type):
            correct += 1
        else:
            mismatches.append(
                f"[{q.question_type}] expected={q.expected_answer!r}, got={answer!r}"
            )

    accuracy = correct / total
    target = 0.8
    mismatch_detail = ""
    if mismatches:
        mismatch_detail = " Mismatches: " + "; ".join(mismatches[:5])
    return MetricResult(
        name="scene_qa_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=target,
        passed=accuracy >= target,
        detail=f"{correct}/{total} questions answered correctly.{mismatch_detail}",
    )
