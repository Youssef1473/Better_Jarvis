import ctypes
from ctypes import wintypes
import os
import sys
import shutil

kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)

class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ('hProcess', wintypes.HANDLE),
        ('hThread', wintypes.HANDLE),
        ('dwProcessId', wintypes.DWORD),
        ('dwThreadId', wintypes.DWORD),
    ]

class STARTUPINFO(ctypes.Structure):
    _fields_ = [
        ('cb', wintypes.DWORD),
        ('lpReserved', wintypes.LPWSTR),
        ('lpDesktop', wintypes.LPWSTR),
        ('lpTitle', wintypes.LPWSTR),
        ('dwX', wintypes.DWORD),
        ('dwY', wintypes.DWORD),
        ('dwXSize', wintypes.DWORD),
        ('dwYSize', wintypes.DWORD),
        ('dwXCountChars', wintypes.DWORD),
        ('dwYCountChars', wintypes.DWORD),
        ('dwFillAttribute', wintypes.DWORD),
        ('dwFlags', wintypes.DWORD),
        ('wShowWindow', wintypes.WORD),
        ('cbReserved2', wintypes.WORD),
        ('lpReserved2', ctypes.c_void_p),
        ('hStdInput', wintypes.HANDLE),
        ('hStdOutput', wintypes.HANDLE),
        ('hStdError', wintypes.HANDLE),
    ]

# Ensure Low Temp directory is used if in sandboxed mode
low_temp = os.path.join(os.environ.get('LOCALAPPDATA', ''), 'Temp', 'Low')
if os.path.isdir(low_temp):
    os.environ['TEMP'] = low_temp
    os.environ['TMP'] = low_temp

if len(sys.argv) < 2:
    print('Usage: run-desktop.py [--detach] <command> [args...]', file=sys.stderr)
    sys.exit(1)

detach = False
arg_start = 1
if sys.argv[1] == '--detach':
    detach = True
    arg_start = 2

if arg_start >= len(sys.argv):
    print('Usage: run-desktop.py [--detach] <command> [args...]', file=sys.stderr)
    sys.exit(1)

target = sys.argv[arg_start]
# If target doesn't contain a path separator and isn't found directly, try resolving via PATH
if not os.path.isabs(target) and not os.path.exists(target):
    resolved = shutil.which(target)
    if resolved:
        target = resolved

args = [target] + sys.argv[arg_start + 1:]
formatted_args = []
for a in args:
    if ' ' in a and not (a.startswith('"') and a.endswith('"')):
        formatted_args.append(f'"{a}"')
    else:
        formatted_args.append(a)
cmdline = ' '.join(formatted_args)

si = STARTUPINFO()
si.cb = ctypes.sizeof(si)
si.lpDesktop = "winsta0\\default"
pi = PROCESS_INFORMATION()

creation_flags = 0
inherit_handles = True
if detach:
    inherit_handles = False
    creation_flags = 0x00000200 # CREATE_NEW_PROCESS_GROUP

success = kernel32.CreateProcessW(
    None,
    cmdline,
    None, None, inherit_handles,
    creation_flags,
    None, None,
    ctypes.byref(si),
    ctypes.byref(pi)
)

if not success:
    err = ctypes.get_last_error()
    print(f'CreateProcess failed with error {err}', file=sys.stderr)
    sys.exit(err)

if detach:
    print(pi.dwProcessId)
    kernel32.CloseHandle(pi.hProcess)
    kernel32.CloseHandle(pi.hThread)
    sys.exit(0)

kernel32.WaitForSingleObject(pi.hProcess, 0xFFFFFFFF)
exit_code = wintypes.DWORD()
kernel32.GetExitCodeProcess(pi.hProcess, ctypes.byref(exit_code))
kernel32.CloseHandle(pi.hProcess)
kernel32.CloseHandle(pi.hThread)

sys.exit(exit_code.value)
