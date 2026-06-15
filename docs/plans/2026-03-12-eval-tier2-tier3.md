# Eval Tier 2 & 3 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add LLM-based evaluation metrics that test whether the scene snapshot text representation actually enables AI reasoning — going beyond Tier 1's structural self-consistency checks.

**Architecture:** An `LLMClient` abstraction enables deterministic testing via `MockLLMClient` while supporting real LLM backends (Anthropic, OpenAI) for actual evaluation. Tier 2 metrics use template-generated questions with ground-truth answers evaluated by LLM. Tier 3 metrics use LLM-as-judge scoring and multi-hop spatial reasoning. All metrics produce `MetricResult` under the `OBSERVABILITY` dimension and integrate with the existing `EvalRunner`.

**Tech Stack:** Python 3.12, frozen dataclasses, pytest. Optional: `anthropic` SDK for real eval runs. No required external dependencies — `MockLLMClient` makes all tests self-contained.

---

## Context

**Why Tier 2 & 3 exist:** Tier 1 metrics score 1.0 because they compare the scene snapshot against the same ECS state that produced it — that's self-consistency, not proof of utility. Tier 2 asks "can an AI answer questions about the game by reading the scene text?" and Tier 3 asks "can an AI reason spatially and judge text quality?"

**Text representation under test:** `SceneSnapshot.summary()` produces human-readable text like:
```
Scene @ tick 1 (t=0.017s)
  Entities: 2
  Bounds: (0,0) to (800,600)
  [1] paddle (character) @ (400.0,560.0) 100x15
  [2] ball (projectile) @ (400.0,300.0) none v=(200.0,-300.0)
```

**Existing patterns to follow:**
- Frozen dataclasses with `to_dict()`/`from_dict()` serialization
- Metric functions return `MetricResult` with `EvalDimension.OBSERVABILITY`
- Helper functions for test data (no fixtures in eval tests)
- Tests in `tests/test_eval.py` pattern (classes grouping related tests)

**Key files:**
- `python/nomai-sdk/nomai/eval/metrics.py` — `MetricResult`, `EvalDimension`
- `python/nomai-sdk/nomai/eval/observability.py` — existing Tier 1 metrics
- `python/nomai-sdk/nomai/eval/runner.py` — `EvalRunner` orchestrator
- `python/nomai-sdk/nomai/scene.py` — `SceneSnapshot`, `SceneEntity`
- `python/nomai-sdk/nomai/manifest.py` — `EntityEntry` for ground truth
- `python/nomai-sdk/tests/test_eval.py` — existing eval tests
- `run_eval_baseline.py` — end-to-end eval script

---

### Task 1: LLM Client Abstraction

**Files:**
- Create: `python/nomai-sdk/nomai/eval/llm_client.py`
- Test: `python/nomai-sdk/tests/test_eval_llm.py`

**Step 1: Write the failing test**

Create `python/nomai-sdk/tests/test_eval_llm.py`:

```python
"""Tests for LLM client abstraction."""

from nomai.eval.llm_client import LLMClient, MockLLMClient


class TestMockLLMClient:
    def test_returns_configured_response(self):
        client = MockLLMClient(responses=["42"])
        result = client.complete("system prompt", "What is 6*7?")
        assert result == "42"

    def test_cycles_through_responses(self):
        client = MockLLMClient(responses=["first", "second", "third"])
        assert client.complete("sys", "q1") == "first"
        assert client.complete("sys", "q2") == "second"
        assert client.complete("sys", "q3") == "third"
        # Cycles back
        assert client.complete("sys", "q4") == "first"

    def test_strict_mode_raises_on_exhaustion(self):
        client = MockLLMClient(responses=["only one"], strict=True)
        client.complete("sys", "q1")
        try:
            client.complete("sys", "q2")
            assert False, "Should have raised"
        except IndexError:
            pass

    def test_records_call_history(self):
        client = MockLLMClient(responses=["yes"])
        client.complete("You are a judge.", "Is the sky blue?")
        assert len(client.history) == 1
        assert client.history[0] == ("You are a judge.", "Is the sky blue?")

    def test_default_response(self):
        client = MockLLMClient()
        result = client.complete("sys", "question")
        assert isinstance(result, str)
        assert len(result) > 0

    def test_isinstance_of_protocol(self):
        client = MockLLMClient()
        assert isinstance(client, LLMClient)
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_llm.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'nomai.eval.llm_client'`

**Step 3: Write minimal implementation**

Create `python/nomai-sdk/nomai/eval/llm_client.py`:

```python
"""LLM client abstraction for eval metrics that require language model calls.

Provides a protocol-based interface so metrics work with any LLM backend.
``MockLLMClient`` enables deterministic testing without API calls.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field

logger = logging.getLogger(__name__)


class LLMClient:
    """Base class for LLM clients used in evaluation.

    Subclass and override ``complete`` for real backends (Anthropic, OpenAI).
    """

    def complete(self, system: str, prompt: str) -> str:
        """Send a prompt to the LLM and return the response text.

        Args:
            system: System prompt setting the LLM's role/behavior.
            prompt: User prompt with the actual question.

        Returns:
            The LLM's response as a string.
        """
        raise NotImplementedError


@dataclass
class MockLLMClient(LLMClient):
    """Deterministic LLM client for testing.

    Cycles through pre-configured responses and records all calls
    for assertion in tests.  Use ``strict=True`` to raise
    ``IndexError`` when responses are exhausted (catches call-count
    drift in tests).
    """

    responses: list[str] = field(default_factory=lambda: ["mock response"])
    history: list[tuple[str, str]] = field(default_factory=list)
    strict: bool = False
    _call_index: int = field(default=0, repr=False)

    def complete(self, system: str, prompt: str) -> str:
        self.history.append((system, prompt))
        if self.strict and self._call_index >= len(self.responses):
            raise IndexError(
                f"MockLLMClient exhausted: {self._call_index} calls but only "
                f"{len(self.responses)} responses configured"
            )
        response = self.responses[self._call_index % len(self.responses)]
        self._call_index += 1
        return response
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_llm.py -v`
Expected: PASS (5/5)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/llm_client.py python/nomai-sdk/tests/test_eval_llm.py
git commit -m "feat(eval): add LLM client abstraction with mock for testing"
```

---

### Task 2: Scene QA Question Generation (Tier 2)

**Files:**
- Create: `python/nomai-sdk/nomai/eval/scene_qa.py`
- Test: `python/nomai-sdk/tests/test_eval_scene_qa.py`

**Step 1: Write the failing test**

Create `python/nomai-sdk/tests/test_eval_scene_qa.py`:

```python
"""Tests for Tier 2: Scene QA evaluation."""

from nomai.eval.scene_qa import SceneQuestion, generate_scene_questions
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _make_snapshot() -> SceneSnapshot:
    """Create a minimal breakout snapshot for testing."""
    return SceneSnapshot(
        schema_version=1,
        tick=10,
        sim_time=0.167,
        entities=[
            SceneEntity(
                entity_id=1, entity_type="character", role="paddle",
                tier="Semantic", position=(400.0, 560.0), size=(100.0, 15.0),
                velocity=None, visible=True, z_index=1.0,
            ),
            SceneEntity(
                entity_id=2, entity_type="projectile", role="ball",
                tier="Semantic", position=(350.0, 300.0), size=(10.0, 10.0),
                velocity=(200.0, -300.0), visible=True, z_index=2.0,
            ),
            SceneEntity(
                entity_id=3, entity_type="obstacle", role="brick",
                tier="Pooled", position=(100.0, 50.0), size=(60.0, 20.0),
                velocity=None, visible=True, z_index=0.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=3,
    )


class TestSceneQuestion:
    def test_creation(self):
        q = SceneQuestion(
            question="How many entities?",
            expected_answer="3",
            question_type="count",
        )
        assert q.question_type == "count"
        assert q.expected_answer == "3"

    def test_round_trip(self):
        q = SceneQuestion(
            question="How many entities?",
            expected_answer="3",
            question_type="count",
        )
        d = q.to_dict()
        q2 = SceneQuestion.from_dict(d)
        assert q2 == q


class TestGenerateSceneQuestions:
    def test_generates_questions(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        assert len(questions) >= 5

    def test_includes_count_question(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        count_qs = [q for q in questions if q.question_type == "count"]
        assert len(count_qs) >= 1
        assert count_qs[0].expected_answer == "3"

    def test_includes_existence_question(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        exist_qs = [q for q in questions if q.question_type == "existence"]
        assert len(exist_qs) >= 1

    def test_includes_position_question(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        pos_qs = [q for q in questions if q.question_type == "position"]
        assert len(pos_qs) >= 1

    def test_includes_relative_position_question(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        rel_qs = [q for q in questions if q.question_type == "relative_position"]
        assert len(rel_qs) >= 1

    def test_includes_velocity_question_when_entities_have_velocity(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        vel_qs = [q for q in questions if q.question_type == "velocity"]
        assert len(vel_qs) >= 1

    def test_no_velocity_question_when_no_velocities(self):
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=1,
        )
        questions = generate_scene_questions(snap)
        vel_qs = [q for q in questions if q.question_type == "velocity"]
        assert len(vel_qs) == 0
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_scene_qa.py -v`
Expected: FAIL with `ModuleNotFoundError`

**Step 3: Write minimal implementation**

Create `python/nomai-sdk/nomai/eval/scene_qa.py`:

```python
"""Tier 2: Scene QA evaluation — task-based sufficiency.

Tests whether AI can answer factual questions about the game scene
by reading only the text representation (``SceneSnapshot.summary()``).

Question generation is template-based (deterministic, zero LLM cost).
Answer evaluation uses an ``LLMClient`` to test comprehension.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Self

from nomai.eval.llm_client import LLMClient
from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.scene import SceneSnapshot

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class SceneQuestion:
    """A question about the scene with a known ground-truth answer."""

    question: str
    expected_answer: str
    question_type: str  # count, existence, position, type, relative_position, velocity

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


def generate_scene_questions(snapshot: SceneSnapshot) -> list[SceneQuestion]:
    """Generate factual questions from a scene snapshot.

    All questions have deterministic ground-truth answers computed
    directly from the snapshot data.

    Args:
        snapshot: The scene snapshot to generate questions about.

    Returns:
        List of SceneQuestion with ground-truth answers.
    """
    questions: list[SceneQuestion] = []

    # Count question
    questions.append(SceneQuestion(
        question="How many entities are in the scene?",
        expected_answer=str(snapshot.entity_count),
        question_type="count",
    ))

    # Existence questions (one per entity role)
    seen_roles: set[str] = set()
    for e in snapshot.entities:
        if e.role not in seen_roles:
            seen_roles.add(e.role)
            questions.append(SceneQuestion(
                question=f"Is there a {e.role} in the scene?",
                expected_answer="yes",
                question_type="existence",
            ))

    # Negative existence question (entity that doesn't exist)
    _absent = {"enemy", "powerup", "wall", "portal"} - seen_roles
    if _absent:
        absent_role = sorted(_absent)[0]
        questions.append(SceneQuestion(
            question=f"Is there a {absent_role} in the scene?",
            expected_answer="no",
            question_type="existence",
        ))

    # Position questions (for entities with positions)
    entities_with_pos = [e for e in snapshot.entities if e.position is not None]
    for e in entities_with_pos:
        assert e.position is not None
        questions.append(SceneQuestion(
            question=f"What is the approximate x-position of the {e.role}?",
            expected_answer=str(round(e.position[0])),
            question_type="position",
        ))

    # Size questions (for entities with size)
    entities_with_size = [e for e in snapshot.entities if e.size is not None]
    for e in entities_with_size:
        assert e.size is not None
        questions.append(SceneQuestion(
            question=f"What is the width of the {e.role}?",
            expected_answer=str(round(e.size[0])),
            question_type="size",
        ))

    # Type questions
    for e in snapshot.entities:
        questions.append(SceneQuestion(
            question=f"What type of entity is the {e.role}?",
            expected_answer=e.entity_type,
            question_type="type",
        ))

    # Relative position questions (pairwise for entities with positions)
    if len(entities_with_pos) >= 2:
        for i, a in enumerate(entities_with_pos):
            for b in entities_with_pos[i + 1:]:
                assert a.position is not None and b.position is not None
                if abs(a.position[1] - b.position[1]) > 10.0:
                    above = a.role if a.position[1] < b.position[1] else b.role
                    below = b.role if above == a.role else a.role
                    questions.append(SceneQuestion(
                        question=f"Is the {a.role} above or below the {b.role}?",
                        expected_answer=f"The {above} is above the {below}.",
                        question_type="relative_position",
                    ))

    # Velocity questions
    entities_with_vel = [e for e in snapshot.entities if e.velocity is not None]
    for e in entities_with_vel:
        assert e.velocity is not None
        dx, dy = e.velocity
        parts = []
        if abs(dx) > 0.1:
            parts.append("right" if dx > 0 else "left")
        if abs(dy) > 0.1:
            parts.append("down" if dy > 0 else "up")
        direction = " and ".join(parts) if parts else "stationary"
        questions.append(SceneQuestion(
            question=f"What direction is the {e.role} moving?",
            expected_answer=direction,
            question_type="velocity",
        ))

    return questions
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_scene_qa.py -v`
Expected: PASS (all tests)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/scene_qa.py python/nomai-sdk/tests/test_eval_scene_qa.py
git commit -m "feat(eval): add Tier 2 scene QA question generation"
```

---

### Task 3: Scene QA Evaluation Metric (Tier 2)

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/scene_qa.py`
- Modify: `python/nomai-sdk/tests/test_eval_scene_qa.py`

**Step 1: Write the failing tests**

Add to `tests/test_eval_scene_qa.py`:

```python
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.eval.scene_qa import scene_qa_accuracy


class TestSceneQAAccuracy:
    def test_perfect_score_when_all_correct(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        # Mock returns the exact expected answer for each question
        client = MockLLMClient(responses=[q.expected_answer for q in questions])
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 1.0
        assert result.passed is True
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_zero_score_when_all_wrong(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        client = MockLLMClient(responses=["wrong answer"])
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 0.0
        assert result.passed is False

    def test_partial_score(self):
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        # First answer correct, rest wrong
        responses = [questions[0].expected_answer] + ["wrong"] * (len(questions) - 1)
        client = MockLLMClient(responses=responses)
        result = scene_qa_accuracy(snap, questions, client)
        assert 0.0 < result.value < 1.0

    def test_empty_questions_vacuously_correct(self):
        snap = _make_snapshot()
        client = MockLLMClient()
        result = scene_qa_accuracy(snap, [], client)
        assert result.value == 1.0
        assert result.passed is True

    def test_llm_receives_scene_text(self):
        snap = _make_snapshot()
        questions = [SceneQuestion("How many?", "3", "count")]
        client = MockLLMClient(responses=["3"])
        scene_qa_accuracy(snap, questions, client)
        assert len(client.history) == 1
        _, prompt = client.history[0]
        assert "paddle" in prompt  # Scene text included
        assert "How many?" in prompt  # Question included

    def test_answer_matching_is_case_insensitive(self):
        snap = _make_snapshot()
        questions = [SceneQuestion("Is there a paddle?", "yes", "existence")]
        client = MockLLMClient(responses=["Yes, there is a paddle."])
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 1.0

    def test_numeric_matching_extracts_numbers(self):
        """LLM may wrap answer in a sentence."""
        snap = _make_snapshot()
        questions = [SceneQuestion("How many entities?", "3", "count")]
        client = MockLLMClient(responses=["There are 3 entities in the scene."])
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 1.0

    def test_no_false_positive_on_substring(self):
        """'1' should not match '10'."""
        snap = _make_snapshot()
        questions = [SceneQuestion("How many entities?", "1", "count")]
        client = MockLLMClient(responses=["There are 10 entities."])
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 0.0
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_scene_qa.py::TestSceneQAAccuracy -v`
Expected: FAIL with `ImportError: cannot import name 'scene_qa_accuracy'`

**Step 3: Write minimal implementation**

Add to `python/nomai-sdk/nomai/eval/scene_qa.py`:

```python
_QA_SYSTEM_PROMPT = (
    "You are evaluating a game engine's text scene representation. "
    "Answer the question about the scene based ONLY on the text provided. "
    "Be concise — answer with just the relevant value or short phrase."
)


def _answer_matches(expected: str, actual: str, question_type: str) -> bool:
    """Check if LLM's answer matches the expected answer.

    Uses question-type-specific matching to avoid false positives
    (e.g. "1" matching "10", "ball" matching "wall").
    """
    expected_lower = expected.lower().strip()
    actual_lower = actual.lower().strip()

    # Numeric types: extract numbers and compare with tolerance
    if question_type in ("count", "position", "size"):
        import re
        expected_nums = re.findall(r'-?\d+\.?\d*', expected_lower)
        actual_nums = re.findall(r'-?\d+\.?\d*', actual_lower)
        if not expected_nums or not actual_nums:
            return False
        try:
            exp_val = float(expected_nums[0])
            act_val = float(actual_nums[0])
            tolerance = 5.0 if question_type == "position" else 1.0
            return abs(exp_val - act_val) <= tolerance
        except ValueError:
            return False

    # Yes/no questions: check for affirmative/negative
    if question_type == "existence":
        yes_words = {"yes", "true", "correct", "there is", "present"}
        no_words = {"no", "false", "not", "there is no", "absent", "none"}
        if expected_lower in ("yes", "true"):
            return any(w in actual_lower for w in yes_words) and not any(
                w in actual_lower for w in no_words
            )
        else:
            return any(w in actual_lower for w in no_words)

    # String types (type, velocity, relative_position): word-boundary match
    if question_type in ("type", "velocity", "relative_position"):
        import re
        # Use word boundary to avoid "ball" matching "wall"
        pattern = r'\b' + re.escape(expected_lower) + r'\b'
        return bool(re.search(pattern, actual_lower))

    # Fallback: exact match
    return expected_lower == actual_lower


def scene_qa_accuracy(
    snapshot: SceneSnapshot,
    questions: list[SceneQuestion],
    llm_client: LLMClient,
) -> MetricResult:
    """Evaluate LLM comprehension of scene text via Q&A.

    Feeds each question plus the scene snapshot summary to the LLM,
    then compares the LLM's answer against the ground-truth expected
    answer.

    Args:
        snapshot: The scene snapshot under evaluation.
        questions: Questions with ground-truth answers.
        llm_client: LLM client for answer generation.

    Returns:
        MetricResult with accuracy (0.0-1.0).  Target >= 0.8.
    """
    if not questions:
        return MetricResult(
            name="scene_qa_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.8,
            passed=True,
            detail="No questions to evaluate (vacuously correct).",
        )

    scene_text = snapshot.summary()
    correct = 0
    for q in questions:
        prompt = f"Scene description:\n{scene_text}\n\nQuestion: {q.question}"
        answer = llm_client.complete(_QA_SYSTEM_PROMPT, prompt)
        if _answer_matches(q.expected_answer, answer, q.question_type):
            correct += 1
        else:
            logger.debug(
                "QA miss: %s expected=%r got=%r", q.question, q.expected_answer, answer
            )

    accuracy = correct / len(questions)
    target = 0.8
    return MetricResult(
        name="scene_qa_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=target,
        passed=accuracy >= target,
        detail=f"{correct}/{len(questions)} questions answered correctly.",
    )
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_scene_qa.py -v`
Expected: PASS (all tests)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/scene_qa.py python/nomai-sdk/tests/test_eval_scene_qa.py
git commit -m "feat(eval): add scene_qa_accuracy metric (Tier 2)"
```

---

### Task 4: Action Prediction Metric (Tier 2)

**Files:**
- Create: `python/nomai-sdk/nomai/eval/action_prediction.py`
- Create: `python/nomai-sdk/tests/test_eval_action_prediction.py`

**Step 1: Write the failing test**

Create `python/nomai-sdk/tests/test_eval_action_prediction.py`:

```python
"""Tests for Tier 2: Action prediction evaluation."""

from nomai.eval.action_prediction import (
    PredictionCase,
    action_prediction_accuracy,
)
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _snap(tick: int, ball_x: float, ball_y: float) -> SceneSnapshot:
    return SceneSnapshot(
        schema_version=1, tick=tick, sim_time=tick / 60.0,
        entities=[
            SceneEntity(
                entity_id=1, entity_type="character", role="paddle",
                tier="Semantic", position=(400.0, 560.0), size=(100.0, 15.0),
                velocity=None, visible=True, z_index=1.0,
            ),
            SceneEntity(
                entity_id=2, entity_type="projectile", role="ball",
                tier="Semantic", position=(ball_x, ball_y), size=(10.0, 10.0),
                velocity=(200.0, -300.0), visible=True, z_index=2.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=2,
    )


class TestPredictionCase:
    def test_creation(self):
        case = PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball bounces off walls and paddle.",
        )
        assert case.current.tick == 1
        assert case.next_state.tick == 2

    def test_round_trip(self):
        case = PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball bounces off walls.",
        )
        d = case.to_dict()
        case2 = PredictionCase.from_dict(d)
        assert case2.current.tick == case.current.tick
        assert case2.game_rules == case.game_rules


class TestActionPredictionAccuracy:
    def test_perfect_prediction(self):
        cases = [PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball moves by velocity each tick.",
        )]
        # LLM correctly predicts positions keyed by entity_id
        client = MockLLMClient(responses=[
            '{"1_x": 400.0, "1_y": 560.0, "2_x": 403.3, "2_y": 295.0}'
        ])
        result = action_prediction_accuracy(cases, client)
        assert result.value == 1.0
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_wrong_prediction(self):
        cases = [PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball moves by velocity each tick.",
        )]
        client = MockLLMClient(responses=[
            '{"1_x": 400.0, "1_y": 560.0, "2_x": 100.0, "2_y": 100.0}'
        ])
        result = action_prediction_accuracy(cases, client)
        assert result.value == 0.0

    def test_empty_cases(self):
        client = MockLLMClient()
        result = action_prediction_accuracy([], client)
        assert result.value == 1.0
        assert result.passed is True

    def test_tolerance_for_close_predictions(self):
        """Predictions within tolerance should count as correct."""
        cases = [PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball moves by velocity each tick.",
        )]
        # Prediction within 5.0 tolerance
        client = MockLLMClient(responses=[
            '{"1_x": 400.0, "1_y": 560.0, "2_x": 405.0, "2_y": 296.0}'
        ])
        result = action_prediction_accuracy(cases, client, tolerance=5.0)
        assert result.value == 1.0

    def test_malformed_json_counts_as_wrong(self):
        cases = [PredictionCase(
            current=_snap(1, 400.0, 300.0),
            next_state=_snap(2, 403.3, 295.0),
            game_rules="Ball moves.",
        )]
        client = MockLLMClient(responses=["I don't know"])
        result = action_prediction_accuracy(cases, client)
        assert result.value == 0.0
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_action_prediction.py -v`
Expected: FAIL with `ModuleNotFoundError`

**Step 3: Write minimal implementation**

Create `python/nomai-sdk/nomai/eval/action_prediction.py`:

```python
"""Tier 2: Action prediction evaluation.

Tests whether AI can predict the next game state from the current
scene text and game rules. This measures whether the text
representation captures enough information for forward simulation.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from typing import Self

from nomai.eval.llm_client import LLMClient
from nomai.eval.metrics import EvalDimension, MetricResult
from nomai.scene import SceneSnapshot

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class PredictionCase:
    """A single action-prediction test case."""

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


_PREDICTION_SYSTEM = (
    "You are predicting the next game state. Given the current scene and game rules, "
    "predict the positions of key entities after one tick. "
    "Respond with ONLY a JSON object mapping entity_id_axis to predicted value. "
    "Example: {\"1_x\": 400.0, \"1_y\": 560.0, \"2_x\": 403.3, \"2_y\": 295.0}"
)


def _extract_predictions(response: str) -> dict[str, float] | None:
    """Parse LLM response as JSON position predictions."""
    try:
        # Try to extract JSON from response
        text = response.strip()
        # Handle markdown code fences
        if "```" in text:
            start = text.index("{")
            end = text.rindex("}") + 1
            text = text[start:end]
        return json.loads(text)
    except (json.JSONDecodeError, ValueError):
        return None


def _build_ground_truth(snapshot: SceneSnapshot) -> dict[str, float]:
    """Build ground-truth position dict from snapshot, keyed by entity_id."""
    truth: dict[str, float] = {}
    for e in snapshot.entities:
        if e.position is not None:
            truth[f"{e.entity_id}_x"] = e.position[0]
            truth[f"{e.entity_id}_y"] = e.position[1]
    return truth


def action_prediction_accuracy(
    cases: list[PredictionCase],
    llm_client: LLMClient,
    tolerance: float = 1.0,
) -> MetricResult:
    """Evaluate LLM's ability to predict next state from scene text.

    For each case, the LLM receives the current scene summary and
    game rules, then predicts entity positions for the next tick.
    Predictions within ``tolerance`` units count as correct.

    Args:
        cases: Prediction test cases with current/next snapshots.
        llm_client: LLM client for prediction generation.
        tolerance: Position tolerance for matching (default 1.0).

    Returns:
        MetricResult with accuracy (0.0-1.0).  Target >= 0.7.
    """
    if not cases:
        return MetricResult(
            name="action_prediction_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=0.7,
            passed=True,
            detail="No prediction cases (vacuously correct).",
        )

    correct = 0
    for case in cases:
        prompt = (
            f"Current scene:\n{case.current.summary()}\n\n"
            f"Game rules: {case.game_rules}\n\n"
            f"Predict the positions of all entities after one tick."
        )
        response = llm_client.complete(_PREDICTION_SYSTEM, prompt)
        predictions = _extract_predictions(response)
        if predictions is None:
            logger.debug("Malformed prediction response: %s", response[:200])
            continue

        ground_truth = _build_ground_truth(case.next_state)
        all_close = True
        for key, expected in ground_truth.items():
            predicted = predictions.get(key)
            if predicted is None or abs(predicted - expected) > tolerance:
                all_close = False
                break
        if all_close:
            correct += 1

    accuracy = correct / len(cases)
    target = 0.7
    return MetricResult(
        name="action_prediction_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=target,
        passed=accuracy >= target,
        detail=f"{correct}/{len(cases)} state predictions correct (tolerance={tolerance}).",
    )
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_action_prediction.py -v`
Expected: PASS (all tests)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/action_prediction.py python/nomai-sdk/tests/test_eval_action_prediction.py
git commit -m "feat(eval): add action_prediction_accuracy metric (Tier 2)"
```

---

### Task 5: G-Eval Scene Scoring (Tier 3)

**Files:**
- Create: `python/nomai-sdk/nomai/eval/reasoning.py`
- Create: `python/nomai-sdk/tests/test_eval_reasoning.py`

**Step 1: Write the failing test**

Create `python/nomai-sdk/tests/test_eval_reasoning.py`:

```python
"""Tests for Tier 3: Reasoning quality evaluation."""

from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.eval.reasoning import (
    GEVAL_CRITERIA,
    geval_score,
    geval_all,
)
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _make_snapshot() -> SceneSnapshot:
    return SceneSnapshot(
        schema_version=1, tick=10, sim_time=0.167,
        entities=[
            SceneEntity(
                entity_id=1, entity_type="character", role="paddle",
                tier="Semantic", position=(400.0, 560.0), size=(100.0, 15.0),
                velocity=None, visible=True, z_index=1.0,
            ),
            SceneEntity(
                entity_id=2, entity_type="projectile", role="ball",
                tier="Semantic", position=(350.0, 300.0), size=(10.0, 10.0),
                velocity=(200.0, -300.0), visible=True, z_index=2.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=2,
    )


class TestGEvalCriteria:
    def test_all_criteria_defined(self):
        assert "completeness" in GEVAL_CRITERIA
        assert "clarity" in GEVAL_CRITERIA
        assert "spatial_accuracy" in GEVAL_CRITERIA
        assert "actionability" in GEVAL_CRITERIA

    def test_criteria_have_descriptions(self):
        for name, desc in GEVAL_CRITERIA.items():
            assert len(desc) > 10, f"Criterion {name} needs a description"


class TestGEvalScore:
    def test_high_score(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 1.0  # 5/5 normalized
        assert result.name == "geval_completeness"
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_low_score(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["1"])
        result = geval_score(snap, client, "clarity")
        assert result.value == 0.0  # 1/5 normalized to 0.0

    def test_mid_score(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["3"])
        result = geval_score(snap, client, "spatial_accuracy")
        assert result.value == 0.5  # 3/5 normalized

    def test_parses_score_from_verbose_response(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["I rate this a 4 out of 5."])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.75  # 4/5 normalized

    def test_invalid_response_scores_zero(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["not a number"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.0

    def test_prompt_includes_criterion(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        geval_score(snap, client, "actionability")
        _, prompt = client.history[0]
        assert "actionability" in prompt.lower() or "actionable" in prompt.lower()


class TestGEvalAll:
    def test_returns_four_metrics(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["4"])
        results = geval_all(snap, client)
        assert len(results) == 4
        names = {r.name for r in results}
        assert names == {
            "geval_completeness", "geval_clarity",
            "geval_spatial_accuracy", "geval_actionability",
        }

    def test_all_under_observability(self):
        snap = _make_snapshot()
        client = MockLLMClient(responses=["3"])
        results = geval_all(snap, client)
        for r in results:
            assert r.dimension == EvalDimension.OBSERVABILITY
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_reasoning.py -v`
Expected: FAIL with `ModuleNotFoundError`

**Step 3: Write minimal implementation**

Create `python/nomai-sdk/nomai/eval/reasoning.py`:

```python
"""Tier 3: Reasoning quality evaluation — LLM-as-judge.

Evaluates the scene text representation quality using LLM scoring
(G-Eval) and multi-hop spatial reasoning tests.
"""

from __future__ import annotations

import logging
import re

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

_GEVAL_SYSTEM = (
    "You are a strict evaluator of game scene text descriptions. "
    "Score the description on the given criterion from 1 to 5. "
    "1 = very poor, 2 = poor, 3 = adequate, 4 = good, 5 = excellent. "
    "Respond with ONLY the integer score (1-5), nothing else."
)


def _parse_score(response: str) -> int | None:
    """Extract a 1-5 score from an LLM response."""
    # Try to find a digit 1-5 in the response
    match = re.search(r'\b([1-5])\b', response)
    if match:
        return int(match.group(1))
    return None


def geval_score(
    snapshot: SceneSnapshot,
    llm_client: LLMClient,
    criterion: str,
) -> MetricResult:
    """Score a scene snapshot on a single G-Eval criterion.

    The LLM judges the snapshot's text summary on the given criterion,
    returning a score from 1-5 which is normalized to 0.0-1.0.

    Args:
        snapshot: The scene snapshot to evaluate.
        llm_client: LLM client for scoring.
        criterion: One of the GEVAL_CRITERIA keys.

    Returns:
        MetricResult with normalized score (0.0-1.0). No target (tracking-only).
    """
    description = GEVAL_CRITERIA.get(criterion, criterion)
    scene_text = snapshot.summary()
    prompt = (
        f"Scene description to evaluate:\n{scene_text}\n\n"
        f"Criterion — {criterion}:\n{description}\n\n"
        f"Score (1-5):"
    )
    response = llm_client.complete(_GEVAL_SYSTEM, prompt)
    raw_score = _parse_score(response)

    if raw_score is None:
        logger.warning("Could not parse G-Eval score from: %s", response[:200])
        normalized = 0.0
    else:
        normalized = (raw_score - 1) / 4.0  # Map 1-5 to 0.0-1.0

    return MetricResult(
        name=f"geval_{criterion}",
        dimension=EvalDimension.OBSERVABILITY,
        value=normalized,
        target=None,  # Tracking-only, not a CI gate
        passed=True,  # Always passes (tracking metric)
        detail=f"G-Eval {criterion}: {raw_score}/5 (normalized {normalized:.2f}).",
    )


def geval_all(
    snapshot: SceneSnapshot,
    llm_client: LLMClient,
) -> list[MetricResult]:
    """Run all G-Eval criteria on a snapshot.

    Returns:
        List of 4 MetricResult objects, one per criterion.
    """
    return [
        geval_score(snapshot, llm_client, criterion)
        for criterion in GEVAL_CRITERIA
    ]
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_reasoning.py -v`
Expected: PASS (all tests)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/reasoning.py python/nomai-sdk/tests/test_eval_reasoning.py
git commit -m "feat(eval): add G-Eval scene scoring (Tier 3)"
```

---

### Task 6: Multi-hop Spatial Reasoning (Tier 3)

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/reasoning.py`
- Modify: `python/nomai-sdk/tests/test_eval_reasoning.py`

**Step 1: Write the failing tests**

Add to `tests/test_eval_reasoning.py`:

```python
import math
from nomai.eval.reasoning import (
    SpatialQuestion,
    generate_spatial_questions,
    multihop_spatial_accuracy,
)


def _three_entity_snapshot() -> SceneSnapshot:
    """Snapshot with 3 entities at known positions for spatial reasoning."""
    return SceneSnapshot(
        schema_version=1, tick=5, sim_time=0.083,
        entities=[
            SceneEntity(
                entity_id=1, entity_type="character", role="paddle",
                tier="Semantic", position=(400.0, 560.0), size=(100.0, 15.0),
                velocity=None, visible=True, z_index=1.0,
            ),
            SceneEntity(
                entity_id=2, entity_type="projectile", role="ball",
                tier="Semantic", position=(200.0, 300.0), size=(10.0, 10.0),
                velocity=(200.0, -300.0), visible=True, z_index=2.0,
            ),
            SceneEntity(
                entity_id=3, entity_type="obstacle", role="brick",
                tier="Pooled", position=(600.0, 100.0), size=(60.0, 20.0),
                velocity=None, visible=True, z_index=0.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=3,
    )


class TestSpatialQuestion:
    def test_creation(self):
        q = SpatialQuestion(
            question="Which entity is closest to the top wall?",
            expected_answer="brick",
            reasoning_steps=2,
        )
        assert q.reasoning_steps == 2

    def test_round_trip(self):
        q = SpatialQuestion(
            question="test?",
            expected_answer="answer",
            reasoning_steps=3,
        )
        d = q.to_dict()
        q2 = SpatialQuestion.from_dict(d)
        assert q2 == q


class TestGenerateSpatialQuestions:
    def test_generates_questions(self):
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        assert len(questions) >= 2

    def test_includes_distance_comparison(self):
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        dist_qs = [q for q in questions if "closer" in q.question or "closest" in q.question]
        assert len(dist_qs) >= 1

    def test_includes_boundary_question(self):
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        wall_qs = [q for q in questions if "wall" in q.question or "edge" in q.question]
        assert len(wall_qs) >= 1

    def test_needs_at_least_two_entities(self):
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=1,
        )
        questions = generate_spatial_questions(snap)
        # Can still generate boundary questions with 1 entity
        dist_qs = [q for q in questions if "closer" in q.question]
        assert len(dist_qs) == 0


class TestMultihopSpatialAccuracy:
    def test_all_correct(self):
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        client = MockLLMClient(responses=[q.expected_answer for q in questions])
        result = multihop_spatial_accuracy(snap, questions, client)
        assert result.value == 1.0
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_all_wrong(self):
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        client = MockLLMClient(responses=["completely wrong"])
        result = multihop_spatial_accuracy(snap, questions, client)
        assert result.value == 0.0

    def test_empty_questions(self):
        snap = _three_entity_snapshot()
        client = MockLLMClient()
        result = multihop_spatial_accuracy(snap, [], client)
        assert result.value == 1.0
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_reasoning.py::TestSpatialQuestion -v`
Expected: FAIL with `ImportError: cannot import name 'SpatialQuestion'`

**Step 3: Write minimal implementation**

Add to `python/nomai-sdk/nomai/eval/reasoning.py`:

```python
import math
from dataclasses import dataclass
from typing import Self


@dataclass(frozen=True)
class SpatialQuestion:
    """A multi-hop spatial reasoning question."""

    question: str
    expected_answer: str
    reasoning_steps: int  # How many reasoning hops needed

    def to_dict(self) -> dict[str, object]:
        return {
            "question": self.question,
            "expected_answer": self.expected_answer,
            "reasoning_steps": self.reasoning_steps,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        return cls(
            question=str(data["question"]),
            expected_answer=str(data["expected_answer"]),
            reasoning_steps=int(data["reasoning_steps"]),  # type: ignore[arg-type]
        )


def _distance(a: tuple[float, float], b: tuple[float, float]) -> float:
    return math.sqrt((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2)


def generate_spatial_questions(snapshot: SceneSnapshot) -> list[SpatialQuestion]:
    """Generate multi-hop spatial reasoning questions from a snapshot.

    Questions require combining multiple pieces of spatial information
    (positions, distances, bounds) to answer correctly.

    Args:
        snapshot: Scene snapshot with entity positions.

    Returns:
        List of SpatialQuestion with computed ground-truth answers.
    """
    questions: list[SpatialQuestion] = []
    entities_with_pos = [e for e in snapshot.entities if e.position is not None]

    # Distance comparison: "Is A closer to B or to C?"
    if len(entities_with_pos) >= 3:
        for i in range(len(entities_with_pos)):
            for j in range(i + 1, len(entities_with_pos)):
                for k in range(j + 1, len(entities_with_pos)):
                    a, b, c = entities_with_pos[i], entities_with_pos[j], entities_with_pos[k]
                    assert a.position and b.position and c.position
                    d_ab = _distance(a.position, b.position)
                    d_ac = _distance(a.position, c.position)
                    if abs(d_ab - d_ac) < 1.0:
                        continue  # Skip ties
                    closer = b.role if d_ab < d_ac else c.role
                    questions.append(SpatialQuestion(
                        question=f"Is the {a.role} closer to the {b.role} or the {c.role}?",
                        expected_answer=closer,
                        reasoning_steps=2,
                    ))

    # Closest to boundary: "Which entity is closest to the top wall?"
    if entities_with_pos:
        # Top wall (min_y)
        closest_top = min(entities_with_pos, key=lambda e: e.position[1])  # type: ignore[index]
        questions.append(SpatialQuestion(
            question="Which entity is closest to the top wall (top edge of the scene)?",
            expected_answer=closest_top.role,
            reasoning_steps=2,
        ))

        # Right wall (max_x)
        closest_right = min(
            entities_with_pos,
            key=lambda e: snapshot.bounds.max_x - e.position[0],  # type: ignore[index]
        )
        questions.append(SpatialQuestion(
            question="Which entity is closest to the right wall (right edge of the scene)?",
            expected_answer=closest_right.role,
            reasoning_steps=2,
        ))

    # Furthest from center
    if len(entities_with_pos) >= 2:
        cx = (snapshot.bounds.min_x + snapshot.bounds.max_x) / 2
        cy = (snapshot.bounds.min_y + snapshot.bounds.max_y) / 2
        furthest = max(
            entities_with_pos,
            key=lambda e: _distance(e.position, (cx, cy)),  # type: ignore[arg-type]
        )
        questions.append(SpatialQuestion(
            question="Which entity is furthest from the center of the scene?",
            expected_answer=furthest.role,
            reasoning_steps=2,
        ))

    return questions


_SPATIAL_SYSTEM = (
    "You are answering spatial reasoning questions about a game scene. "
    "Use the positions and coordinates provided to reason step by step. "
    "Give your final answer as just the entity name/role (e.g. 'paddle', 'ball', 'brick'). "
    "Answer with ONLY the entity name, nothing else."
)


def multihop_spatial_accuracy(
    snapshot: SceneSnapshot,
    questions: list[SpatialQuestion],
    llm_client: LLMClient,
) -> MetricResult:
    """Evaluate LLM accuracy on multi-hop spatial reasoning questions.

    Each question requires combining multiple spatial facts from the
    scene text to derive the answer.

    Args:
        snapshot: Scene snapshot providing the text representation.
        questions: Spatial questions with ground-truth answers.
        llm_client: LLM client for answer generation.

    Returns:
        MetricResult with accuracy (0.0-1.0). No target (tracking-only).
    """
    if not questions:
        return MetricResult(
            name="multihop_spatial_accuracy",
            dimension=EvalDimension.OBSERVABILITY,
            value=1.0,
            target=None,
            passed=True,
            detail="No spatial questions to evaluate (vacuously correct).",
        )

    scene_text = snapshot.summary()
    correct = 0
    for q in questions:
        prompt = f"Scene:\n{scene_text}\n\nQuestion: {q.question}"
        answer = llm_client.complete(_SPATIAL_SYSTEM, prompt)
        if q.expected_answer.lower() in answer.lower():
            correct += 1
        else:
            logger.debug(
                "Spatial miss: %s expected=%r got=%r",
                q.question, q.expected_answer, answer,
            )

    accuracy = correct / len(questions)
    return MetricResult(
        name="multihop_spatial_accuracy",
        dimension=EvalDimension.OBSERVABILITY,
        value=accuracy,
        target=None,  # Tracking-only
        passed=True,  # Always passes (tracking metric)
        detail=f"{correct}/{len(questions)} spatial questions correct.",
    )
```

**Step 4: Run test to verify it passes**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval_reasoning.py -v`
Expected: PASS (all tests)

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/reasoning.py python/nomai-sdk/tests/test_eval_reasoning.py
git commit -m "feat(eval): add multi-hop spatial reasoning (Tier 3)"
```

---

### Task 7: Wire Tier 2 & 3 into EvalRunner

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/runner.py`
- Modify: `python/nomai-sdk/nomai/eval/__init__.py`
- Modify: `python/nomai-sdk/tests/test_eval.py`

**Step 1: Write the failing test**

Add a new test class to `tests/test_eval.py`:

```python
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.scene_qa import SceneQuestion
from nomai.eval.action_prediction import PredictionCase
from nomai.eval.reasoning import SpatialQuestion


class TestEvalRunnerTier2Tier3:
    def test_run_scene_qa(self):
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=1,
        )
        questions = [SceneQuestion("How many entities?", "1", "count")]
        client = MockLLMClient(responses=["1"])
        results = EvalRunner.run_scene_qa(snap, questions, client)
        assert len(results) == 1
        assert results[0].name == "scene_qa_accuracy"

    def test_run_geval(self):
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=0,
        )
        client = MockLLMClient(responses=["4"])
        results = EvalRunner.run_geval(snap, client)
        assert len(results) == 4  # 4 G-Eval criteria

    def test_run_all_includes_tier2_when_provided(self):
        runner = EvalRunner()
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=1,
        )
        questions = [SceneQuestion("How many entities?", "1", "count")]
        client = MockLLMClient(responses=["1", "no"])
        report = runner.run_all(
            scene_snapshot=snap,
            scene_qa_questions=questions,
            llm_client=client,
        )
        # Note: G-Eval NOT run because run_geval=False (opt-in)
        metric_names = {m.name for m in report.metrics}
        assert "scene_qa_accuracy" in metric_names
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval.py::TestEvalRunnerTier2Tier3 -v`
Expected: FAIL

**Step 3: Implement the runner integration**

Modify `python/nomai-sdk/nomai/eval/runner.py` to add:

1. Import new modules at top:
```python
from nomai.eval import scene_qa as qa_mod
from nomai.eval import action_prediction as pred_mod
from nomai.eval import reasoning as reason_mod
from nomai.eval.action_prediction import PredictionCase
from nomai.eval.llm_client import LLMClient
from nomai.eval.reasoning import SpatialQuestion
from nomai.eval.scene_qa import SceneQuestion
```

2. Add static methods to `EvalRunner`:
```python
    @staticmethod
    def run_scene_qa(
        snapshot: SceneSnapshot,
        questions: list[SceneQuestion],
        llm_client: LLMClient,
    ) -> list[MetricResult]:
        """Run Tier 2 Scene QA metrics."""
        return [qa_mod.scene_qa_accuracy(snapshot, questions, llm_client)]

    @staticmethod
    def run_action_prediction(
        cases: list[PredictionCase],
        llm_client: LLMClient,
    ) -> list[MetricResult]:
        """Run Tier 2 action prediction metrics."""
        return [pred_mod.action_prediction_accuracy(cases, llm_client)]

    @staticmethod
    def run_geval(
        snapshot: SceneSnapshot,
        llm_client: LLMClient,
    ) -> list[MetricResult]:
        """Run Tier 3 G-Eval metrics."""
        return reason_mod.geval_all(snapshot, llm_client)

    @staticmethod
    def run_multihop(
        snapshot: SceneSnapshot,
        questions: list[SpatialQuestion],
        llm_client: LLMClient,
    ) -> list[MetricResult]:
        """Run Tier 3 multi-hop spatial reasoning metrics."""
        return [reason_mod.multihop_spatial_accuracy(snapshot, questions, llm_client)]
```

3. Extend `run_all()` signature with new optional params:
```python
        # Tier 2 inputs
        scene_qa_questions: list[SceneQuestion] | None = None,
        prediction_cases: list[PredictionCase] | None = None,
        # Tier 3 inputs (opt-in — expensive LLM calls)
        spatial_questions: list[SpatialQuestion] | None = None,
        run_geval: bool = False,
        # LLM client (required for Tier 2/3)
        llm_client: LLMClient | None = None,
```

4. Add Tier 2/3 sections to `run_all()` body. **CRITICAL:** these must be
   added BEFORE the observability `DimensionScore.from_metrics()` call so
   the dimension score includes Tier 2/3 metrics. Move the dimension score
   computation to after all observability metrics are collected:
```python
        # Tier 2: Scene QA (requires LLM client + snapshot)
        if llm_client is not None and scene_snapshot is not None:
            if scene_qa_questions is not None:
                qa_metrics = self.run_scene_qa(
                    scene_snapshot, scene_qa_questions, llm_client,
                )
                obs_metrics.extend(qa_metrics)

            if prediction_cases is not None:
                pred_metrics = self.run_action_prediction(
                    prediction_cases, llm_client,
                )
                obs_metrics.extend(pred_metrics)

        # Tier 3: G-Eval + spatial reasoning (opt-in, expensive)
        if run_geval and llm_client is not None and scene_snapshot is not None:
            geval_metrics = self.run_geval(scene_snapshot, llm_client)
            obs_metrics.extend(geval_metrics)

            if spatial_questions is not None:
                spatial_metrics = self.run_multihop(
                    scene_snapshot, spatial_questions, llm_client,
                )
                obs_metrics.extend(spatial_metrics)

        # NOW compute dimension score (includes Tier 1 + 2 + 3)
        all_metrics.extend(obs_metrics)
        dimension_scores["observability"] = DimensionScore.from_metrics(
            EvalDimension.OBSERVABILITY, obs_metrics,
        )
```

   Remove the original `dimension_scores["observability"]` assignment that
   currently sits right after `run_observability()` — it must move to after
   all Tier 2/3 metrics are appended to `obs_metrics`.

5. Update `python/nomai-sdk/nomai/eval/__init__.py` to export new types:
```python
from nomai.eval.llm_client import LLMClient, MockLLMClient
from nomai.eval.scene_qa import SceneQuestion, generate_scene_questions, scene_qa_accuracy
from nomai.eval.action_prediction import PredictionCase, action_prediction_accuracy
from nomai.eval.reasoning import (
    GEVAL_CRITERIA, SpatialQuestion,
    geval_score, geval_all,
    generate_spatial_questions, multihop_spatial_accuracy,
)
```

**Step 4: Run tests to verify**

Run: `cd B:/Projects/Nomai/python/nomai-sdk && python -m pytest tests/test_eval.py tests/test_eval_llm.py tests/test_eval_scene_qa.py tests/test_eval_action_prediction.py tests/test_eval_reasoning.py -v`
Expected: ALL PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/runner.py python/nomai-sdk/nomai/eval/__init__.py python/nomai-sdk/tests/test_eval.py
git commit -m "feat(eval): wire Tier 2 & 3 metrics into EvalRunner"
```

---

### Task 8: Update Baseline Script

**Files:**
- Modify: `run_eval_baseline.py`

**Step 1: Understand the integration point**

The baseline script collects data from the engine and feeds it to `EvalRunner.run_all()`. For Tier 2/3, we need to:
1. Generate scene QA questions from the snapshot
2. Create prediction cases from consecutive ticks
3. Generate spatial questions
4. Use `MockLLMClient` for the baseline (real LLM is Tier 2/3's job — baseline just proves the pipeline works)

**Step 2: Add Tier 2/3 data collection**

Add imports at top of `run_eval_baseline.py`:
```python
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.scene_qa import generate_scene_questions
from nomai.eval.action_prediction import PredictionCase
from nomai.eval.reasoning import generate_spatial_questions
```

Add to the observability collection function (or create new collection functions):
```python
def collect_tier2_tier3(engine):
    """Collect Tier 2/3 eval inputs from the engine."""
    snap = engine.scene_snapshot()

    # Scene QA questions
    from nomai.scene import SceneSnapshot
    scene_snapshot = SceneSnapshot.from_dict(snap) if isinstance(snap, dict) else snap
    questions = generate_scene_questions(scene_snapshot)

    # Prediction cases: capture current + next tick
    snap1 = engine.scene_snapshot()
    engine.tick()
    snap2 = engine.scene_snapshot()
    prediction_cases = [PredictionCase(
        current=snap1 if isinstance(snap1, SceneSnapshot) else SceneSnapshot.from_dict(snap1),
        next_state=snap2 if isinstance(snap2, SceneSnapshot) else SceneSnapshot.from_dict(snap2),
        game_rules="Ball moves by velocity each tick. Ball bounces off walls and paddle. Bricks are destroyed on collision.",
    )]

    # Spatial questions
    spatial_questions = generate_spatial_questions(scene_snapshot)

    # Mock LLM for baseline (demonstrates pipeline, not real evaluation)
    # Build prediction JSON from actual next state
    pred_snap = prediction_cases[0].next_state if prediction_cases else None
    pred_json = "{}"
    if pred_snap:
        import json as _json
        pred_dict = {}
        for e in pred_snap.entities:
            if e.position is not None:
                pred_dict[f"{e.entity_id}_x"] = e.position[0]
                pred_dict[f"{e.entity_id}_y"] = e.position[1]
        pred_json = _json.dumps(pred_dict)

    mock_answers = [q.expected_answer for q in questions]
    mock_answers.append(pred_json)  # Action prediction response
    mock_answers.extend(["4"] * 4)  # G-Eval scores
    mock_answers.extend([q.expected_answer for q in spatial_questions])
    llm_client = MockLLMClient(responses=mock_answers if mock_answers else ["mock"])

    return {
        "scene_qa_questions": questions,
        "prediction_cases": prediction_cases,
        "spatial_questions": spatial_questions,
        "llm_client": llm_client,
    }
```

Pass these to `runner.run_all()`:
```python
    tier23 = collect_tier2_tier3(engine)
    report = runner.run_all(
        # ... existing params ...
        scene_qa_questions=tier23["scene_qa_questions"],
        prediction_cases=tier23["prediction_cases"],
        spatial_questions=tier23["spatial_questions"],
        run_geval=True,  # Opt-in for baseline demo
        llm_client=tier23["llm_client"],
    )
```

**Step 3: Run baseline to verify**

Run: `cd B:/Projects/Nomai && python run_eval_baseline.py`
Expected: Report shows Tier 2/3 metrics. Mock LLM produces perfect scores (expected — this just validates the pipeline).

**Step 4: Commit**

```bash
git add run_eval_baseline.py
git commit -m "feat(eval): add Tier 2 & 3 to baseline eval pipeline"
```

---

## Summary of New Files

| File | Purpose |
|------|---------|
| `nomai/eval/llm_client.py` | LLM client abstraction + `MockLLMClient` |
| `nomai/eval/scene_qa.py` | Tier 2: Scene QA questions + accuracy metric |
| `nomai/eval/action_prediction.py` | Tier 2: Action prediction metric |
| `nomai/eval/reasoning.py` | Tier 3: G-Eval scoring + multi-hop spatial |
| `tests/test_eval_llm.py` | LLM client tests |
| `tests/test_eval_scene_qa.py` | Scene QA tests |
| `tests/test_eval_action_prediction.py` | Action prediction tests |
| `tests/test_eval_reasoning.py` | G-Eval + spatial reasoning tests |

## New Metrics

| Metric | Tier | Target | Type |
|--------|------|--------|------|
| `scene_qa_accuracy` | 2 | >= 0.8 | Gated |
| `action_prediction_accuracy` | 2 | >= 0.7 | Gated |
| `geval_completeness` | 3 | None | Tracking |
| `geval_clarity` | 3 | None | Tracking |
| `geval_spatial_accuracy` | 3 | None | Tracking |
| `geval_actionability` | 3 | None | Tracking |
| `multihop_spatial_accuracy` | 3 | None | Tracking |

## Deferred (Future Work)

- **`task_pass_at_k`**: Requires full game-playing agent loop. Deferred until agent infrastructure exists.
- **Real LLM backend**: `AnthropicLLMClient` wrapping `anthropic` SDK. Add when ready for real evaluation runs.
- **Ablation testing**: Systematically removing text fields to measure degradation. Natural extension of Scene QA.
- **Cross-version regression**: Comparing Tier 3 scores between representation versions. Needs versioned storage.
- **Hidden held-out QA set**: Anti-gaming safeguard for Scene QA. Add when representation tuning begins.
