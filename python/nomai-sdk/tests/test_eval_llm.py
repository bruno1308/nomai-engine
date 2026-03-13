"""Tests for the LLM client abstraction.

Verifies the MockLLMClient correctly cycles responses, records history,
and raises in strict mode when responses are exhausted.  Also tests
ClaudeCodeLLMClient command construction and error handling.
"""

from __future__ import annotations

from unittest.mock import patch, MagicMock
import subprocess

from nomai.eval.llm_client import ClaudeCodeLLMClient, LLMClient, MockLLMClient


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


class TestClaudeCodeLLMClient:
    def test_isinstance_of_base(self) -> None:
        client = ClaudeCodeLLMClient()
        assert isinstance(client, LLMClient)

    def test_default_model(self) -> None:
        client = ClaudeCodeLLMClient()
        assert client.model == "sonnet"

    def test_custom_model(self) -> None:
        client = ClaudeCodeLLMClient(model="haiku")
        assert client.model == "haiku"

    @patch("nomai.eval.llm_client.subprocess.run")
    def test_calls_claude_cli(self, mock_run: MagicMock) -> None:
        mock_run.return_value = MagicMock(
            returncode=0, stdout="  The answer is 4.  ", stderr=""
        )
        client = ClaudeCodeLLMClient(model="haiku")
        result = client.complete("You are a calculator.", "What is 2+2?")
        assert result == "The answer is 4."

        args = mock_run.call_args
        cmd = args[0][0]
        assert cmd[0] == "claude"
        assert "-p" in cmd
        assert "What is 2+2?" in cmd
        assert "--system-prompt" in cmd
        idx = cmd.index("--system-prompt")
        assert cmd[idx + 1] == "You are a calculator."
        assert "--model" in cmd
        idx = cmd.index("--model")
        assert cmd[idx + 1] == "haiku"
        assert "--no-session-persistence" in cmd
        assert "--tools" in cmd
        # Verify CLAUDECODE env var is stripped
        env = args[1]["env"]
        assert "CLAUDECODE" not in env

    @patch("nomai.eval.llm_client.subprocess.run")
    def test_nonzero_exit_raises(self, mock_run: MagicMock) -> None:
        mock_run.return_value = MagicMock(
            returncode=1, stdout="", stderr="auth error"
        )
        client = ClaudeCodeLLMClient()
        try:
            client.complete("sys", "prompt")
            assert False, "Should have raised"
        except RuntimeError as e:
            assert "auth error" in str(e)

    @patch("nomai.eval.llm_client.subprocess.run")
    def test_timeout_raises(self, mock_run: MagicMock) -> None:
        mock_run.side_effect = subprocess.TimeoutExpired("claude", 60)
        client = ClaudeCodeLLMClient(timeout=60)
        try:
            client.complete("sys", "prompt")
            assert False, "Should have raised"
        except RuntimeError as e:
            assert "timed out" in str(e)

    @patch("nomai.eval.llm_client.subprocess.run")
    def test_missing_cli_raises(self, mock_run: MagicMock) -> None:
        mock_run.side_effect = FileNotFoundError()
        client = ClaudeCodeLLMClient()
        try:
            client.complete("sys", "prompt")
            assert False, "Should have raised"
        except RuntimeError as e:
            assert "not found" in str(e)
