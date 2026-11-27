#!/usr/bin/env python3
"""
Better_Jarvis Engineering Guardrail Hook Handler
Enforces deterministic safety controls, blocks destructive commands,
and validates tool arguments for Antigravity lifecycle hooks.
Fail-closed architecture: Any malformed security-sensitive input or unhandled
exception during security checks evaluates to "deny".
"""

import sys
import json
import re
import os
import time

def sanitize_log_text(text):
    """Sanitize strings to ensure credentials, tokens, or secrets are never logged."""
    if not text:
        return ""
    # Redact Authorization headers or bearer tokens
    text = re.sub(r"(?i)(bearer\s+)[A-Za-z0-9_\-\.]{8,}", r"\1[REDACTED]", text)
    # Redact passwords, secrets, keys, and tokens
    text = re.sub(r"(?i)(password|secret|key|token|api_key|access_token)[\"']?\s*[:=]\s*[\"']?[A-Za-z0-9_\-\.]{8,}", r"\1=[REDACTED]", text)
    return text

def log_hook_execution(event, tool_name, decision_dict):
    try:
        log_dir = os.path.join(os.path.dirname(__file__), "..", "scratch")
        os.makedirs(log_dir, exist_ok=True)
        log_path = os.path.join(log_dir, "hook_runtime.log")
        with open(log_path, "a", encoding="utf-8") as f:
            ts = time.strftime("%Y-%m-%d %H:%M:%S")
            decision = decision_dict.get("decision", "none")
            reason = sanitize_log_text(decision_dict.get("reason", ""))
            f.write(f"[{ts}] PID:{os.getpid()} EVENT:{event} TOOL:{tool_name} DECISION:{decision} REASON:{reason}\n")
    except Exception:
        pass

DESTRUCTIVE_COMMAND_PATTERNS = [
    r"git\s+reset\s+--hard",
    r"git\s+clean\s+-[a-zA-Z]*f",
    r"git\s+restore\s+\.",
    r"git\s+checkout\s+--\s+\.",
    r"git\s+branch\s+-[a-zA-Z]*D",
    r"git\s+push\s+.*--force",
    r"Remove-Item\s+.*-Recurse",
    r"del\s+/[a-zA-Z]*s\s+/[a-zA-Z]*q",
    r"rmdir\s+/[a-zA-Z]*s\s+/[a-zA-Z]*q",
    r"format\s+[a-zA-Z]:",
    r"diskpart"
]

SENSITIVE_FILE_PATTERNS = [
    r"\.git/config",
    r"\.env(?:\..*)?$",
    r".*\.pem$",
    r".*\.key$",
    r"id_rsa",
    r"\.aws/",
    r"\.ssh/"
]

FILE_MUTATION_TOOLS = {
    "write_file",
    "edit_file",
    "write_to_file",
    "replace_file_content",
    "multi_replace_file_content"
}

def extract_paths_from_args(args):
    """Extract all potential file paths from tool arguments."""
    paths = []
    if not isinstance(args, dict):
        return paths

    # Common path parameter names across tools
    for key in ("Path", "path", "file_path", "filePath", "target_file", "TargetFile", "file", "target"):
        val = args.get(key)
        if isinstance(val, str) and val.strip():
            paths.append(val.strip())
        elif isinstance(val, list):
            for item in val:
                if isinstance(item, str) and item.strip():
                    paths.append(item.strip())
                elif isinstance(item, dict):
                    nested_path = item.get("path") or item.get("Path") or item.get("file")
                    if isinstance(nested_path, str) and nested_path.strip():
                        paths.append(nested_path.strip())

    # Check for multi-file edits (e.g. edits: [{path: ...}, ...])
    for key in ("edits", "files", "operations"):
        val = args.get(key)
        if isinstance(val, list):
            for item in val:
                if isinstance(item, dict):
                    nested_path = item.get("path") or item.get("Path") or item.get("file")
                    if isinstance(nested_path, str) and nested_path.strip():
                        paths.append(nested_path.strip())

    return paths

def extract_command_from_args(args):
    """Extract command line string from tool arguments."""
    if not isinstance(args, dict):
        return ""
    for key in ("CommandLine", "command", "cmd", "command_line"):
        val = args.get(key)
        if isinstance(val, str):
            return val
    return ""

def handle_pre_tool():
    raw_input = ""
    tool_name = "unknown"
    try:
        raw_input = sys.stdin.read()
        if not raw_input.strip():
            res = {"decision": "allow"}
            log_hook_execution("pre_tool", "empty", res)
            print(json.dumps(res))
            return
        
        try:
            data = json.loads(raw_input)
        except Exception as json_err:
            res = {
                "decision": "deny",
                "reason": f"Deterministic Guardrail Block: Malformed JSON payload received by hook ({json_err})."
            }
            log_hook_execution("pre_tool", "malformed_json", res)
            sys.stderr.write(f"Guardrail hook JSON parse error: {json_err}\n")
            print(json.dumps(res))
            return

        tool_call = data.get("toolCall", {})
        tool_name = tool_call.get("name", "")
        args = tool_call.get("args", {})

        def emit_decision(res):
            log_hook_execution("pre_tool", tool_name, res)
            print(json.dumps(res))

        # 1. Guard run_command
        if tool_name == "run_command":
            cmd = extract_command_from_args(args)
            
            # Check destructive commands
            for pat in DESTRUCTIVE_COMMAND_PATTERNS:
                if re.search(pat, cmd, re.IGNORECASE):
                    emit_decision({
                        "decision": "deny",
                        "reason": f"Deterministic Guardrail Block: Destructive command matching pattern '{pat}' is prohibited."
                    })
                    return

            # Check unauthorized git mutations
            if re.search(r"\bgit\s+(?:commit|push|merge|rebase)\b", cmd, re.IGNORECASE):
                emit_decision({
                    "decision": "ask",
                    "reason": "VCS mutation (commit/push/merge/rebase) requires explicit user confirmation per Section 5 of Universal Directives."
                })
                return

        # 2. Guard file modifications across all supported tool names
        if tool_name in FILE_MUTATION_TOOLS:
            paths = extract_paths_from_args(args)
            if not paths and tool_name in FILE_MUTATION_TOOLS:
                raw_args_str = json.dumps(args)
                for pat in SENSITIVE_FILE_PATTERNS:
                    if re.search(pat, raw_args_str, re.IGNORECASE):
                        emit_decision({
                            "decision": "deny",
                            "reason": f"Deterministic Guardrail Block: Potential sensitive file pattern '{pat}' detected in unparsed arguments."
                        })
                        return

            for raw_path in paths:
                normalized_path = raw_path.replace("\\", "/")
                for pat in SENSITIVE_FILE_PATTERNS:
                    if re.search(pat, normalized_path, re.IGNORECASE):
                        emit_decision({
                            "decision": "deny",
                            "reason": f"Deterministic Guardrail Block: Mutation of sensitive file matching '{pat}' is prohibited."
                        })
                        return

        emit_decision({"decision": "allow"})
    except Exception as e:
        # Fail-closed for safety
        sys.stderr.write(f"Guardrail hook error (failing closed): {e}\n")
        res = {
            "decision": "deny",
            "reason": f"Deterministic Guardrail Error (Fail-Closed): {e}"
        }
        log_hook_execution("pre_tool", tool_name, res)
        print(json.dumps(res))

def handle_session_start():
    output = {
        "injectSteps": [
            {
                "systemMessage": {
                    "systemMessage": "Better_Jarvis Autonomous Engineering Environment Active. Preserving all uncommitted user work. Precedence governed by .agents/docs/authority_system.md."
                }
            }
        ]
    }
    print(json.dumps(output))

def handle_post_tool():
    print(json.dumps({}))

def handle_stop():
    print(json.dumps({"decision": "allow"}))

def main():
    event = sys.argv[1] if len(sys.argv) > 1 else "pre_tool"
    if event == "pre_tool":
        handle_pre_tool()
    elif event == "session_start":
        handle_session_start()
    elif event == "post_tool":
        handle_post_tool()
    elif event == "stop":
        handle_stop()
    else:
        print(json.dumps({"decision": "allow"}))

if __name__ == "__main__":
    main()
