"""Nomai Engine evaluation framework.

Measures how well the engine enables autonomous AI game development
across five dimensions: Observability, Controllability, Reproducibility,
Verification, and Autonomy, with Efficiency as a cross-cutting constraint.

The north-star metric is CW-ZTVCR (Complexity-Weighted Zero-Touch
Verified Completion Rate).
"""

from nomai.eval.action_prediction import PredictionCase, action_prediction_accuracy
from nomai.eval.agent_harness import AgentConfig, AgentRun, launch_agent, run_agent_eval, score_game
from nomai.eval.llm_client import ClaudeCodeLLMClient, LLMClient, MockLLMClient
from nomai.eval.metrics import (
    DimensionScore,
    EvalDimension,
    MetricResult,
)
from nomai.eval.reasoning import (
    GEVAL_CRITERIA,
    SpatialQuestion,
    geval_all,
    geval_score,
    generate_spatial_questions,
    multihop_spatial_accuracy,
)
from nomai.eval.report import EvalReport
from nomai.eval.runner import EvalRunner
from nomai.eval.scene_qa import SceneQuestion, generate_scene_questions, scene_qa_accuracy

__all__ = [
    "AgentConfig",
    "AgentRun",
    "DimensionScore",
    "EvalDimension",
    "EvalReport",
    "EvalRunner",
    "GEVAL_CRITERIA",
    "ClaudeCodeLLMClient",
    "LLMClient",
    "MetricResult",
    "MockLLMClient",
    "PredictionCase",
    "SceneQuestion",
    "SpatialQuestion",
    "action_prediction_accuracy",
    "launch_agent",
    "run_agent_eval",
    "score_game",
    "geval_all",
    "geval_score",
    "generate_scene_questions",
    "generate_spatial_questions",
    "multihop_spatial_accuracy",
    "scene_qa_accuracy",
]
