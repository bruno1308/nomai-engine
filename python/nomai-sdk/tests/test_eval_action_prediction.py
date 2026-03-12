"""Tests for action prediction accuracy metric (Tier 2).

Verifies PredictionCase serialization and the action_prediction_accuracy
metric using MockLLMClient for deterministic evaluation.
"""

from __future__ import annotations

from nomai.eval.action_prediction import (
    PredictionCase,
    action_prediction_accuracy,
)
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.metrics import EvalDimension
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


def _snap(tick: int, ball_x: float, ball_y: float) -> SceneSnapshot:
    return SceneSnapshot(
        schema_version=1,
        tick=tick,
        sim_time=tick / 60.0,
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
                position=(ball_x, ball_y),
                size=(10.0, 10.0),
                velocity=(200.0, -300.0),
                visible=True,
                z_index=2.0,
            ),
        ],
        bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
        entity_count=2,
    )


# ---------------------------------------------------------------------------
# PredictionCase tests
# ---------------------------------------------------------------------------


class TestPredictionCase:
    def test_creation(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        assert case.current is current
        assert case.next_state is next_state
        assert case.game_rules == "Ball moves at constant velocity."

    def test_round_trip(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        d = case.to_dict()
        restored = PredictionCase.from_dict(d)
        assert restored.game_rules == case.game_rules
        assert restored.current.tick == case.current.tick
        assert restored.next_state.tick == case.next_state.tick
        assert len(restored.current.entities) == len(case.current.entities)
        assert len(restored.next_state.entities) == len(case.next_state.entities)


# ---------------------------------------------------------------------------
# action_prediction_accuracy metric tests
# ---------------------------------------------------------------------------


class TestActionPredictionAccuracy:
    def test_perfect_prediction(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        client = MockLLMClient(
            responses=['{"1_x": 400.0, "1_y": 560.0, "2_x": 403.3, "2_y": 295.0}'],
        )
        result = action_prediction_accuracy([case], client)
        assert result.value == 1.0
        assert result.passed is True
        assert result.name == "action_prediction_accuracy"
        assert result.dimension == EvalDimension.OBSERVABILITY

    def test_wrong_prediction(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        client = MockLLMClient(
            responses=['{"1_x": 400.0, "1_y": 560.0, "2_x": 100.0, "2_y": 100.0}'],
        )
        result = action_prediction_accuracy([case], client)
        assert result.value == 0.0
        assert result.passed is False

    def test_empty_cases(self) -> None:
        client = MockLLMClient(responses=["unused"])
        result = action_prediction_accuracy([], client)
        assert result.value == 1.0
        assert result.passed is True

    def test_tolerance_for_close_predictions(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        client = MockLLMClient(
            responses=['{"1_x": 400.0, "1_y": 560.0, "2_x": 405.0, "2_y": 296.0}'],
        )
        result = action_prediction_accuracy([case], client, tolerance=5.0)
        assert result.value == 1.0
        assert result.passed is True

    def test_malformed_json_counts_as_wrong(self) -> None:
        current = _snap(10, 400.0, 300.0)
        next_state = _snap(11, 403.3, 295.0)
        case = PredictionCase(
            current=current,
            next_state=next_state,
            game_rules="Ball moves at constant velocity.",
        )
        client = MockLLMClient(responses=["I don't know"])
        result = action_prediction_accuracy([case], client)
        assert result.value == 0.0
        assert result.passed is False
