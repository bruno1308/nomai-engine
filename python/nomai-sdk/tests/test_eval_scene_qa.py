"""Tests for Scene QA question generation and accuracy metric.

Verifies template-based question generation from SceneSnapshots and
the scene_qa_accuracy metric using MockLLMClient for deterministic
evaluation.
"""

from __future__ import annotations

from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.eval.scene_qa import (
    SceneQuestion,
    _answer_matches,
    generate_scene_questions,
    scene_qa_accuracy,
)
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _make_snapshot() -> SceneSnapshot:
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
                position=(350.0, 300.0),
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
                position=(100.0, 50.0),
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
# Question generation tests
# ---------------------------------------------------------------------------


class TestGenerateSceneQuestions:
    def test_generates_questions(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        assert len(questions) >= 5

    def test_includes_count_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        count_qs = [q for q in questions if q.question_type == "count"]
        assert len(count_qs) >= 1
        assert count_qs[0].expected_answer == "3"

    def test_includes_existence_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        exist_qs = [
            q
            for q in questions
            if q.question_type == "existence" and q.expected_answer == "yes"
        ]
        assert len(exist_qs) >= 1
        # Should reference a role that's in the scene.
        roles_in_scene = {"paddle", "ball", "brick"}
        for q in exist_qs:
            assert any(r in q.question for r in roles_in_scene)

    def test_includes_negative_existence_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        neg_qs = [
            q
            for q in questions
            if q.question_type == "existence" and q.expected_answer == "no"
        ]
        assert len(neg_qs) >= 1

    def test_includes_position_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        pos_qs = [q for q in questions if q.question_type == "position"]
        assert len(pos_qs) >= 1

    def test_includes_size_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        size_qs = [q for q in questions if q.question_type == "size"]
        assert len(size_qs) >= 1

    def test_includes_relative_position_question(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        rel_qs = [q for q in questions if q.question_type == "relative_position"]
        assert len(rel_qs) >= 1

    def test_includes_velocity_question_when_entities_have_velocity(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        vel_qs = [q for q in questions if q.question_type == "velocity"]
        assert len(vel_qs) >= 1

    def test_no_velocity_question_when_no_velocities(self) -> None:
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
        questions = generate_scene_questions(snap)
        vel_qs = [q for q in questions if q.question_type == "velocity"]
        assert len(vel_qs) == 0


# ---------------------------------------------------------------------------
# SceneQuestion dataclass tests
# ---------------------------------------------------------------------------


class TestSceneQuestion:
    def test_to_dict(self) -> None:
        q = SceneQuestion(
            question="How many?",
            expected_answer="3",
            question_type="count",
        )
        d = q.to_dict()
        assert d == {
            "question": "How many?",
            "expected_answer": "3",
            "question_type": "count",
        }

    def test_from_dict(self) -> None:
        d = {
            "question": "How many?",
            "expected_answer": "3",
            "question_type": "count",
        }
        q = SceneQuestion.from_dict(d)
        assert q.question == "How many?"
        assert q.expected_answer == "3"
        assert q.question_type == "count"


# ---------------------------------------------------------------------------
# Accuracy metric tests
# ---------------------------------------------------------------------------


class TestSceneQAAccuracy:
    def test_perfect_score_when_all_correct(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        # Provide the exact expected answers.
        responses = [q.expected_answer for q in questions]
        client = MockLLMClient(responses=responses, strict=True)
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 1.0
        assert result.passed is True
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_zero_score_when_all_wrong(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        # Provide nonsensical answers.
        responses = ["xyzzy_wrong_42"] * len(questions)
        client = MockLLMClient(responses=responses, strict=True)
        result = scene_qa_accuracy(snap, questions, client)
        assert result.value == 0.0
        assert result.passed is False

    def test_partial_score(self) -> None:
        snap = _make_snapshot()
        questions = generate_scene_questions(snap)
        n = len(questions)
        assert n >= 2, "Need at least 2 questions for partial test"
        # Answer the first question correctly, the rest wrong.
        responses = [questions[0].expected_answer] + ["xyzzy_wrong_42"] * (n - 1)
        client = MockLLMClient(responses=responses, strict=True)
        result = scene_qa_accuracy(snap, questions, client)
        assert 0.0 < result.value < 1.0

    def test_empty_questions_vacuously_correct(self) -> None:
        snap = _make_snapshot()
        client = MockLLMClient(responses=["unused"])
        result = scene_qa_accuracy(snap, [], client)
        assert result.value == 1.0
        assert result.passed is True

    def test_llm_receives_scene_text(self) -> None:
        snap = _make_snapshot()
        questions = [
            SceneQuestion(
                question="Is there a paddle in the scene?",
                expected_answer="yes",
                question_type="existence",
            ),
        ]
        client = MockLLMClient(responses=["yes"])
        scene_qa_accuracy(snap, questions, client)
        assert len(client.history) == 1
        _sys, prompt = client.history[0]
        assert "paddle" in prompt
        assert "Is there a paddle in the scene?" in prompt

    def test_answer_matching_is_case_insensitive(self) -> None:
        assert _answer_matches("yes", "Yes, there is a paddle.", "existence") is True

    def test_numeric_matching_extracts_numbers(self) -> None:
        assert _answer_matches("3", "There are 3 entities", "count") is True

    def test_no_false_positive_on_substring(self) -> None:
        # "10" should NOT match expected "1" for count questions.
        assert _answer_matches("1", "10", "count") is False
