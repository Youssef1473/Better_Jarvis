#!/usr/bin/env python3
"""
Unit tests for Better_Jarvis Guardrail Hook
Verifies deterministic blocking of destructive commands, protection of secrets,
fail-closed behavior on malformed input, and proper passthrough for safe development commands.
"""

import subprocess
import json
import sys
import os

SCRIPT_PATH = os.path.join(os.path.dirname(__file__), "guardrail.py")

def run_hook(event, payload, raw_str=None):
    proc = subprocess.Popen(
        [sys.executable, SCRIPT_PATH, event],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    input_data = raw_str if raw_str is not None else json.dumps(payload)
    stdout, stderr = proc.communicate(input=input_data)
    if proc.returncode != 0:
        raise RuntimeError(f"Hook process exited with code {proc.returncode}: {stderr}")
    return json.loads(stdout.strip())

def test_destructive_command_blocking():
    destructive_cmds = [
        "git reset --hard HEAD~1",
        "git clean -fdx",
        "git restore .",
        "git checkout -- .",
        "git branch -D main",
        "git push origin main --force",
        "Remove-Item -Recurse -Force .git",
        "del /s /q C:\\Users\\Youssef\\Projects\\Better_Jarvis",
        "rmdir /s /q build",
        "format D:",
        "diskpart"
    ]
    for cmd in destructive_cmds:
        for arg_key in ("CommandLine", "command", "cmd", "command_line"):
            payload = {
                "toolCall": {
                    "name": "run_command",
                    "args": {arg_key: cmd}
                }
            }
            resp = run_hook("pre_tool", payload)
            assert resp.get("decision") == "deny", f"Failed to deny destructive command with {arg_key}: {cmd}, got: {resp}"
    print("PASS: test_destructive_command_blocking")

def test_safe_commands_allowed():
    safe_cmds = [
        "cargo check -p jarvis-daemon",
        "cargo test -p jarvis-daemon",
        "cargo clippy -p jarvis-daemon",
        "git status --short",
        "git log -1 --oneline",
        "git diff --stat",
        "python scripts/verify-env.py"
    ]
    for cmd in safe_cmds:
        payload = {
            "toolCall": {
                "name": "run_command",
                "args": {"CommandLine": cmd}
            }
        }
        resp = run_hook("pre_tool", payload)
        assert resp.get("decision") == "allow", f"Safe command unexpectedly blocked: {cmd}, got: {resp}"
    print("PASS: test_safe_commands_allowed")

def test_vcs_mutations_ask():
    vcs_cmds = [
        "git commit -m 'test'",
        "git push origin main",
        "git merge feature",
        "git rebase main"
    ]
    for cmd in vcs_cmds:
        payload = {
            "toolCall": {
                "name": "run_command",
                "args": {"CommandLine": cmd}
            }
        }
        resp = run_hook("pre_tool", payload)
        assert resp.get("decision") == "ask", f"VCS mutation should ask for confirmation: {cmd}, got: {resp}"
    print("PASS: test_vcs_mutations_ask")

def test_sensitive_file_blocking_multi_tool():
    sensitive_paths = [
        "C:\\Users\\Youssef\\Projects\\Better_Jarvis\\.env",
        ".env.production",
        ".env.local",
        "certs/server.key",
        "keys/id_rsa",
        "keys/private.pem",
        ".git/config",
        ".aws/credentials",
        ".ssh/id_ed25519"
    ]
    tools = [
        "write_file",
        "edit_file",
        "write_to_file",
        "replace_file_content"
    ]
    path_keys = ["Path", "path", "file_path", "filePath", "target_file", "file"]

    for tool in tools:
        for path in sensitive_paths:
            for pkey in path_keys:
                payload = {
                    "toolCall": {
                        "name": tool,
                        "args": {pkey: path, "content": "secret"}
                    }
                }
                resp = run_hook("pre_tool", payload)
                assert resp.get("decision") == "deny", (
                    f"Sensitive file mutation not blocked for tool={tool}, key={pkey}, path={path}: {resp}"
                )
    print("PASS: test_sensitive_file_blocking_multi_tool")

def test_multi_replace_file_content_blocking():
    payload = {
        "toolCall": {
            "name": "multi_replace_file_content",
            "args": {
                "edits": [
                    {"path": "safe_dir/file.txt", "target": "a", "replacement": "b"},
                    {"path": "secrets/.env", "target": "c", "replacement": "d"}
                ]
            }
        }
    }
    resp = run_hook("pre_tool", payload)
    assert resp.get("decision") == "deny", f"multi_replace_file_content failed to block sensitive edit: {resp}"
    print("PASS: test_multi_replace_file_content_blocking")

def test_safe_file_mutations_allowed():
    safe_paths = [
        "daemon-rust/src/main.rs",
        "host-cpp/src/main.cpp",
        "scripts/verify-env.py",
        ".agents/docs/test.md"
    ]
    tools = [
        "write_file",
        "edit_file",
        "write_to_file",
        "replace_file_content"
    ]
    for tool in tools:
        for path in safe_paths:
            payload = {
                "toolCall": {
                    "name": tool,
                    "args": {"Path": path, "Replacement": "// safe"}
                }
            }
            resp = run_hook("pre_tool", payload)
            assert resp.get("decision") == "allow", f"Safe file mutation unexpectedly blocked: {path}, got: {resp}"
    print("PASS: test_safe_file_mutations_allowed")

def test_fail_closed_on_malformed_input():
    malformed_inputs = [
        "{ invalid json ::::",
        '{"toolCall": "not an object"}',
        "NOT_JSON_AT_ALL_JUST_GARBAGE"
    ]
    for bad_input in malformed_inputs:
        resp = run_hook("pre_tool", payload=None, raw_str=bad_input)
        assert resp.get("decision") == "deny", f"Hook should fail-closed on malformed input: {bad_input}, got: {resp}"
    print("PASS: test_fail_closed_on_malformed_input")

def test_session_start():
    resp = run_hook("session_start", {})
    assert "injectSteps" in resp
    assert len(resp["injectSteps"]) > 0
    print("PASS: test_session_start")

def test_post_tool_and_stop():
    resp_post = run_hook("post_tool", {"toolCall": {"name": "run_command"}})
    assert resp_post == {}
    resp_stop = run_hook("stop", {"terminationReason": "NO_TOOL_CALL"})
    assert resp_stop.get("decision") == "allow"
    print("PASS: test_post_tool_and_stop")

if __name__ == "__main__":
    test_destructive_command_blocking()
    test_safe_commands_allowed()
    test_vcs_mutations_ask()
    test_sensitive_file_blocking_multi_tool()
    test_multi_replace_file_content_blocking()
    test_safe_file_mutations_allowed()
    test_fail_closed_on_malformed_input()
    test_session_start()
    test_post_tool_and_stop()
    print("\nALL HARDENED GUARDRAIL TESTS PASSED CLEANLY (9/9).")
