"""Tests for the Nomai eval framework.

Verifies metric computation, report serialization, bug corpus integrity,
and the eval runner using mock data.
"""

from __future__ import annotations

import json

from nomai.eval.autonomy import (
    TaskResult,
    convergence_median,
    human_intervention_count,
    zero_touch_completion_rate,
)
from nomai.eval.bug_corpus import SeededBug, full_corpus
from nomai.eval.controllability import (
    CommandResult,
    LatencyObservation,
    action_effect_latency,
    api_capability_coverage,
    command_semantic_reliability,
)
from nomai.eval.metrics import DimensionScore, EvalDimension, MetricResult
from nomai.eval.observability import (
    manifest_change_recall,
    root_cause_recoverability_at_k,
    state_reconstruction_fidelity,
)
from nomai.eval.report import EvalReport
from nomai.eval.reproducibility import (
    HashCheckpoint,
    replay_hash_match_rate,
    snapshot_fidelity,
)
from nomai.eval.runner import EvalRunner
from nomai.eval.verification import (
    BugCorpusResult,
    bug_detection_precision,
    bug_detection_recall,
    diagnosis_to_fix_success_at_k,
    intent_expressibility_coverage,
)
from nomai.eval.llm_client import MockLLMClient
from nomai.eval.scene_qa import SceneQuestion
from nomai.eval.reasoning import SpatialQuestion
from nomai.manifest import (
    Aggregates,
    CausalChain,
    CausalStep,
    ComponentChange,
    TickManifest,
)
from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_manifest(
    tick: int,
    changes: list[ComponentChange] | None = None,
) -> TickManifest:
    return TickManifest(
        tick=tick,
        sim_time=tick / 60.0,
        entity_spawns=[],
        entity_despawns=[],
        component_changes=changes or [],
        events=[],
        aggregates=Aggregates(
            entity_count_by_tier={},
            entity_count_by_type={},
            total_entity_count=0,
        ),
        systems_executed=[],
        commands_processed=0,
        commands_succeeded=0,
    )


def _pos_change(eid: int, tick: int, x: float, y: float) -> ComponentChange:
    return ComponentChange(
        entity_id=eid,
        component_type_name="position",
        old_value={"x": 0.0, "y": 0.0},
        new_value={"x": x, "y": y},
        changed_by_system=0,
        reason_type="SystemInternal",
        reason_detail="physics_step",
        command_index=0,
        tick=tick,
    )


# ===========================================================================
# MetricResult tests
# ===========================================================================

class TestMetricResult:
    def test_creation(self) -> None:
        m = MetricResult(
            name="test",
            dimension=EvalDimension.OBSERVABILITY,
            value=0.99,
            target=0.995,
            passed=False,
            detail="test detail",
        )
        assert m.name == "test"
        assert m.value == 0.99
        assert not m.passed

    def test_serialization_round_trip(self) -> None:
        m = MetricResult(
            name="test",
            dimension=EvalDimension.CONTROLLABILITY,
            value=1.0,
            target=0.99,
            passed=True,
            detail="ok",
        )
        d = m.to_dict()
        restored = MetricResult.from_dict(d)
        assert restored.name == m.name
        assert restored.dimension == m.dimension
        assert restored.value == m.value
        assert restored.passed == m.passed

    def test_none_target(self) -> None:
        m = MetricResult(
            name="tracking",
            dimension=EvalDimension.EFFICIENCY,
            value=42.0,
            target=None,
            passed=True,
            detail="tracking only",
        )
        d = m.to_dict()
        assert d["target"] is None
        restored = MetricResult.from_dict(d)
        assert restored.target is None


# ===========================================================================
# DimensionScore tests
# ===========================================================================

class TestDimensionScore:
    def test_from_metrics_all_pass(self) -> None:
        metrics = [
            MetricResult("a", EvalDimension.OBSERVABILITY, 1.0, 0.9, True, "ok"),
            MetricResult("b", EvalDimension.OBSERVABILITY, 0.95, 0.9, True, "ok"),
        ]
        score = DimensionScore.from_metrics(EvalDimension.OBSERVABILITY, metrics)
        assert score.passed is True
        assert score.score == 1.0

    def test_from_metrics_one_fails(self) -> None:
        metrics = [
            MetricResult("a", EvalDimension.OBSERVABILITY, 1.0, 0.9, True, "ok"),
            MetricResult("b", EvalDimension.OBSERVABILITY, 0.5, 0.9, False, "bad"),
        ]
        score = DimensionScore.from_metrics(EvalDimension.OBSERVABILITY, metrics)
        assert score.passed is False
        assert score.score == 0.5

    def test_from_metrics_empty(self) -> None:
        score = DimensionScore.from_metrics(EvalDimension.OBSERVABILITY, [])
        assert score.passed is True
        assert score.score == 0.0

    def test_serialization_round_trip(self) -> None:
        metrics = [
            MetricResult("a", EvalDimension.OBSERVABILITY, 1.0, 0.9, True, "ok"),
        ]
        score = DimensionScore.from_metrics(EvalDimension.OBSERVABILITY, metrics)
        d = score.to_dict()
        restored = DimensionScore.from_dict(d)
        assert restored.dimension == score.dimension
        assert len(restored.metrics) == 1


# ===========================================================================
# EvalReport tests
# ===========================================================================

class TestEvalReport:
    def test_creation_and_json_round_trip(self) -> None:
        report = EvalReport(
            timestamp="2026-03-12T00:00:00Z",
            engine_version="abc123",
            cw_ztvcr=0.85,
            complexity_tier="breakout",
        )
        json_str = report.to_json()
        data = json.loads(json_str)
        restored = EvalReport.from_dict(data)
        assert restored.cw_ztvcr == 0.85
        assert restored.engine_version == "abc123"

    def test_summary(self) -> None:
        metrics = [
            MetricResult("a", EvalDimension.OBSERVABILITY, 1.0, 0.9, True, "ok"),
        ]
        dim = DimensionScore.from_metrics(EvalDimension.OBSERVABILITY, metrics)
        report = EvalReport(
            timestamp="2026-03-12T00:00:00Z",
            engine_version="abc123",
            dimensions={"observability": dim},
            cw_ztvcr=1.0,
            metrics=metrics,
        )
        summary = report.summary()
        assert "CW-ZTVCR: 1.000" in summary
        assert "observability" in summary
        assert "PASS" in summary


# ===========================================================================
# Observability metric tests
# ===========================================================================

class TestObservability:
    def test_manifest_change_recall_perfect(self) -> None:
        changes = [_pos_change(0, 1, 1.0, 2.0)]
        manifests = [_make_manifest(1, changes)]
        result = manifest_change_recall(manifests, changes)
        assert result.value == 1.0
        assert result.passed is True

    def test_manifest_change_recall_missing(self) -> None:
        gt = [_pos_change(0, 1, 1.0, 2.0), _pos_change(1, 1, 3.0, 4.0)]
        manifests = [_make_manifest(1, [gt[0]])]  # Missing entity 1.
        result = manifest_change_recall(manifests, gt)
        assert result.value == 0.5
        assert result.passed is False

    def test_state_reconstruction_perfect(self) -> None:
        changes = [_pos_change(0, 1, 10.0, 20.0)]
        manifests = [_make_manifest(1, changes)]
        gt_states = {0: {"position": {"x": 10.0, "y": 20.0}}}
        result = state_reconstruction_fidelity(manifests, gt_states)
        assert result.value == 1.0

    def test_state_reconstruction_mismatch(self) -> None:
        changes = [_pos_change(0, 1, 10.0, 20.0)]
        manifests = [_make_manifest(1, changes)]
        gt_states = {0: {"position": {"x": 99.0, "y": 99.0}}}
        result = state_reconstruction_fidelity(manifests, gt_states)
        assert result.value == 0.0

    def test_root_cause_recoverability(self) -> None:
        chain = CausalChain(
            entity_id=0,
            component="position",
            steps=[
                CausalStep(1, 0, 0, "CollisionResponse", "ball_paddle", "bounce"),
                CausalStep(1, 0, 0, "GameRule", "physics_step", "velocity update"),
            ],
        )
        gt_causes = {"position": "ball_paddle"}
        result = root_cause_recoverability_at_k([chain], gt_causes, k=3)
        assert result.value == 1.0
        assert result.passed is True


# ===========================================================================
# Controllability metric tests
# ===========================================================================

class TestControllability:
    def test_command_reliability_perfect(self) -> None:
        results = [CommandResult("spawn", "entity 1", "entity 1", True)]
        result = command_semantic_reliability(results)
        assert result.value == 1.0

    def test_command_reliability_failure(self) -> None:
        results = [
            CommandResult("spawn", "entity 1", "entity 1", True),
            CommandResult("move", "pos 100", "pos 0", False),
        ]
        result = command_semantic_reliability(results)
        assert result.value == 0.5

    def test_latency_within_target(self) -> None:
        obs = [LatencyObservation(1, 2), LatencyObservation(3, 4)]
        result = action_effect_latency(obs)
        assert result.value == 1.0
        assert result.passed is True

    def test_latency_over_target(self) -> None:
        obs = [LatencyObservation(1, 5)]  # 4-tick latency.
        result = action_effect_latency(obs)
        assert result.value == 4.0
        assert result.passed is False

    def test_api_coverage(self) -> None:
        required = {"spawn", "despawn", "set_component", "tick"}
        exposed = {"spawn", "despawn", "set_component", "tick", "run"}
        result = api_capability_coverage(required, exposed)
        assert result.value == 1.0
        assert result.passed is True

    def test_api_coverage_missing(self) -> None:
        required = {"spawn", "despawn", "replay"}
        exposed = {"spawn", "despawn"}
        result = api_capability_coverage(required, exposed)
        assert abs(result.value - 2 / 3) < 0.01


# ===========================================================================
# Reproducibility metric tests
# ===========================================================================

class TestReproducibility:
    def test_replay_hash_all_match(self) -> None:
        cps = [HashCheckpoint(1, "abc", "abc"), HashCheckpoint(2, "def", "def")]
        result = replay_hash_match_rate(cps)
        assert result.value == 1.0

    def test_replay_hash_divergence(self) -> None:
        cps = [HashCheckpoint(1, "abc", "abc"), HashCheckpoint(2, "def", "xyz")]
        result = replay_hash_match_rate(cps)
        assert result.value == 0.5

    def test_snapshot_fidelity_perfect(self) -> None:
        pairs = [("h1", "h1"), ("h2", "h2")]
        result = snapshot_fidelity(pairs)
        assert result.value == 1.0

    def test_snapshot_fidelity_broken(self) -> None:
        pairs = [("h1", "h1"), ("h2", "h3")]
        result = snapshot_fidelity(pairs)
        assert result.value == 0.5


# ===========================================================================
# Verification metric tests
# ===========================================================================

class TestVerification:
    def test_intent_coverage(self) -> None:
        result = intent_expressibility_coverage(10, 9)
        assert result.value == 0.9
        assert result.passed is True

    def test_bug_detection_precision(self) -> None:
        results = [
            BugCorpusResult("b1", detected=True, is_true_bug=True),
            BugCorpusResult("b2", detected=True, is_true_bug=True),
            BugCorpusResult("clean", detected=False, is_true_bug=False),
        ]
        result = bug_detection_precision(results)
        assert result.value == 1.0  # 2 TP, 0 FP.

    def test_bug_detection_recall(self) -> None:
        results = [
            BugCorpusResult("b1", detected=True, is_true_bug=True),
            BugCorpusResult("b2", detected=False, is_true_bug=True),
        ]
        result = bug_detection_recall(results)
        assert result.value == 0.5  # 1 TP, 1 FN.

    def test_diagnosis_to_fix(self) -> None:
        results = [
            BugCorpusResult("b1", detected=True, is_true_bug=True, attempts_to_fix=1),
            BugCorpusResult("b2", detected=True, is_true_bug=True, attempts_to_fix=3),
            BugCorpusResult("b3", detected=True, is_true_bug=True, attempts_to_fix=2),
        ]
        result = diagnosis_to_fix_success_at_k(results, k=2)
        assert abs(result.value - 2 / 3) < 0.01  # b1 and b3 fixed in <=2.


# ===========================================================================
# Autonomy metric tests
# ===========================================================================

class TestAutonomy:
    def test_cw_ztvcr_all_succeed(self) -> None:
        tasks = [
            TaskResult("t1", True, 1.0, 3, 0, True, True),
            TaskResult("t2", True, 2.0, 2, 0, True, True),
        ]
        result = zero_touch_completion_rate(tasks)
        assert result.value == 1.0

    def test_cw_ztvcr_partial(self) -> None:
        tasks = [
            TaskResult("t1", True, 1.0, 3, 0, True, True),  # Success.
            TaskResult("t2", True, 1.0, 5, 1, True, True),  # Human intervention.
        ]
        result = zero_touch_completion_rate(tasks)
        assert result.value == 0.5  # Only t1 fully succeeded.

    def test_convergence_median(self) -> None:
        tasks = [
            TaskResult("t1", True, 1.0, 3),
            TaskResult("t2", True, 1.0, 5),
            TaskResult("t3", True, 1.0, 1),
        ]
        result = convergence_median(tasks)
        assert result.value == 3.0  # Median of [1, 3, 5].

    def test_human_intervention_zero(self) -> None:
        tasks = [
            TaskResult("t1", True, 1.0, 3, 0),
            TaskResult("t2", True, 1.0, 2, 0),
        ]
        result = human_intervention_count(tasks)
        assert result.value == 0.0
        assert result.passed is True


# ===========================================================================
# Bug corpus tests
# ===========================================================================

class TestBugCorpus:
    def test_corpus_well_formed(self) -> None:
        corpus = full_corpus()
        assert len(corpus) >= 5
        for bug in corpus:
            assert bug.bug_id
            assert bug.name
            assert bug.description
            assert bug.category
            assert bug.severity in ("critical", "major", "minor")
            assert isinstance(bug.manifests, list)

    def test_has_clean_scenario(self) -> None:
        corpus = full_corpus()
        clean = [b for b in corpus if not b.expected_detection]
        assert len(clean) >= 1, "Corpus must include at least one clean scenario."

    def test_has_true_bugs(self) -> None:
        corpus = full_corpus()
        bugs = [b for b in corpus if b.expected_detection]
        assert len(bugs) >= 5, "Corpus must include at least 5 seeded bugs."


# ===========================================================================
# EvalRunner tests
# ===========================================================================

class TestEvalRunner:
    def test_compute_cw_ztvcr(self) -> None:
        tasks = [
            TaskResult("t1", True, 2.0, 3, 0, True, True),
            TaskResult("t2", False, 1.0, 10, 0, True, True),
        ]
        cw = EvalRunner.compute_cw_ztvcr(tasks)
        assert abs(cw - 2 / 3) < 0.01

    def test_run_all_empty(self) -> None:
        runner = EvalRunner()
        report = runner.run_all(engine_version="test")
        assert report.engine_version == "test"
        assert len(report.dimensions) == 5
        assert len(report.metrics) > 0

    def test_run_all_with_data(self) -> None:
        runner = EvalRunner()
        changes = [_pos_change(0, 1, 1.0, 2.0)]
        manifests = [_make_manifest(1, changes)]
        report = runner.run_all(
            manifests=manifests,
            ground_truth_changes=changes,
            ground_truth_states={0: {"position": {"x": 1.0, "y": 2.0}}},
            hash_checkpoints=[HashCheckpoint(1, "abc", "abc")],
            snapshot_pairs=[("h", "h")],
            task_results=[TaskResult("t1", True, 1.0, 2, 0, True, True)],
            engine_version="v0.1",
        )
        assert report.cw_ztvcr == 1.0
        obs = report.dimensions["observability"]
        assert obs.passed is True


# ===========================================================================
# EvalRunner Tier 2 & 3 tests
# ===========================================================================

class TestEvalRunnerTier2Tier3:
    def test_run_scene_qa(self):
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0),
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
        assert len(results) == 4

    def test_run_all_includes_tier2_when_provided(self):
        runner = EvalRunner()
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[
                SceneEntity(entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0), size=None,
                    velocity=None, visible=True, z_index=0.0),
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

    def test_run_all_geval_opt_in(self):
        """G-Eval only runs when run_geval=True."""
        runner = EvalRunner()
        snap = SceneSnapshot(
            schema_version=1, tick=1, sim_time=0.0,
            entities=[],
            bounds=SceneBounds(min_x=0.0, min_y=0.0, max_x=800.0, max_y=600.0),
            entity_count=0,
        )
        client = MockLLMClient(responses=["4", "4", "4", "4"])
        # Without run_geval=True, no G-Eval metrics
        report1 = runner.run_all(scene_snapshot=snap, llm_client=client)
        geval_names = {m.name for m in report1.metrics if m.name.startswith("geval_")}
        assert len(geval_names) == 0

        # With run_geval=True, G-Eval metrics present
        client2 = MockLLMClient(responses=["4", "4", "4", "4"])
        report2 = runner.run_all(scene_snapshot=snap, llm_client=client2, run_geval=True)
        geval_names2 = {m.name for m in report2.metrics if m.name.startswith("geval_")}
        assert len(geval_names2) == 4
