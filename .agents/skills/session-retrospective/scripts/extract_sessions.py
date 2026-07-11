#!/usr/bin/env python3
"""Behavioral extractor for Claude Code JSONL session transcripts.

Reads a project's Claude session directory and emits TWO artifacts into <output-dir>:

  aggregate.json    cross-session error taxonomy + per-session behavioral metrics
  sessions/<id>.md  per-session readable, size-capped, redacted transcript (for deep reading)

Design (see references/extractor-output-schema.md):
  * tool_result is paired to its originating tool_use by id; errors are classified by ORIGIN
    tool. Content-producing tools (Read/Grep/Glob/WebFetch/WebSearch/ToolSearch/NotebookRead)
    are DATA, never errors, unless the tool itself set is_error. This is what kills the
    substring-on-content false-positive class — do NOT reintroduce substring matching on it.
  * Secret redaction is applied to every emitted string; --self-check greps outputs with the
    same high-confidence patterns to prove nothing leaked.
  * Robust: streams line by line, guards malformed lines / huge files / unknown types, skips
    sessions with zero real user messages, never aborts the whole run on one bad file.

Usage:
  python extract_sessions.py [<session-dir>] <output-dir> [--since YYYY-MM-DD] [--until YYYY-MM-DD] [--self-check]

<session-dir> is OPTIONAL — when omitted it is auto-derived from this script's location plus
~/.claude/projects/ (see _derive_session_dir), so the skill works for any engineer on any machine
without a hardcoded path. Pass it explicitly only to override (e.g. a personal-scope install where
the skill is not inside the project's .claude/). <output-dir> MUST be repo-relative
(e.g. .ai/tmp/retro/), never the system /tmp (shells and platforms resolve /tmp inconsistently).
"""

from __future__ import annotations

import argparse
import contextlib
import json
import re
import sys
from collections import defaultdict
from datetime import UTC, datetime
from pathlib import Path

SCHEMA_VERSION = 3

# --- schema contract (machine source of truth; mirrored in extractor-output-schema.md) ---
SCHEMA_KEYS = {
    "top": [
        "schemaVersion",
        "generatedFor",
        "stats",
        "errorTaxonomy",
        "subAgentErrorTaxonomy",
        "sessions",
    ],
    "session": [
        "sessionId",
        "title",
        "gitBranch",
        "startTime",
        "endTime",
        "counts",
        "transcriptBytes",
        "toolsUsed",
        "errorsByCategory",
        "behavior",
        "skillInvocations",
        "askUserQuestions",
        "subAgents",
        "transcriptPath",
        "backtrackingMarkers",
    ],
}

ERROR_CATEGORIES = [
    "path-not-found",
    "edit-stale-read",
    "build-compile",
    "test-failure",
    "skill-blocked",
    "runtime-exit",
    "tool-error",
    "other",
]

# Tools whose tool_result is content (data), never an error unless is_error is set.
CONTENT_TOOLS = {
    "Read",
    "Grep",
    "Glob",
    "WebFetch",
    "WebSearch",
    "ToolSearch",
    "NotebookRead",
}
SHELL_TOOLS = {"Bash", "PowerShell"}
TEST_BUILD_RE = re.compile(
    r"(pytest|docker compose.*test|ruff|basedpyright|mypy|alembic|npm test|uv run)",
    re.I,
)

# Text that marks an IDE/system-injected pseudo-user message (not real user input).
SYSTEM_PREFIXES = (
    "<ide_",
    "<system-reminder>",
    "<command-name>",
    "<command-message>",
    "<local-command",
    "The user opened the file",
    "Caveat:",
    "[Request interrupted",
)

PER_MESSAGE_TEXT_CAP = 2000
TRANSCRIPT_BYTE_CAP = 120 * 1024
SAMPLES_PER_CATEGORY = 3
RE_READ_THRESHOLD = 3
REPEAT_CMD_THRESHOLD = 2
# Sub-agent transcript emission: emit only the HIGH-SIGNAL ones (any error, or the largest few) so a
# retrospective can deep-read them without ever touching raw .jsonl. Bounded so a large fan-out
# can't explode the output; emitted-vs-total is logged (no silent truncation).
SUBAGENT_TOP_BYTES_EMIT = 2
MAX_SUBAGENT_EMIT_PER_SESSION = 8

# --- redaction -------------------------------------------------------------------------------
REDACTIONS = [
    (
        re.compile(
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"
        ),
        "[REDACTED_PEM]",
    ),
    (re.compile(r'"private_key"\s*:\s*"[^"]*"'), '"private_key":"[REDACTED]"'),
    (re.compile(r'"type"\s*:\s*"service_account"'), '"type":"[REDACTED_SA]"'),
    (re.compile(r"sk-ant-[A-Za-z0-9_\-]{12,}"), "[REDACTED_KEY]"),
    (re.compile(r"sk-[A-Za-z0-9_\-]{16,}"), "[REDACTED_KEY]"),
    (
        re.compile(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{6,}\.[A-Za-z0-9_\-]{6,}"),
        "[REDACTED_JWT]",
    ),
    (
        re.compile(r"(?i)authorization:\s*bearer\s+\S+"),
        "Authorization: Bearer [REDACTED]",
    ),
    (re.compile(r"(?i)\bbearer\s+[A-Za-z0-9_\-\.]{12,}"), "Bearer [REDACTED]"),
    (re.compile(r"AIza[A-Za-z0-9_\-]{20,}"), "[REDACTED_GOOGLE_KEY]"),
    (re.compile(r"AKIA[A-Z0-9]{12,}"), "[REDACTED_AWS_KEY]"),
    (
        re.compile(
            r"(?i)\b([A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PWD|CRED)[A-Z0-9_]*)\s*[=:]\s*([^\s,;\"']{4,})"
        ),
        r"\1=[REDACTED]",
    ),
    (re.compile(r"\b[A-Fa-f0-9]{32,}\b"), "[REDACTED_HEX]"),
    (re.compile(r"(?i)[a-z]:\\users\\[^\\/\s]+"), "~"),
    (re.compile(r"(?i)/(?:c/)?users/[^\\/\s]+"), "~"),
    (re.compile(r"/home/[^\\/\s]+"), "~"),
    # Separator-stripped Windows home path: when the Bash tool eats the backslashes a path collapses
    # to e.g. "c:UsersjdoeDocuments…" — the patterns above need separators and miss it, leaking the
    # username. Drive-letter-anchored so it won't match mid-word "…s:users…".
    (re.compile(r"(?i)\b[a-z]:users[a-z0-9._\-]+"), "~"),
]
# High-confidence shapes whose presence in OUTPUT means a redaction miss (used by --self-check).
LEAK_PATTERNS = [
    re.compile(r"sk-[A-Za-z0-9_\-]{16,}"),
    re.compile(r"eyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{6,}\.[A-Za-z0-9_\-]{6,}"),
    re.compile(r"AIza[A-Za-z0-9_\-]{20,}"),
    re.compile(r"AKIA[A-Z0-9]{12,}"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r'"private_key"\s*:\s*"[^"\]]{8,}"'),
]
# Note: home-dir paths are masked in all emitted CONTENT via REDACTIONS (home -> ~). The only
# unredacted absolute path is the functional `transcriptPath` field in the gitignored
# aggregate.json (sub-agents need it); it is never part of the committed report, so the leak gate
# below targets true secret shapes only, not the home username.


def redact(text):
    """Mask secrets/PII in any string before it is emitted. Safe on non-strings."""
    if not isinstance(text, str):
        text = str(text)
    for pattern, repl in REDACTIONS:
        text = pattern.sub(repl, text)
    return text


# --- helpers ---------------------------------------------------------------------------------
def is_real_user_text(text):
    if not text or not text.strip():
        return False
    stripped = text.lstrip()
    return not any(stripped.startswith(p) for p in SYSTEM_PREFIXES)


def clean_user_text(text):
    text = re.sub(r"<system-reminder>[\s\S]*?</system-reminder>", "", text)
    text = re.sub(r"<ide_[^>]*>[\s\S]*?</ide_[^>]*>", "", text)
    return text.strip()


def result_text(content):
    """Flatten a tool_result content (string or list of blocks) to plain text."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for b in content:
            if (
                isinstance(b, dict)
                and b.get("type") == "text"
                and isinstance(b.get("text"), str)
            ):
                parts.append(b["text"])
        return "\n".join(parts)
    return ""


def classify_error(origin, is_error, text):
    """Return an error category, 'user-rejected', or None. Classifies by ORIGIN tool.

    Content tools are never errors unless is_error is set. Shell tools may also error via a
    non-zero exit code surfaced in the result body.
    """
    low = text.lower()
    if is_error and (
        "user doesn't want" in low
        or "user rejected" in low
        or "the user doesn't want to proceed" in low
        or "rejected the" in low
        or "user has chosen not to" in low
    ):
        return "user-rejected"

    is_shell = origin in SHELL_TOOLS
    nonzero_exit = bool(re.search(r"exit code:?\s*[1-9]", low))
    if not (is_error or (is_shell and nonzero_exit)):
        return None

    if origin in {"Read", "Edit", "Write"} and (
        "does not exist" in low or "no such file" in low
    ):
        return "path-not-found"
    if origin in {"Edit", "Write"} and (
        "modified since" in low or "not been read" in low or "has been modified" in low
    ):
        return "edit-stale-read"
    if is_shell:
        if (
            re.search(r"\b(ruff|basedpyright|mypy|pyright|syntaxerror)\b", low)
            or "compileerror" in low
        ):
            return "build-compile"
        if re.search(r"\b(pytest|test session starts|failed|assertionerror)\b", low):
            return "test-failure"
        return "runtime-exit"
    if origin == "Skill":
        return "skill-blocked"
    # Non-shell catch-all. ("other" in ERROR_CATEGORIES is a reserved/forward-compat bucket and is
    # never produced here — tool-error and runtime-exit are the real catch-alls.)
    return "tool-error"


def tool_input_summary(name, inp):
    """One-line, redacted summary of a tool_use input for the readable transcript."""
    if not isinstance(inp, dict):
        return ""
    if name in {"Read", "Edit", "Write", "NotebookEdit"}:
        s = inp.get("file_path", "")
    elif name in SHELL_TOOLS:
        s = inp.get("command", "")
    elif name in {"Grep", "Glob"}:
        s = inp.get("pattern", "")
    elif name == "Skill":
        s = inp.get("skill", "")
    elif name == "Task" or name == "Agent":
        s = inp.get("description", "")
    elif name == "TodoWrite":
        s = f"{len(inp.get('todos', []))} todos"
    else:
        s = inp.get("description") or inp.get("prompt") or inp.get("query") or ""
    return redact(str(s).replace("\n", " "))[:100]


# --- per-session processing ------------------------------------------------------------------
def iter_jsonl(path):
    """Yield parsed objects from a JSONL file; skip malformed lines without aborting."""
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                except (ValueError, json.JSONDecodeError):
                    continue
                if isinstance(
                    obj, dict
                ):  # skip valid-but-non-object lines (bare strings/numbers)
                    yield obj
    except OSError as exc:
        print(f"  ! cannot read {path}: {exc}", file=sys.stderr)


def process_session(jsonl_path, sessions_dir):
    sid = jsonl_path.stem
    sess = {
        "sessionId": sid,
        "title": None,
        "gitBranch": None,
        "startTime": None,
        "endTime": None,
        "counts": {
            "userMsgs": 0,
            "assistantMsgs": 0,
            "toolUse": 0,
            "toolResult": 0,
            "thinking": 0,
        },
        "transcriptBytes": 0,
        "toolsUsed": defaultdict(int),
        "errorsByCategory": defaultdict(int),
        "behavior": {
            "reReads": [],
            "repeatedCommands": [],
            "buildTestLoops": [],
            "staleReadEdits": 0,
        },
        "skillInvocations": [],
        "askUserQuestions": [],
        "subAgents": {
            "count": 0,
            "bytes": 0,
            "msgs": 0,
            "agents": [],
            "errorsByCategory": {},
            "agentsWithErrors": 0,
            "errorSamples": {},
            "transcriptsEmitted": 0,
            "highSignal": [],
        },
        "transcriptPath": None,
        "backtrackingMarkers": 0,
    }
    # cross-session error samples collected by the caller; here we collect locally then return
    local_errors = defaultdict(list)  # category -> [redacted sample]
    user_rejected = []  # [redacted sample]

    id_to_tool = {}
    skill_ids = {}  # tool_use_id -> skill name
    reads = defaultdict(int)
    cmds = defaultdict(int)
    transcript = []
    transcript_bytes = 0
    truncated = False

    def emit(line):
        nonlocal transcript_bytes, truncated
        if truncated:
            return
        b = len(line.encode("utf-8", "replace")) + 1
        if transcript_bytes + b > TRANSCRIPT_BYTE_CAP:
            transcript.append("\n[...transcript truncated at 120 KB...]")
            truncated = True
            return
        transcript.append(line)
        transcript_bytes += b

    for obj in iter_jsonl(jsonl_path):
        otype = obj.get("type")
        ts = obj.get("timestamp")
        if ts:
            if not sess["startTime"] or ts < sess["startTime"]:
                sess["startTime"] = ts
            if not sess["endTime"] or ts > sess["endTime"]:
                sess["endTime"] = ts
        if obj.get("gitBranch") and not sess["gitBranch"]:
            sess["gitBranch"] = obj["gitBranch"]
        if otype == "ai-title":
            sess["title"] = obj.get("aiTitle")
            continue

        msg = obj.get("message") or {}
        content = msg.get("content")

        if otype == "assistant":
            if not isinstance(content, list):
                continue
            sess["counts"]["assistantMsgs"] += 1
            for b in content:
                if not isinstance(b, dict):
                    continue
                bt = b.get("type")
                if bt == "text" and b.get("text", "").strip():
                    emit(
                        f"ASSISTANT: {redact(b['text'].strip())[:PER_MESSAGE_TEXT_CAP]}"
                    )
                elif bt == "thinking":
                    think = b.get("thinking", "") or ""
                    sess["counts"]["thinking"] += 1
                    sess["backtrackingMarkers"] += len(
                        re.findall(
                            r"\b(wait|actually|let me reconsider|on second thought)\b",
                            think,
                            re.I,
                        )
                    )
                    emit(f"  [thinking {len(think)} chars]")
                elif bt == "tool_use":
                    name = b.get("name", "?")
                    sess["counts"]["toolUse"] += 1
                    sess["toolsUsed"][name] += 1
                    tid = b.get("id")
                    if tid and name:
                        id_to_tool[tid] = name
                    inp = b.get("input") or {}
                    if name == "Read" and inp.get("file_path"):
                        reads[inp["file_path"]] += 1
                    elif name in SHELL_TOOLS and inp.get("command"):
                        cmds[inp["command"].strip()] += 1
                    elif name == "Skill" and tid:
                        skill_ids[tid] = inp.get("skill", "?")
                    elif name == "AskUserQuestion":
                        for q in inp.get("questions") or []:
                            if isinstance(q, dict) and q.get("question"):
                                sess["askUserQuestions"].append(
                                    redact(q["question"])[:300]
                                )
                    emit(f"  [{name}] {tool_input_summary(name, inp)}")
            continue

        if otype == "user":
            # user entries carry EITHER real user text OR tool_result blocks (or both).
            blocks = (
                content
                if isinstance(content, list)
                else (
                    [{"type": "text", "text": content}]
                    if isinstance(content, str)
                    else []
                )
            )
            had_user_text = False
            for b in blocks:
                if not isinstance(b, dict):
                    continue
                bt = b.get("type")
                if bt == "text" and is_real_user_text(b.get("text", "")):
                    cleaned = clean_user_text(b["text"])
                    if cleaned:
                        had_user_text = True
                        emit(f"USER: {redact(cleaned)[:PER_MESSAGE_TEXT_CAP]}")
                elif bt == "tool_result":
                    sess["counts"]["toolResult"] += 1
                    origin = id_to_tool.get(b.get("tool_use_id"), "<unknown>")
                    is_err = b.get("is_error") is True
                    txt = result_text(b.get("content"))
                    cat = classify_error(origin, is_err, txt)
                    if cat == "user-rejected":
                        if len(user_rejected) < SAMPLES_PER_CATEGORY:
                            user_rejected.append(
                                f"[{sid[:8]}/{origin}] {redact(txt)[:160]}"
                            )
                    elif cat:
                        sess["errorsByCategory"][cat] += 1
                        if len(local_errors[cat]) < SAMPLES_PER_CATEGORY:
                            local_errors[cat].append(
                                f"[{sid[:8]}/{origin}] {redact(txt)[:160]}"
                            )
                        if cat == "edit-stale-read":
                            sess["behavior"]["staleReadEdits"] += 1
                    # blocked skill: a Skill result with is_error
                    if b.get("tool_use_id") in skill_ids:
                        sess["skillInvocations"].append(
                            {"skill": skill_ids[b["tool_use_id"]], "blocked": is_err}
                        )
                    status = "ok" if not cat else f"ERROR({cat}): {redact(txt)[:120]}"
                    emit(f"    -> {status}")
            if had_user_text:
                sess["counts"]["userMsgs"] += 1
            continue
        # other entry types (last-prompt, queue-operation, file-history-snapshot, attachment, mode)
        # are intentionally ignored for behavioral analysis.

    # behavioral aggregates
    sess["behavior"]["reReads"] = [
        {"file": redact(f), "count": n}
        for f, n in sorted(reads.items(), key=lambda x: -x[1])
        if n >= RE_READ_THRESHOLD
    ]
    rep = [
        {"cmd": redact(c)[:120], "count": n}
        for c, n in sorted(cmds.items(), key=lambda x: -x[1])
        if n >= REPEAT_CMD_THRESHOLD
    ]
    sess["behavior"]["repeatedCommands"] = rep
    sess["behavior"]["buildTestLoops"] = [
        r for r in rep if TEST_BUILD_RE.search(r["cmd"])
    ]

    # sub-agents (nested per parent session): aggregate metrics + an error taxonomy, and emit
    # redacted transcripts for the high-signal ones so they can be deep-dived without raw .jsonl.
    walk_subagents(jsonl_path.parent / sid, sess, sessions_dir)

    # finalize types
    sess["toolsUsed"] = dict(sess["toolsUsed"])
    sess["errorsByCategory"] = dict(sess["errorsByCategory"])

    # write per-session transcript
    if sess["counts"]["userMsgs"] > 0:
        header = f"# Session {sid}\nTitle: {redact(sess['title'] or '(none)')}\nBranch: {sess['gitBranch']}\n{sess['startTime']} -> {sess['endTime']}\n\n"
        body = header + "\n".join(transcript)
        out = sessions_dir / f"{sid}.md"
        out.write_text(body, encoding="utf-8")
        sess["transcriptPath"] = str(out.resolve())
        sess["transcriptBytes"] = len(body.encode("utf-8", "replace"))

    return sess, local_errors, user_rejected


def render_subagent(jf):
    """Parse one sub-agent transcript .jsonl → (redacted_text, msgs, errorsByCategory, samples).

    Mirrors process_session's message walk but lighter: no behavior metrics, just the readable
    redacted narrative + an error taxonomy (same classify_error, by ORIGIN tool). `samples` is a
    list of (category, redacted_sample) tuples. Errors inside sub-agents were previously invisible.
    """
    id_to_tool = {}
    lines = []
    nbytes = 0
    truncated = False
    msgs = 0
    errs = defaultdict(int)
    samples = []

    def emit(line):
        nonlocal nbytes, truncated
        if truncated:
            return
        b = len(line.encode("utf-8", "replace")) + 1
        if nbytes + b > TRANSCRIPT_BYTE_CAP:
            lines.append("\n[...sub-agent transcript truncated at 120 KB...]")
            truncated = True
            return
        lines.append(line)
        nbytes += b

    for obj in iter_jsonl(jf):
        otype = obj.get("type")
        if otype in ("user", "assistant"):
            msgs += 1
        content = (obj.get("message") or {}).get("content")
        if otype == "assistant":
            if not isinstance(content, list):
                continue
            for b in content:
                if not isinstance(b, dict):
                    continue
                bt = b.get("type")
                if bt == "text" and b.get("text", "").strip():
                    emit(
                        f"ASSISTANT: {redact(b['text'].strip())[:PER_MESSAGE_TEXT_CAP]}"
                    )
                elif bt == "thinking":
                    emit("  [thinking]")
                elif bt == "tool_use":
                    name = b.get("name", "?")
                    tid = b.get("id")
                    if tid and name:
                        id_to_tool[tid] = name
                    emit(f"  [{name}] {tool_input_summary(name, b.get('input') or {})}")
        elif otype == "user":
            blocks = (
                content
                if isinstance(content, list)
                else (
                    [{"type": "text", "text": content}]
                    if isinstance(content, str)
                    else []
                )
            )
            for b in blocks:
                if not isinstance(b, dict):
                    continue
                bt = b.get("type")
                if bt == "text" and is_real_user_text(b.get("text", "")):
                    cleaned = clean_user_text(b["text"])
                    if cleaned:
                        emit(f"USER: {redact(cleaned)[:PER_MESSAGE_TEXT_CAP]}")
                elif bt == "tool_result":
                    origin = id_to_tool.get(b.get("tool_use_id"), "<unknown>")
                    is_err = b.get("is_error") is True
                    txt = result_text(b.get("content"))
                    cat = classify_error(origin, is_err, txt)
                    if cat and cat != "user-rejected":
                        errs[cat] += 1
                        if len(samples) < SAMPLES_PER_CATEGORY * 2:
                            samples.append((cat, f"{origin}: {redact(txt)[:140]}"))
                    status = "ok" if not cat else f"ERROR({cat}): {redact(txt)[:120]}"
                    emit(f"    -> {status}")
    return "\n".join(lines), msgs, dict(errs), samples


def walk_subagents(session_dir, sess, sessions_dir):
    """Aggregate nested sub-agent transcripts (<id>/subagents/*.jsonl and
    .../workflows/wf_*/*.jsonl), build a per-session sub-agent error taxonomy, and emit redacted
    transcripts for the HIGH-SIGNAL ones (any error, or the largest few) so they can be deep-dived
    without ever reading raw .jsonl. Uses agent-*.meta.json sidecars for type/goal. Mutates `sess`."""
    sub_root = session_dir / "subagents"
    if not sub_root.is_dir():
        return
    sid8 = sess["sessionId"][:8]
    parsed = []
    for jf in sorted(sub_root.rglob("*.jsonl")):
        sess["subAgents"]["count"] += 1
        size = 0
        with contextlib.suppress(OSError):
            size = jf.stat().st_size
        sess["subAgents"]["bytes"] += size
        meta = jf.with_suffix(".meta.json")  # agent-X.jsonl -> agent-X.meta.json
        atype, goal = None, None
        if meta.is_file():  # sidecar is authoritative for agentType + description
            with contextlib.suppress(OSError, ValueError):
                m = json.loads(meta.read_text(encoding="utf-8", errors="replace"))
                atype = m.get("agentType")
                goal = m.get("description")
        text, msgs, errs, samples = render_subagent(jf)
        sess["subAgents"]["msgs"] += msgs
        if len(sess["subAgents"]["agents"]) < 30:
            sess["subAgents"]["agents"].append(
                {"type": atype or "?", "goal": redact(goal or "?")[:120]}
            )
        parsed.append(
            {
                "rel": str(jf.relative_to(sub_root)),
                "type": atype or "?",
                "goal": redact(goal or "?")[:120],
                "bytes": size,
                "msgs": msgs,
                "errs": errs,
                "samples": samples,
                "text": text,
            }
        )

    # roll up the session-level sub-agent error taxonomy
    agg_errs = defaultdict(int)
    sample_by_cat = defaultdict(list)
    agents_with_errors = 0
    for p in parsed:
        if p["errs"]:
            agents_with_errors += 1
            for c, n in p["errs"].items():
                agg_errs[c] += n
            for c, txt in p["samples"]:
                if len(sample_by_cat[c]) < SAMPLES_PER_CATEGORY:
                    sample_by_cat[c].append(f"[{sid8}/sub] {txt}")
    sess["subAgents"]["errorsByCategory"] = dict(agg_errs)
    sess["subAgents"]["agentsWithErrors"] = agents_with_errors
    sess["subAgents"]["errorSamples"] = dict(sample_by_cat)

    # select high-signal transcripts to emit: every error-bearing agent + the top-N by bytes,
    # de-duplicated and capped (bounded so a huge fan-out can't explode the output).
    top_bytes = sorted(parsed, key=lambda p: -p["bytes"])[:SUBAGENT_TOP_BYTES_EMIT]
    candidates = [p for p in parsed if p["errs"]] + [
        p for p in top_bytes if not p["errs"]
    ]
    seen, ordered = set(), []
    for p in candidates:
        if p["rel"] in seen:
            continue
        seen.add(p["rel"])
        ordered.append(p)
    ordered = ordered[:MAX_SUBAGENT_EMIT_PER_SESSION]
    if ordered:
        sub_dir = sessions_dir / "sub" / sess["sessionId"]
        sub_dir.mkdir(parents=True, exist_ok=True)
        for p in ordered:
            safe = (
                re.sub(r"\.jsonl$", "", p["rel"]).replace("\\", "__").replace("/", "__")
                + ".md"
            )
            header = (
                f"# Sub-agent {redact(safe)} (parent {sid8})\n"
                f"Type: {p['type']}\nGoal: {p['goal']}\n"
                f"bytes: {p['bytes']}  msgs: {p['msgs']}  errors: {p['errs'] or 'none'}\n\n"
            )
            outp = sub_dir / safe
            outp.write_text(header + p["text"], encoding="utf-8")
            sess["subAgents"]["highSignal"].append(
                {
                    "file": safe,
                    "type": p["type"],
                    "goal": p["goal"],
                    "bytes": p["bytes"],
                    "msgs": p["msgs"],
                    "errorsByCategory": p["errs"],
                    "transcriptPath": str(outp.resolve()),
                }
            )
    sess["subAgents"]["transcriptsEmitted"] = len(sess["subAgents"]["highSignal"])


# --- session-dir auto-derivation -------------------------------------------------------------
# Claude Code stores a project's sessions under ~/.claude/projects/<encoded>, where <encoded> is the
# project's ABSOLUTE path with every "/", "\" and ":" replaced by "-". That name is machine-specific
# (drive letter, username, clone location all differ per engineer), so it MUST be derived at runtime,
# never hardcoded — a hardcoded path is portable to exactly one person.
def _encode_repo_path(root):
    """Encode an absolute repo path the way Claude Code names its ~/.claude/projects/<dir>."""
    return re.sub(r"[/\\:]", "-", str(root))


def _find_session_dir(repo_root, projects_base):
    """Return the projects/<encoded> dir for repo_root, or None.

    Tries an exact match first, then a case-insensitive scan so a Windows drive-letter casing diff
    (Path.resolve() yields 'C:\\…' but the on-disk dir is 'c--…') still resolves.
    """
    enc = _encode_repo_path(repo_root)
    cand = projects_base / enc
    if cand.is_dir():
        return cand
    if projects_base.is_dir():
        for d in projects_base.iterdir():
            if d.is_dir() and d.name.casefold() == enc.casefold():
                return d
    return None


def _derive_session_dir():
    """Auto-derive THIS repo's Claude session dir from the script location + the user's home.

    Walks up from this file to the repo root (the dir containing `.claude/`), encodes it, and looks
    under ~/.claude/projects/. Pure runtime derivation (Path(__file__) / Path.home()) so it works for
    any engineer on any machine; exits with a hint (pass the dir explicitly) if it cannot resolve —
    e.g. a personal-scope install where this skill does not live in the project's `.claude/`.
    """
    here = Path(__file__).resolve()
    repo_root = next((p for p in here.parents if (p / ".claude").is_dir()), Path.cwd())
    projects_base = Path.home() / ".claude" / "projects"
    found = _find_session_dir(repo_root, projects_base)
    if found is None:
        avail = (
            sorted(p.name for p in projects_base.iterdir() if p.is_dir())
            if projects_base.is_dir()
            else []
        )
        raise SystemExit(
            "ERROR: could not auto-derive the Claude session dir.\n"
            f"  repo root:  {repo_root}\n"
            f"  looked for: {projects_base / _encode_repo_path(repo_root)}\n"
            f"  available:  {avail or '(none)'}\n"
            "  Pass the session dir explicitly as the first argument to override."
        )
    return found


# --- main ------------------------------------------------------------------------------------
def self_check(output_dir):
    """Grep emitted artifacts for high-confidence secret shapes. Returns list of leaks."""
    leaks = []
    for path in [
        output_dir / "aggregate.json",
        *(
            (output_dir / "sessions").rglob(
                "*.md"
            )  # rglob → also scans emitted sub-agent dirs
            if (output_dir / "sessions").is_dir()
            else []
        ),
    ]:
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for pat in LEAK_PATTERNS:
            m = pat.search(text)
            if m:
                leaks.append(
                    f"{path.name}: matched /{pat.pattern[:40]}/ -> {m.group(0)[:30]}"
                )
    return leaks


def main():
    ap = argparse.ArgumentParser(
        description="Behavioral extractor for Claude Code sessions"
    )
    ap.add_argument(
        "session_dir",
        nargs="?",
        help="Claude session dir under ~/.claude/projects/; auto-derived from the repo root if omitted",
    )
    ap.add_argument(
        "output_dir", help="repo-relative output dir (e.g. .ai/tmp/retro/), never /tmp"
    )
    ap.add_argument("--since")
    ap.add_argument("--until")
    ap.add_argument("--self-check", action="store_true")
    args = ap.parse_args()

    session_dir = Path(args.session_dir) if args.session_dir else _derive_session_dir()
    print(f"Using session dir: {session_dir}", file=sys.stderr)
    output_dir = Path(args.output_dir)
    if str(output_dir).replace("\\", "/").lower().startswith("/tmp"):
        print(
            "ERROR: output-dir must be repo-relative (e.g. .ai/tmp/retro/), never the system /tmp",
            file=sys.stderr,
        )
        sys.exit(2)
    sessions_dir = output_dir / "sessions"
    sessions_dir.mkdir(parents=True, exist_ok=True)

    since = args.since
    until = args.until

    files = sorted(p for p in session_dir.glob("*.jsonl"))
    print(f"Found {len(files)} top-level session files", file=sys.stderr)

    sessions = []
    tax = {c: {"count": 0, "sessions": [], "samples": []} for c in ERROR_CATEGORIES}
    user_rejected = {"count": 0, "sessions": [], "samples": []}

    for f in files:
        try:
            sess, local_errors, rejected = process_session(f, sessions_dir)
        except Exception as exc:  # never abort the whole run on one file
            print(f"  ! error processing {f.name}: {exc}", file=sys.stderr)
            continue
        if sess["counts"]["userMsgs"] == 0:
            continue
        # date overlap filter
        if since and sess["endTime"] and sess["endTime"][:10] < since:
            continue
        if until and sess["startTime"] and sess["startTime"][:10] > until:
            continue
        sessions.append(sess)
        for cat, samples in local_errors.items():
            tax[cat]["count"] += sess["errorsByCategory"].get(cat, 0)
            if sess["errorsByCategory"].get(cat):
                tax[cat]["sessions"].append(sess["sessionId"][:8])
            for s in samples:
                if len(tax[cat]["samples"]) < 8:
                    tax[cat]["samples"].append(s)
        if rejected:
            user_rejected["count"] += len(rejected)
            user_rejected["sessions"].append(sess["sessionId"][:8])
            for s in rejected:
                if len(user_rejected["samples"]) < 8:
                    user_rejected["samples"].append(s)

    sessions.sort(key=lambda s: s["startTime"] or "")

    # cross-session sub-agent error taxonomy (rolled up from each session's walk_subagents pass)
    sub_tax = {c: {"count": 0, "sessions": [], "samples": []} for c in ERROR_CATEGORIES}
    sub_totals = {"totalAgents": 0, "agentsWithErrors": 0, "transcriptsEmitted": 0}
    for s in sessions:
        sa = s["subAgents"]
        sub_totals["totalAgents"] += sa.get("count", 0)
        sub_totals["agentsWithErrors"] += sa.get("agentsWithErrors", 0)
        sub_totals["transcriptsEmitted"] += sa.get("transcriptsEmitted", 0)
        for c, n in sa.get("errorsByCategory", {}).items():
            sub_tax[c]["count"] += n
            sub_tax[c]["sessions"].append(s["sessionId"][:8])
        for c, lst in sa.get("errorSamples", {}).items():
            for smp in lst:
                if len(sub_tax[c]["samples"]) < 8:
                    sub_tax[c]["samples"].append(smp)

    stats = {
        "totalSessions": len(sessions),
        "dateRange": {
            "from": sessions[0]["startTime"] if sessions else None,
            "to": sessions[-1]["endTime"] if sessions else None,
        },
        "totalUserMessages": sum(s["counts"]["userMsgs"] for s in sessions),
        "totalToolCalls": sum(s["counts"]["toolUse"] for s in sessions),
        "totalThinkingBlocks": sum(s["counts"]["thinking"] for s in sessions),
        "subAgent": {
            "files": sum(s["subAgents"]["count"] for s in sessions),
            "bytes": sum(s["subAgents"]["bytes"] for s in sessions),
            "msgs": sum(s["subAgents"]["msgs"] for s in sessions),
        },
        "branchesWorkedOn": sorted(
            {s["gitBranch"] for s in sessions if s["gitBranch"]}
        ),
        "skillsUsed": sorted(
            {si["skill"] for s in sessions for si in s["skillInvocations"]}
        ),
        "skillsBlocked": sorted(
            {
                si["skill"]
                for s in sessions
                for si in s["skillInvocations"]
                if si["blocked"]
            }
        ),
        "allToolsUsed": _merge_tools(sessions),
    }

    out = {
        "schemaVersion": SCHEMA_VERSION,
        "generatedFor": {
            "sessionDir": str(session_dir),
            "since": since,
            "until": until,
            "generatedAt": datetime.now(UTC).isoformat(),
        },
        "stats": stats,
        "errorTaxonomy": {"byCategory": tax, "userRejected": user_rejected},
        "subAgentErrorTaxonomy": {"byCategory": sub_tax, **sub_totals},
        "sessions": sessions,
    }

    # validate schema contract before writing
    _validate_schema(out)

    (output_dir / "aggregate.json").write_text(
        json.dumps(out, indent=2), encoding="utf-8"
    )
    print(
        f"Wrote aggregate.json ({len(sessions)} sessions) + {len(list(sessions_dir.glob('*.md')))} transcripts to {output_dir}",
        file=sys.stderr,
    )
    print(
        "Errors by category: "
        + ", ".join(
            f"{c}={tax[c]['count']}" for c in ERROR_CATEGORIES if tax[c]["count"]
        )
        + f" | user-rejected={user_rejected['count']}",
        file=sys.stderr,
    )
    sub_cat = ", ".join(
        f"{c}={sub_tax[c]['count']}" for c in ERROR_CATEGORIES if sub_tax[c]["count"]
    )
    print(
        f"Sub-agents: {sub_totals['totalAgents']} agents, "
        f"{sub_totals['agentsWithErrors']} with errors, "
        f"{sub_totals['transcriptsEmitted']} transcripts emitted"
        + (f" | {sub_cat}" if sub_cat else ""),
        file=sys.stderr,
    )

    if args.self_check:
        leaks = self_check(output_dir)
        if leaks:
            print("SELF-CHECK FAILED — possible secret leak:", file=sys.stderr)
            for lk in leaks:
                print("  " + lk, file=sys.stderr)
            sys.exit(3)
        print("SELF-CHECK PASSED — no secret shapes found in outputs", file=sys.stderr)


def _merge_tools(sessions):
    merged = defaultdict(int)
    for s in sessions:
        for t, n in s["toolsUsed"].items():
            merged[t] += n
    return dict(sorted(merged.items(), key=lambda x: -x[1]))


def _validate_schema(out):
    missing = [k for k in SCHEMA_KEYS["top"] if k not in out]
    if missing:
        raise ValueError(f"aggregate missing top-level keys: {missing}")
    for s in out["sessions"]:
        miss = [k for k in SCHEMA_KEYS["session"] if k not in s]
        if miss:
            raise ValueError(f"session {s.get('sessionId')} missing keys: {miss}")


if __name__ == "__main__":
    main()
