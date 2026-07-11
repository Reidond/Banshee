"""Unit tests for the behavioral session extractor.

Pure stdlib, no DB / service / migration dependencies, so this does NOT require the project's
Docker/CI test harness. Run with:

    uv run python -m pytest .claude/skills/session-retrospective/scripts/test_extract_sessions.py
"""

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).parent))
import extract_sessions as ex  # noqa: E402


# --- redaction ------------------------------------------------------------------------------
def test_redaction_masks_secret_shapes():
    raw = (
        "key sk-ant-abcdefghij1234567890XYZ token "
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMeKKF2QT4 "
        "Authorization: Bearer abcdef1234567890ghij "
        'config "private_key": "-----BEGIN PRIVATE KEY-----MIIabc-----END PRIVATE KEY-----" '
        "OPENAI_API_KEY=supersecretvalue123 "
        "google AIzaSyA1234567890abcdefghij1234567890 path C:\\Users\\someuser\\Documents\\x "
        "mangled c:UsersomeuserDocumentsProjectsexample-project"
    )
    out = ex.redact(raw)
    assert "sk-ant-" not in out
    assert "eyJhbGci" not in out
    assert "supersecretvalue123" not in out
    assert "AIzaSy" not in out
    assert "-----BEGIN PRIVATE KEY-----" not in out
    assert (
        "someuser" not in out
    )  # home dir masked (generic placeholder, not a real username)
    assert "REDACTED" in out


def test_redaction_is_idempotent():
    raw = "OPENAI_API_KEY=secretsecret123 sk-abcdefghij1234567890"
    assert ex.redact(ex.redact(raw)) == ex.redact(raw)


# --- session-dir auto-derivation (portability) ----------------------------------------------
def test_encode_repo_path_replaces_separators():
    # "/", "\" and ":" all collapse to "-" — mirrors how Claude Code names ~/.claude/projects/<dir>.
    # Path normalizes "/"->"\" on Windows and keeps "/" on POSIX; replacing BOTH makes it stable.
    assert (
        ex._encode_repo_path(Path("/home/alice/example-project"))
        == "-home-alice-example-project"
    )
    enc = ex._encode_repo_path(Path("C:/Users/x/proj"))
    assert not any(c in enc for c in "/\\:")
    assert enc.casefold() == "c--users-x-proj"


def test_find_session_dir_exact_and_case_insensitive(tmp_path):
    base = tmp_path / "projects"
    base.mkdir()
    # on-disk dir uses a lowercase drive ("c--…"); Path.resolve() may yield an uppercase "C:" — the
    # case-insensitive fallback must still resolve it to the dir that actually exists.
    target = base / "c--Users-x-proj"
    target.mkdir()
    found = ex._find_session_dir(Path("C:/Users/x/proj"), base)
    assert found is not None
    # Compare filesystem identity, not resolved-path strings: on a case-insensitive filesystem
    # (macOS APFS, Windows) the exact-match branch may return the queried casing rather than the
    # on-disk casing, and resolve() does not normalize case.
    assert found.samefile(target)


def test_find_session_dir_returns_none_when_absent(tmp_path):
    base = tmp_path / "projects"
    base.mkdir()
    assert ex._find_session_dir(Path("/no/such/repo"), base) is None


# --- error classification by ORIGIN tool ----------------------------------------------------
def test_content_tool_with_error_word_is_not_an_error():
    # The core false-positive fix: Read returning file content containing "Error:" is DATA.
    assert (
        ex.classify_error("Read", False, "line 1\nraise ValueError: boom\nFAILED test")
        is None
    )
    assert ex.classify_error("Grep", False, "match: Error: handling here") is None
    assert (
        ex.classify_error("WebFetch", False, "the page says Exit code 1 somewhere")
        is None
    )


def test_genuine_errors_get_correct_category():
    assert ex.classify_error("Read", True, "File does not exist.") == "path-not-found"
    assert (
        ex.classify_error("Edit", True, "File has been modified since read")
        == "edit-stale-read"
    )
    assert (
        ex.classify_error("Write", True, "File has not been read yet")
        == "edit-stale-read"
    )
    assert (
        ex.classify_error("Bash", False, "Exit code 1\nruff check failed")
        == "build-compile"
    )
    assert (
        ex.classify_error("Bash", False, "Exit code 1\npytest: 1 failed")
        == "test-failure"
    )
    assert ex.classify_error("Skill", True, "skill is disabled") == "skill-blocked"
    assert (
        ex.classify_error("Bash", False, "Exit code 127\ncommand not found")
        == "runtime-exit"
    )
    assert ex.classify_error("TodoWrite", True, "boom") == "tool-error"


def test_user_rejection_is_separate_bucket():
    assert (
        ex.classify_error(
            "Bash", True, "The user doesn't want to proceed with this tool use"
        )
        == "user-rejected"
    )
    assert ex.classify_error("Edit", True, "user rejected the edit") == "user-rejected"


def test_exit_code_zero_is_not_an_error():
    assert ex.classify_error("Bash", False, "done\nExit code 0") is None


# --- end-to-end -----------------------------------------------------------------------------
def _write_jsonl(path, rows):
    path.write_text("\n".join(json.dumps(r) for r in rows) + "\n", encoding="utf-8")


def _run(session_dir, out_dir, monkeypatch, self_check=True):
    argv = ["extract_sessions.py", str(session_dir), str(out_dir)]
    if self_check:
        argv.append("--self-check")
    monkeypatch.setattr(sys, "argv", argv)
    ex.main()
    return json.loads((out_dir / "aggregate.json").read_text(encoding="utf-8"))


def test_end_to_end(tmp_path, monkeypatch):
    sdir = tmp_path / "sessions_src"
    sdir.mkdir()
    sid = "aaaaaaaa-1111-2222-3333-444444444444"

    rows = [
        {"type": "ai-title", "aiTitle": "Test session"},
        {
            "type": "user",
            "timestamp": "2026-06-20T10:00:00Z",
            "gitBranch": "main",
            "message": {"content": [{"type": "text", "text": "please fix the bug"}]},
        },
        {
            "type": "assistant",
            "timestamp": "2026-06-20T10:00:01Z",
            "message": {
                "content": [
                    {
                        "type": "thinking",
                        "thinking": "let me reconsider, actually wait",
                    },
                    {
                        "type": "tool_use",
                        "id": "t1",
                        "name": "Read",
                        "input": {"file_path": "/repo/a.py"},
                    },
                ]
            },
        },
        # content tool returns text containing "Error:" with is_error false -> NOT an error
        {
            "type": "user",
            "timestamp": "2026-06-20T10:00:02Z",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "t1",
                        "content": "def f():\n  raise ValueError: nope\nError: sample",
                        "is_error": False,
                    }
                ]
            },
        },
        {
            "type": "assistant",
            "timestamp": "2026-06-20T10:00:03Z",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "t2",
                        "name": "Edit",
                        "input": {"file_path": "/repo/a.py"},
                    }
                ]
            },
        },
        # genuine stale-read error
        {
            "type": "user",
            "timestamp": "2026-06-20T10:00:04Z",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "t2",
                        "content": "File has been modified since read",
                        "is_error": True,
                    }
                ]
            },
        },
        # blocked skill
        {
            "type": "assistant",
            "timestamp": "2026-06-20T10:00:05Z",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "t3",
                        "name": "Skill",
                        "input": {"skill": "plan-critic"},
                    }
                ]
            },
        },
        {
            "type": "user",
            "timestamp": "2026-06-20T10:00:06Z",
            "message": {
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "t3",
                        "content": "disabled",
                        "is_error": True,
                    }
                ]
            },
        },
        # AskUserQuestion capture
        {
            "type": "assistant",
            "timestamp": "2026-06-20T10:00:07Z",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "t4",
                        "name": "AskUserQuestion",
                        "input": {
                            "questions": [{"question": "Which approach do you prefer?"}]
                        },
                    }
                ]
            },
        },
        "a bare JSON string line (valid JSON, non-object) must be skipped",
    ]
    _write_jsonl(sdir / f"{sid}.jsonl", rows)
    # append a TRULY malformed (non-JSON) line — must be skipped without dropping the session
    with open(sdir / f"{sid}.jsonl", "a", encoding="utf-8") as fh:
        fh.write("THIS IS NOT JSON {{{\n")

    # session with zero real user messages -> must be skipped
    _write_jsonl(
        sdir / "bbbbbbbb-0000.jsonl",
        [
            {
                "type": "assistant",
                "timestamp": "2026-06-20T09:00:00Z",
                "message": {"content": [{"type": "text", "text": "hi"}]},
            },
        ],
    )

    # nested sub-agent transcript + meta sidecar
    subdir = sdir / sid / "subagents"
    subdir.mkdir(parents=True)
    _write_jsonl(
        subdir / "agent-x.jsonl",
        [
            {
                "type": "user",
                "message": {"content": [{"type": "text", "text": "sub goal"}]},
            },
            {
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "sub work"}]},
            },
        ],
    )
    (subdir / "agent-x.meta.json").write_text(
        json.dumps({"agentType": "Explore", "description": "find files"}),
        encoding="utf-8",
    )

    out_dir = tmp_path / "out"
    agg = _run(sdir, out_dir, monkeypatch)

    # schema contract
    for k in ex.SCHEMA_KEYS["top"]:
        assert k in agg
    assert agg["schemaVersion"] == ex.SCHEMA_VERSION
    assert len(agg["sessions"]) == 1, "0-user session must be skipped"
    s = agg["sessions"][0]
    for k in ex.SCHEMA_KEYS["session"]:
        assert k in s

    # false positive eliminated: the Read content with 'Error:' is NOT counted
    assert agg["errorTaxonomy"]["byCategory"]["tool-error"]["count"] == 0
    assert agg["errorTaxonomy"]["byCategory"]["edit-stale-read"]["count"] == 1
    assert s["errorsByCategory"].get("edit-stale-read") == 1
    assert s["behavior"]["staleReadEdits"] == 1

    # captures
    assert s["counts"]["userMsgs"] == 1
    assert "Which approach do you prefer?" in s["askUserQuestions"]
    assert any(
        si["skill"] == "plan-critic" and si["blocked"] for si in s["skillInvocations"]
    )
    assert s["backtrackingMarkers"] >= 2  # "let me reconsider", "actually", "wait"

    # sub-agent walk + meta sidecar
    assert s["subAgents"]["count"] == 1
    assert s["subAgents"]["agents"][0]["type"] == "Explore"
    assert agg["stats"]["subAgent"]["files"] == 1
    # a no-error sub-agent is still emitted as a top-by-bytes high-signal transcript
    assert s["subAgents"]["errorsByCategory"] == {}
    assert s["subAgents"]["transcriptsEmitted"] == 1
    assert Path(s["subAgents"]["highSignal"][0]["transcriptPath"]).is_file()

    # transcript artifact written + absolute path
    assert Path(s["transcriptPath"]).is_file()
    assert (out_dir / "sessions").glob("*.md")


def test_output_dir_rejects_system_tmp(tmp_path, monkeypatch):
    monkeypatch.setattr(
        sys, "argv", ["extract_sessions.py", str(tmp_path), "/tmp/retro"]
    )
    with pytest.raises(SystemExit):
        ex.main()


def test_main_omitting_session_dir_uses_autoderivation(tmp_path, monkeypatch):
    # When the session_dir positional is OMITTED, main() must call _derive_session_dir() — not treat
    # output_dir as the session dir. Monkeypatch the derivation to a controlled fixture so the test
    # is deterministic (no dependency on the real ~/.claude/projects/ layout).
    sdir = tmp_path / "derived_src"
    sdir.mkdir()
    _write_jsonl(
        sdir / "dddddddd-1111-2222-3333-666666666666.jsonl",
        [
            {
                "type": "user",
                "timestamp": "2026-06-20T10:00:00Z",
                "gitBranch": "main",
                "message": {"content": [{"type": "text", "text": "hello"}]},
            }
        ],
    )
    monkeypatch.setattr(ex, "_derive_session_dir", lambda: sdir)
    out_dir = tmp_path / "out"
    # ONE positional only (output_dir) — the omitted session_dir forces the derivation branch.
    monkeypatch.setattr(
        sys, "argv", ["extract_sessions.py", str(out_dir), "--self-check"]
    )
    ex.main()
    agg = json.loads((out_dir / "aggregate.json").read_text(encoding="utf-8"))
    assert agg["stats"]["totalSessions"] == 1
    assert agg["generatedFor"]["sessionDir"] == str(sdir)


def test_self_check_detects_a_leak(tmp_path):
    # write an artifact with an unredacted secret and confirm the leak gate catches it
    (tmp_path / "sessions").mkdir()
    (tmp_path / "aggregate.json").write_text(
        '{"x":"sk-abcdefghij1234567890leak"}', encoding="utf-8"
    )
    leaks = ex.self_check(tmp_path)
    assert leaks, "self_check must flag an unredacted sk- key"


def test_self_check_scans_emitted_subagent_transcripts(tmp_path):
    # a leak inside an emitted sub-agent transcript must also be caught (rglob coverage)
    sub = tmp_path / "sessions" / "sub" / "sid"
    sub.mkdir(parents=True)
    (tmp_path / "aggregate.json").write_text("{}", encoding="utf-8")
    (sub / "agent-1.md").write_text(
        "ASSISTANT: leaked sk-abcdefghij1234567890SECRET", encoding="utf-8"
    )
    leaks = ex.self_check(tmp_path)
    assert leaks, (
        "self_check must scan sessions/sub/**/*.md, not just top-level transcripts"
    )


def test_subagent_taxonomy_emission_and_redaction(tmp_path, monkeypatch):
    """Sub-agent internals: errors are classified + counted, a redacted transcript is emitted for
    high-signal agents, the emitted transcript is redacted, and --self-check covers it."""
    sdir = tmp_path / "src"
    sdir.mkdir()
    sid = "cccccccc-1111-2222-3333-555555555555"
    _write_jsonl(
        sdir / f"{sid}.jsonl",
        [
            {
                "type": "user",
                "timestamp": "2026-06-20T10:00:00Z",
                "gitBranch": "main",
                "message": {"content": [{"type": "text", "text": "do research"}]},
            },
            {
                "type": "assistant",
                "timestamp": "2026-06-20T10:00:01Z",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "a1",
                            "name": "Agent",
                            "input": {"description": "verify sources"},
                        }
                    ]
                },
            },
        ],
    )
    # sub-agent transcript: a path-not-found error + a SECRET in the tool output
    subdir = sdir / sid / "subagents"
    subdir.mkdir(parents=True)
    _write_jsonl(
        subdir / "agent-1.jsonl",
        [
            {
                "type": "user",
                "message": {"content": [{"type": "text", "text": "verify sources"}]},
            },
            {
                "type": "assistant",
                "message": {
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "r1",
                            "name": "Read",
                            "input": {"file_path": "/repo/missing.md"},
                        }
                    ]
                },
            },
            {
                "type": "user",
                "message": {
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": "r1",
                            "content": "File does not exist. token sk-abcdefghij1234567890SECRET",
                            "is_error": True,
                        }
                    ]
                },
            },
        ],
    )
    (subdir / "agent-1.meta.json").write_text(
        json.dumps({"agentType": "general-purpose", "description": "verify sources"}),
        encoding="utf-8",
    )

    out_dir = tmp_path / "out"
    # _run passes --self-check → main() sys.exit(3) if the emitted sub-agent transcript leaks
    agg = _run(sdir, out_dir, monkeypatch)

    # taxonomy rolled up to the aggregate
    assert agg["subAgentErrorTaxonomy"]["byCategory"]["path-not-found"]["count"] == 1
    assert agg["subAgentErrorTaxonomy"]["totalAgents"] == 1
    assert agg["subAgentErrorTaxonomy"]["agentsWithErrors"] == 1
    assert agg["subAgentErrorTaxonomy"]["transcriptsEmitted"] == 1

    s = agg["sessions"][0]
    assert s["subAgents"]["errorsByCategory"].get("path-not-found") == 1
    assert s["subAgents"]["agentsWithErrors"] == 1
    assert s["subAgents"]["transcriptsEmitted"] == 1

    # emitted transcript exists at an absolute path AND is redacted
    hs = s["subAgents"]["highSignal"]
    assert len(hs) == 1
    sub_md = Path(hs[0]["transcriptPath"])
    assert sub_md.is_file()
    body = sub_md.read_text(encoding="utf-8")
    assert "sk-abcdefghij" not in body, (
        "secret must be redacted in emitted sub-agent transcript"
    )
    assert "ERROR(path-not-found)" in body
    assert not ex.self_check(out_dir), "no leaks expected after redaction"
