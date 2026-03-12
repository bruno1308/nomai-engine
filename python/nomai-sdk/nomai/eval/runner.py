"""Eval runner that orchestrates all evaluation dimensions.

Usage::

    from nomai.eval.runner import EvalRunner
    runner = EvalRunner()
    report = runner.run_all(...)
    print(report.summary())
"""

from __future__ import annotations

import logging
from datetime import datetime, timezone

from nomai.eval import autonomy as autonomy_mod
from nomai.eval import controllability as ctrl_mod
from nomai.eval import observability as obs_mod
from nomai.eval import reproducibility as repro_mod
from nomai.eval import verification as verif_mod
from nomai.eval.autonomy import TaskResult
from nomai.eval.controllability import CommandResult, LatencyObservation
from nomai.eval.metrics import DimensionScore, EvalDimension, MetricResult
from nomai.eval.report import EvalReport
from nomai.eval.reproducibility import HashCheckpoint
from nomai.eval.verification import BugCorpusResult
from nomai.manifest import CausalChain, ComponentChange, TickManifest

logger = logging.getLogger(__name__)


class EvalRunner:
    """Orchestrates evaluation across all dimensions.

    Each ``run_*`` method evaluates a single dimension and returns a
    list of ``MetricResult`` objects.  ``run_all`` combines them into
    a full ``EvalReport``.
    """

    # -- Per-dimension runners -----------------------------------------------

    @staticmethod
    def run_observability(
        manifests: list[TickManifest],
        ground_truth_changes: list[ComponentChange],
        ground_truth_states: dict[int, dict[str, object]],
        causal_chains: list[CausalChain],
        ground_truth_causes: dict[str, str],
    ) -> list[MetricResult]:
        """Run observability dimension metrics."""
        return [
            obs_mod.manifest_change_recall(manifests, ground_truth_changes),
            obs_mod.state_reconstruction_fidelity(manifests, ground_truth_states),
            obs_mod.root_cause_recoverability_at_k(causal_chains, ground_truth_causes),
        ]

    @staticmethod
    def run_controllability(
        command_results: list[CommandResult],
        latency_observations: list[LatencyObservation],
        required_capabilities: set[str],
        exposed_capabilities: set[str],
    ) -> list[MetricResult]:
        """Run controllability dimension metrics."""
        return [
            ctrl_mod.command_semantic_reliability(command_results),
            ctrl_mod.action_effect_latency(latency_observations),
            ctrl_mod.api_capability_coverage(required_capabilities, exposed_capabilities),
        ]

    @staticmethod
    def run_reproducibility(
        hash_checkpoints: list[HashCheckpoint],
        snapshot_pairs: list[tuple[str, str]],
    ) -> list[MetricResult]:
        """Run reproducibility dimension metrics."""
        return [
            repro_mod.replay_hash_match_rate(hash_checkpoints),
            repro_mod.snapshot_fidelity(snapshot_pairs),
        ]

    @staticmethod
    def run_verification(
        bug_results: list[BugCorpusResult],
        total_rules: int = 0,
        expressible_rules: int = 0,
    ) -> list[MetricResult]:
        """Run verification dimension metrics."""
        return [
            verif_mod.intent_expressibility_coverage(total_rules, expressible_rules),
            verif_mod.bug_detection_precision(bug_results),
            verif_mod.bug_detection_recall(bug_results),
            verif_mod.diagnosis_to_fix_success_at_k(bug_results),
        ]

    @staticmethod
    def run_autonomy(
        task_results: list[TaskResult],
    ) -> list[MetricResult]:
        """Run autonomy dimension metrics."""
        return [
            autonomy_mod.zero_touch_completion_rate(task_results),
            autonomy_mod.convergence_median(task_results),
            autonomy_mod.human_intervention_count(task_results),
        ]

    # -- CW-ZTVCR computation ------------------------------------------------

    @staticmethod
    def compute_cw_ztvcr(task_results: list[TaskResult]) -> float:
        """Compute the north-star CW-ZTVCR metric.

        Delegates to ``autonomy_mod.zero_touch_completion_rate`` to avoid
        duplicating the formula.  Returns just the float value.
        """
        result = autonomy_mod.zero_touch_completion_rate(task_results)
        return result.value

    # -- Full run ------------------------------------------------------------

    def run_all(
        self,
        *,
        # Observability inputs
        manifests: list[TickManifest] | None = None,
        ground_truth_changes: list[ComponentChange] | None = None,
        ground_truth_states: dict[int, dict[str, object]] | None = None,
        causal_chains: list[CausalChain] | None = None,
        ground_truth_causes: dict[str, str] | None = None,
        # Controllability inputs
        command_results: list[CommandResult] | None = None,
        latency_observations: list[LatencyObservation] | None = None,
        required_capabilities: set[str] | None = None,
        exposed_capabilities: set[str] | None = None,
        # Reproducibility inputs
        hash_checkpoints: list[HashCheckpoint] | None = None,
        snapshot_pairs: list[tuple[str, str]] | None = None,
        # Verification inputs
        bug_results: list[BugCorpusResult] | None = None,
        total_rules: int = 0,
        expressible_rules: int = 0,
        # Autonomy inputs
        task_results: list[TaskResult] | None = None,
        # Metadata
        engine_version: str = "unknown",
        complexity_tier: str = "breakout",
    ) -> EvalReport:
        """Run all evaluation dimensions and produce a full report."""
        all_metrics: list[MetricResult] = []
        dimension_scores: dict[str, DimensionScore] = {}

        # Observability
        obs_metrics = self.run_observability(
            manifests or [],
            ground_truth_changes or [],
            ground_truth_states or {},
            causal_chains or [],
            ground_truth_causes or {},
        )
        all_metrics.extend(obs_metrics)
        dimension_scores["observability"] = DimensionScore.from_metrics(
            EvalDimension.OBSERVABILITY, obs_metrics,
        )

        # Controllability
        ctrl_metrics = self.run_controllability(
            command_results or [],
            latency_observations or [],
            required_capabilities or set(),
            exposed_capabilities or set(),
        )
        all_metrics.extend(ctrl_metrics)
        dimension_scores["controllability"] = DimensionScore.from_metrics(
            EvalDimension.CONTROLLABILITY, ctrl_metrics,
        )

        # Reproducibility
        repro_metrics = self.run_reproducibility(
            hash_checkpoints or [],
            snapshot_pairs or [],
        )
        all_metrics.extend(repro_metrics)
        dimension_scores["reproducibility"] = DimensionScore.from_metrics(
            EvalDimension.REPRODUCIBILITY, repro_metrics,
        )

        # Verification
        verif_metrics = self.run_verification(
            bug_results or [],
            total_rules,
            expressible_rules,
        )
        all_metrics.extend(verif_metrics)
        dimension_scores["verification"] = DimensionScore.from_metrics(
            EvalDimension.VERIFICATION, verif_metrics,
        )

        # Autonomy
        auto_metrics = self.run_autonomy(task_results or [])
        all_metrics.extend(auto_metrics)
        dimension_scores["autonomy"] = DimensionScore.from_metrics(
            EvalDimension.AUTONOMY, auto_metrics,
        )

        cw_ztvcr = self.compute_cw_ztvcr(task_results or [])

        return EvalReport(
            timestamp=datetime.now(tz=timezone.utc).isoformat(),
            engine_version=engine_version,
            dimensions=dimension_scores,
            cw_ztvcr=cw_ztvcr,
            metrics=all_metrics,
            complexity_tier=complexity_tier,
        )
