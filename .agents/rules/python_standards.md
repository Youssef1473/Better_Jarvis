# Python Engineering Rules & Win32 Standards
**Scope:** Applied to all files under `scripts/`

## 1. Type Annotations & Coding Standards
- All top-level and helper functions must include standard type hints (`typing` / built-in types).
- Avoid bare `except:` clauses; catch specific expected exceptions (`subprocess.CalledProcessError`, `FileNotFoundError`, `OSError`).
- Adhere to PEP 8 style with clean function names and descriptive variables.

## 2. Win32 Desktop & Process Supervision
- When launching GUI or supervised child processes, bind to the interactive desktop window station:
  `si.lpDesktop = "winsta0\\default"`
- Always check Win32 API return values and log `ctypes.get_last_error()`.

## 3. Dependency Discipline
- Rely on standard library modules (`ctypes`, `subprocess`, `os`, `sys`, `pathlib`, `json`, `hashlib`).
- Never install external packages via `pip` without explicit authorization.

## 4. Verification Gates
- Syntax verification: `python -m py_compile <script_path>` (Exit code 0).
- Execution test with test arguments exits with code 0 and proper error logging.
