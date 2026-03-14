#!/usr/bin/env python
"""Top-level entry point for agent evaluation.

Usage::

    python run_agent_eval.py --task breakout --model sonnet --budget 5.0
"""
from __future__ import annotations

import argparse
import sys


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Run an agent evaluation end-to-end.",
    )
    parser.add_argument(
        "--task",
        default="breakout",
        help="GDD task name (default: breakout)",
    )
    parser.add_argument(
        "--model",
        default="sonnet",
        help="Claude model alias (default: sonnet)",
    )
    parser.add_argument(
        "--budget",
        type=float,
        default=5.0,
        help="Maximum spend in USD (default: 5.0)",
    )
    parser.add_argument(
        "--judge-model",
        default=None,
        help="Model for LLM-judged scoring (omit to skip; e.g. haiku, sonnet)",
    )
    args = parser.parse_args()

    from nomai.eval.agent_harness import run_agent_eval

    report = run_agent_eval(
        task=args.task,
        model=args.model,
        max_budget_usd=args.budget,
        judge_model=args.judge_model,
    )

    task_result = report.get("task_result", {})
    # fully_succeeded mirrors TaskResult.fully_succeeded logic
    fully_succeeded = (
        task_result.get("succeeded", False)
        and task_result.get("human_interventions", 1) == 0
        and task_result.get("iterations", 99) <= 5
        and task_result.get("replay_deterministic", False)
        and task_result.get("perf_gates_met", False)
    )
    sys.exit(0 if fully_succeeded else 1)


if __name__ == "__main__":
    main()
