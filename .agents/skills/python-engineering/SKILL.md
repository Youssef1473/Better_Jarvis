---
name: python-engineering
description: >-
  Python standards, Win32 ctypes integration, desktop process supervision, and testing harnesses for Better_Jarvis scripts. Use when modifying or executing Python automation scripts.
---

# Python Engineering Standards & Procedures (`python-engineering`)

## 1. Purpose
Defines engineering standards, Win32 ctypes conventions, exception handling, typing discipline, and testing procedures for Python scripts in Better_Jarvis (`scripts/`).

## 2. Activation Criteria
- **When to Use:**
  - Writing or editing any Python file in `scripts/` (e.g. `scripts/run-desktop.py`).
  - Creating evaluation scripts or testing harnesses in Python.
- **When NOT to Use:**
  - When modifying C++ or Rust production code.

## 3. Preconditions
- Python 3.12+ verified operational (`python --version`).

## 4. Required Knowledge
- Python Process Supervisor: `scripts/run-desktop.py`
- Authority Hierarchy: [authority_system.md](../../docs/authority_system.md)

## 5. Engineering Standards & Win32 Interop

### 5.1 Desktop Session & Window Station Binding
- When launching GUI test processes or supervised child binaries on Windows, ensure processes bind properly to the interactive desktop window station:
  ```python
  si.lpDesktop = "winsta0\\default"
  ```
  This prevents `0xC0000142` (STATUS_DLL_INIT_FAILED) errors caused by headless or non-interactive session boundaries.

### 5.2 Strict Typing & Exception Safety
- Use standard Python type annotations (`typing` / built-in generics) for public function signatures.
- Never use bare `except:` clauses; catch specific expected exceptions (`subprocess.CalledProcessError`, `FileNotFoundError`, `OSError`).
- Always check Win32 API return values and log `ctypes.get_last_error()`.

### 5.3 Dependency Discipline
- Rely on Python standard library modules (`ctypes`, `subprocess`, `os`, `sys`, `pathlib`, `json`, `hashlib`) for repository automation.
- Never install external packages via `pip` without explicit authorization.

## 6. Step-by-Step Procedure

1. **Syntax Check:** Validate script syntax using Python's compiler module:
   ```powershell
   python -m py_compile scripts/<script_name>.py
   ```
2. **Execute Targeted Test:** Run the script with test arguments in a controlled directory.
3. **Handle Errors Cleanly:** Ensure script exits with non-zero exit codes on failure and prints diagnostic messages to `sys.stderr`.

## 7. Relevant Validation Commands
- Syntax verification: `python -m py_compile scripts/run-desktop.py`
- Test desktop launcher: `python scripts/run-desktop.py --help`

## 8. Expected Evidence
- `py_compile` succeeds with zero errors.
- Script outputs formatted diagnostic logs and returns appropriate exit codes.

## 9. Failure-Handling Guidance
- If `CreateProcessW` fails with error code 5 (Access Denied), check UAC elevation and process token privileges.
- If error code 2 (File Not Found), resolve the executable absolute path using `shutil.which`.

## 10. Completion Criteria
Script passes syntax compilation, adheres to Win32 ctypes laws, and operates reliably without undeclared external dependencies.

## 11. Related Skills
- [repo-intel](../repo-intel/SKILL.md)
- [task-planning](../task-planning/SKILL.md)
