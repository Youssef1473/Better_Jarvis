# Better_Jarvis Deterministic Engineering Guardrails
**Location:** `.agents/docs/guardrails.md`
**Status:** Active Deterministic Enforcement

---

## 1. Overview
This specification documents the deterministic safety and quality controls enforced within the Better_Jarvis autonomous engineering environment. Guardrails operate at two levels:
1. **Deterministic Lifecycle Hooks (`.agents/hooks.json` & `.agents/scripts/guardrail.py`):** Automatically intercept tool calls prior to execution and block prohibited actions.
2. **Constitutional Agent Directives (`AGENTS.md` & Universal Rules):** High-precedence instructions governing agent reasoning and execution boundaries.

---

## 2. Deterministic Enforcement Policies

### 2.1 Destructive Command Prevention (`PreToolUse` on `run_command`)
The hook intercepts all command lines and automatically returns `decision: deny` if the command matches any destructive pattern:
- Hard git resets: `git reset --hard`
- Unversioned file wipes: `git clean -f`
- File checkout reverts: `git checkout -- .`, `git restore .`
- Forced pushes: `git push --force`
- Destructive deletions: `Remove-Item -Recurse`, `del /s /q`, `rmdir /s /q`
- Low-level disk formatting: `format`, `diskpart`

### 2.2 Version Control Mutation Protection (`PreToolUse` on `run_command`)
Commands that mutate Git history (`git commit`, `git push`, `git merge`, `git rebase`) are intercepted with `decision: ask`, requiring explicit human confirmation.

### 2.3 Secret & Configuration Integrity (`PreToolUse` on `write_file`, `edit_file`)
Direct write or edit attempts targeting sensitive files are hard-denied:
- Repository configuration: `.git/config`
- Environment files: `.env`, `.env.*`
- Private keys and certificates: `*.key`, `*.pem`, `id_rsa`
- Cloud and SSH credentials: `.aws/*`, `.ssh/*`

### 2.4 Test-Integrity & Anti-Weakening Protections
- **No Assertion Deletion:** Tests cannot be modified to remove failing assertions.
- **Strict Red-Green Proof:** Tests must fail on the behavioral assertion before code is modified.
- **Zero Mocking of the Unit Under Test:** Real integration seams must be exercised.

---

## 3. Lifecycle Hooks Configuration (`.agents/hooks.json`)

```json
{
  "better-jarvis-guardrails": {
    "enabled": true,
    "SessionStart": [
      {
        "type": "command",
        "command": "python scripts/guardrail.py session_start",
        "timeout": 10
      }
    ],
    "PreToolUse": [
      {
        "matcher": "run_command|write_file|edit_file",
        "hooks": [
          {
            "type": "command",
            "command": "python scripts/guardrail.py pre_tool",
            "timeout": 15
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "*",
        "hooks": [
          {
            "type": "command",
            "command": "python scripts/guardrail.py post_tool",
            "timeout": 10
          }
        ]
      }
    ],
    "Stop": [
      {
        "type": "command",
        "command": "python scripts/guardrail.py stop",
        "timeout": 10
      }
    ]
  }
}
```

---

## 4. Verification & Testing
The guardrail implementation is verified by `.agents/scripts/test_guardrail.py` executing 5 automated test suites:
- `test_destructive_command_blocking` (PASS)
- `test_safe_commands_allowed` (PASS)
- `test_vcs_mutations_ask` (PASS)
- `test_sensitive_file_blocking` (PASS)
- `test_session_start` (PASS)
