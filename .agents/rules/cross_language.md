# Cross-Language IPC Integration Standards
**Scope:** Applied to interactions between `host-cpp` and `daemon-rust`

## 1. Inter-Process Communication Contract (`JRVS` Codec v2)
- All communication between `host-cpp` and `daemon-rust` must occur over the dedicated Windows Named Pipe `\\.\pipe\jarvis_ipc_<session>`.
- Magic: `0x4A 0x52 0x56 0x53` (`JRVS`).
- Protocol Version: `0x0002`.
- Endianness: Little-Endian for length, message type, and turn ID fields.

## 2. Hard Per-Message Framing Bounds
Buffer memory must NEVER be reserved without verifying payload bounds first:
- Control JSON (`MsgType = 0x01`): Max 262,144 bytes (256 KiB).
- Audio PCM (`MsgType = 0x02`): Max 65,536 bytes (64 KiB).
- Event Notification (`MsgType = 0x03`): Max 65,536 bytes (64 KiB).
Any frame exceeding these limits must be rejected with immediate connection termination.

## 3. Security & Access Control
- Server pipe must use `FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`.
- Explicit SDDL DACL restricted strictly to current user's Logon SID (deny `Everyone` and `Anonymous`).
- On incoming connection, `GetNamedPipeClientProcessId` must match the parent `jarvis-host.exe` PID.

## 4. Verification Gates
- Codec unit test in C++: `host-cpp/tests/test_frame_codec.cpp`.
- Codec unit test in Rust: `daemon-rust/src/ipc/codec.rs`.
- Both sides must produce bit-for-bit identical serialization and framing.
