"""Tests for G-Eval scene scoring and multi-hop spatial reasoning (Tier 3).

Verifies G-Eval criteria-based LLM scoring of scene descriptions and
multi-hop spatial question generation and accuracy evaluation.
"""

from __future__ import annotations

from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.eval.reasoning import (
    GEVAL_CRITERIA,
    SpatialQuestion,
    generate_spatial_questions,
    geval_all,
    geval_score,
    multihop_spatial_accuracy,
)
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _make_snapshot() -> SceneSnapshot:
    """Two entities: paddle + ball."""
    return SceneSnapshot(
        schema_version=1,
        tick=10,
        sim_time=0.167,
        entities=[
            SceneEntity(
                entity_id=1,
                entity_type="character",
                role="paddle",
                tier="Semantic",
                position=(400.0, 560.0),
                size=(100.0, 15.0),
                velocity=None,
                visible=True,
                z_index=1.0,
            ),
            SceneEntity(
                entity_id=2,
                entity_type="projectile",
                role="ball",
                tier="Semantic",
                position=(200.0, 300.0),
                size=(10.0, 10.0),
                velocity=(200.0, -300.0),
                visible=True,
                z_index=2.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=2,
    )


def _three_entity_snapshot() -> SceneSnapshot:
    """Three entities: paddle(400,560), ball(200,300), brick(600,100)."""
    return SceneSnapshot(
        schema_version=1,
        tick=10,
        sim_time=0.167,
        entities=[
            SceneEntity(
                entity_id=1,
                entity_type="character",
                role="paddle",
                tier="Semantic",
                position=(400.0, 560.0),
                size=(100.0, 15.0),
                velocity=None,
                visible=True,
                z_index=1.0,
            ),
            SceneEntity(
                entity_id=2,
                entity_type="projectile",
                role="ball",
                tier="Semantic",
                position=(200.0, 300.0),
                size=(10.0, 10.0),
                velocity=(200.0, -300.0),
                visible=True,
                z_index=2.0,
            ),
            SceneEntity(
                entity_id=3,
                entity_type="obstacle",
                role="brick",
                tier="Pooled",
                position=(600.0, 100.0),
                size=(60.0, 20.0),
                velocity=None,
                visible=True,
                z_index=0.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=3,
    )


# ---------------------------------------------------------------------------
# G-Eval criteria tests
# ---------------------------------------------------------------------------


class TestGEvalCriteria:
    def test_all_criteria_defined(self) -> None:
        assert len(GEVAL_CRITERIA) == 4
        expected_keys = {"completeness", "clarity", "spatial_accuracy", "actionability"}
        assert set(GEVAL_CRITERIA.keys()) == expected_keys

    def test_criteria_have_descriptions(self) -> None:
        for key, desc in GEVAL_CRITERIA.items():
            assert len(desc) > 10, f"Criterion {key!r} description too short"


# ---------------------------------------------------------------------------
# G-Eval score tests
# ---------------------------------------------------------------------------


class TestGEvalScore:
    def test_high_score(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 1.0

    def test_low_score(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["1"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.0

    def test_mid_score(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["3"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.5

    def test_parses_score_from_verbose_response(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["I rate this a 4 out of 5."])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.75

    def test_invalid_response_scores_zero(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["not a number"])
        result = geval_score(snap, client, "completeness")
        assert result.value == 0.0

    def test_prompt_includes_criterion(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        geval_score(snap, client, "clarity")
        assert len(client.history) == 1
        system, prompt = client.history[0]
        criterion_text = GEVAL_CRITERIA["clarity"]
        assert criterion_text in prompt or criterion_text in system


# ---------------------------------------------------------------------------
# G-Eval all tests
# ---------------------------------------------------------------------------


class TestGEvalAll:
    def test_returns_four_metrics(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        results = geval_all(snap, client)
        assert len(results) == 4

    def test_all_under_observability(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["5"])
        results = geval_all(snap, client)
        for r in results:
            assert r.dimension == EvalDimension.OBSERVABILITY


# ---------------------------------------------------------------------------
# SpatialQuestion dataclass tests
# ---------------------------------------------------------------------------


class TestSpatialQuestion:
    def test_creation(self) -> None:
        q = SpatialQuestion(
            question="Is A closer to B or C?",
            expected_answer="B",
            reasoning_steps=2,
        )
        assert q.question == "Is A closer to B or C?"
        assert q.expected_answer == "B"
        assert q.reasoning_steps == 2

    def test_round_trip(self) -> None:
        q = SpatialQuestion(
            question="Which entity is closest to the top wall?",
            expected_answer="brick",
            reasoning_steps=1,
        )
        d = q.to_dict()
        q2 = SpatialQuestion.from_dict(d)
        assert q == q2


# ---------------------------------------------------------------------------
# Spatial question generation tests
# ---------------------------------------------------------------------------


class TestGenerateSpatialQuestions:
    def test_generates_questions(self) -> None:
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        assert len(questions) >= 2

    def test_includes_distance_comparison(self) -> None:
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        distance_qs = [
            q for q in questions if "closer" in q.question or "closest" in q.question
        ]
        assert len(distance_qs) >= 1

    def test_includes_boundary_question(self) -> None:
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        boundary_qs = [
            q for q in questions if "wall" in q.question or "edge" in q.question
        ]
        assert len(boundary_qs) >= 1

    def test_needs_at_least_two_entities(self) -> None:
        """Single entity should not produce distance comparison questions."""
        snap = SceneSnapshot(
            schema_version=1,
            tick=5,
            sim_time=0.083,
            entities=[
                SceneEntity(
                    entity_id=1,
                    entity_type="character",
                    role="paddle",
                    tier="Semantic",
                    position=(400.0, 560.0),
                    size=(100.0, 15.0),
                    velocity=None,
                    visible=True,
                    z_index=1.0,
                ),
            ],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=1,
        )
        questions = generate_spatial_questions(snap)
        # Distance comparison questions use "closer to" — boundary questions
        # use "closest to the ... wall" and are a different category.
        distance_qs = [q for q in questions if "closer" in q.question]
        assert len(distance_qs) == 0


# ---------------------------------------------------------------------------
# Multi-hop spatial accuracy tests
# ---------------------------------------------------------------------------


class TestMultihopSpatialAccuracy:
    def test_all_correct(self) -> None:
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        responses = [q.expected_answer for q in questions]
        client = MockLLMClient(responses=responses, strict=True)
        result = multihop_spatial_accuracy(snap, questions, client)
        assert result.value == 1.0

    def test_all_wrong(self) -> None:
        snap = _three_entity_snapshot()
        questions = generate_spatial_questions(snap)
        responses = ["completely wrong"] * len(questions)
        client = MockLLMClient(responses=responses, strict=True)
        result = multihop_spatial_accuracy(snap, questions, client)
        assert result.value == 0.0

    def test_empty_questions(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["unused"])
        result = multihop_spatial_accuracy(snap, [], client)
        assert result.value == 1.0
