"""Tests for IronShellError, timeouts, UTF-8, null, stubs and packaging (SDK 1.5.2)."""

import importlib
import re
import shutil
import subprocess
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

import ironshell
from ironshell import (
    IronShellError,
    IronShellRuntime,
    IronShellTimeout,
    _parse_error,
    pipeline,
)

HAS_ISH = shutil.which("ish") is not None
needs_ish = pytest.mark.skipif(not HAS_ISH, reason="ish is not on PATH")


def _runtime(**kwargs):
    return IronShellRuntime(ish_path="/usr/bin/ish", **kwargs)


def _completed(returncode=0, stdout=b"", stderr=b""):
    return MagicMock(returncode=returncode, stdout=stdout, stderr=stderr)


class TestParseError:
    def test_human_form(self):
        err = _parse_error("error[E_PARSE]: unexpected token\n  hint: close the bracket\n", 2)
        assert isinstance(err, IronShellError) and isinstance(err, RuntimeError)
        assert err.code == "E_PARSE"
        assert err.message == "unexpected token"
        assert err.hint == "close the bracket"
        assert err.exit_status == 2
        assert "E_PARSE" in str(err)

    def test_agent_json_form(self):
        raw = '{"error": {"code": "E_POLICY_DENY", "message": "no", "hint": "ask"}}'
        err = _parse_error(raw, 77)
        assert (err.code, err.message, err.hint, err.exit_status) == (
            "E_POLICY_DENY", "no", "ask", 77,
        )

    def test_plain_error_line(self):
        err = _parse_error("error: boom\n", 1)
        assert err.code is None and err.message == "boom"

    def test_exit_without_stderr(self):
        err = _parse_error("", 3)
        assert err.exit_status == 3
        assert err.message == "exited with status 3"


class TestRun:
    def test_utf8_bytes_decoded(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(stdout='"héllo ✓ 日本"'.encode("utf-8"))
            assert _runtime().eval("x") == "héllo ✓ 日本"
            assert run.call_args.kwargs.get("text") is not True

    def test_empty_stdout_is_none(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(stdout=b"")
            assert _runtime().eval("null") is None

    def test_null_is_none(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(stdout=b"null\n")
            assert _runtime().eval("null") is None

    def test_failure_raises_structured_error(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(
                returncode=2, stderr=b"error[E_PARSE]: bad\n  hint: fix it\n"
            )
            with pytest.raises(IronShellError) as info:
                _runtime().eval("{{")
        assert info.value.code == "E_PARSE"
        assert info.value.hint == "fix it"
        assert info.value.exit_status == 2

    def test_exit_status_without_message(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(returncode=3)
            with pytest.raises(IronShellError, match="exited with status 3") as info:
                _runtime().eval("exit(3)")
        assert info.value.exit_status == 3

    def test_timeout_default_and_override(self):
        with patch("ironshell.subprocess.run") as run:
            run.return_value = _completed(stdout=b"1")
            rt = _runtime(timeout=7, memory_limit_mb=256)
            rt.eval("1")
            cmd = run.call_args.args[0]
            assert run.call_args.kwargs["timeout"] == 7
            assert cmd[cmd.index("--timeout") + 1] == "7"
            assert cmd[cmd.index("--memory-limit") + 1] == "256"
            rt.eval("1", timeout=None)
            assert run.call_args.kwargs["timeout"] is None
            assert "--timeout" not in run.call_args.args[0]

    def test_timeout_raises_ironshelltimeout(self):
        with patch(
            "ironshell.subprocess.run",
            side_effect=subprocess.TimeoutExpired(cmd="ish", timeout=1),
        ):
            with pytest.raises(IronShellTimeout) as info:
                _runtime(timeout=1).eval("loop()")
        assert isinstance(info.value, IronShellError)

    def test_missing_binary(self):
        rt = IronShellRuntime(ish_path="/nonexistent/dir/ish-not-here")
        with pytest.raises(IronShellError, match="could not run the ish binary"):
            rt.eval("1")


class TestPipelineNoOps:
    def test_source_alone(self):
        assert pipeline([1]).to_code() == "[1]"


class TestPackaging:
    ROOT = Path(__file__).resolve().parent.parent

    def test_py_typed_declared(self):
        text = (self.ROOT / "pyproject.toml").read_text(encoding="utf-8")
        assert "py.typed" in text
        assert (self.ROOT / "python" / "ironshell" / "py.typed").is_file()

    def test_cloud_requirements_use_sdk_version(self):
        from ironshell.cloud import (
            AzureFunctionsRuntime,
            GCPFunctionsRuntime,
            KnativeRuntime,
            LambdaRuntime,
        )

        want = f"ironshell>={ironshell.__version__.split('+')[0]}"
        for cls in (LambdaRuntime, AzureFunctionsRuntime, GCPFunctionsRuntime, KnativeRuntime):
            assert want in cls().get_requirements()

    def test_version_is_1_5_2(self):
        text = (self.ROOT / "pyproject.toml").read_text(encoding="utf-8")
        assert re.search(r'^version\s*=\s*"1\.5\.2"', text, re.MULTILINE)


class TestLangchainStubs:
    def test_import_succeeds_and_use_fails_clearly(self):
        mod = importlib.import_module("ironshell.langchain")
        if mod.LANGCHAIN_AVAILABLE:
            pytest.skip("langchain is installed")
        from ironshell.langchain import (  # noqa: F401
            IronShellMapReduceTool,
            IronShellMetricsTool,
            IronShellWorkflowTool,
            get_all_ironshell_tools,
        )

        with pytest.raises(ImportError, match="pip install ironshell\\[langchain\\]"):
            IronShellWorkflowTool()


@needs_ish
class TestRealIsh:
    def test_non_ascii_round_trip(self):
        assert IronShellRuntime().eval('"héllo ✓ 日本"') == "héllo ✓ 日本"

    def test_error_has_code(self):
        with pytest.raises(IronShellError) as info:
            IronShellRuntime().eval("[1,")
        assert info.value.code
