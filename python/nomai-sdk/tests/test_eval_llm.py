"""Tests for the LLM client abstraction.

Verifies the MockLLMClient correctly cycles responses, records history,
and raises in strict mode when responses are exhausted.
"""

from __future__ import annotations

from nomai.eval.llm_client import LLMClient, MockLLMClient


class TestMockLLMClient:
    def test_returns_configured_response(self) -> None:
        client = MockLLMClient(responses=["42"])
        result = client.complete("system prompt", "What is 6*7?")
        assert result == "42"

    def test_cycles_through_responses(self) -> None:
        client = MockLLMClient(responses=["first", "second", "third"])
        assert client.complete("sys", "q1") == "first"
        assert client.complete("sys", "q2") == "second"
        assert client.complete("sys", "q3") == "third"
        assert client.complete("sys", "q4") == "first"

    def test_strict_mode_raises_on_exhaustion(self) -> None:
        client = MockLLMClient(responses=["only one"], strict=True)
        client.complete("sys", "q1")
        try:
            client.complete("sys", "q2")
            assert False, "Should have raised"
        except IndexError:
            pass

    def test_records_call_history(self) -> None:
        client = MockLLMClient(responses=["yes"])
        client.complete("You are a judge.", "Is the sky blue?")
        assert len(client.history) == 1
        assert client.history[0] == ("You are a judge.", "Is the sky blue?")

    def test_default_response(self) -> None:
        client = MockLLMClient()
        result = client.complete("sys", "question")
        assert isinstance(result, str)
        assert len(result) > 0

    def test_isinstance_of_protocol(self) -> None:
        client = MockLLMClient()
        assert isinstance(client, LLMClient)
