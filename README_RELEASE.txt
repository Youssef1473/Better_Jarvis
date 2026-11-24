================================================================================
   Better_Jarvis v1.0.0 — Native C++20 / Rust Windows Voice Assistant
================================================================================

Better_Jarvis is a zero-latency, local-first conversational voice assistant for
Windows 11 x64, engineered with real-time C++20 audio processing and an async
Rust security-hardened daemon.

FEATURES:
- Pure on-device neural inference:
  * Silero VAD v5 ONNX (voice activity detection)
  * Whisper.cpp base.en (local speech-to-text transcription)
  * Piper TTS ONNX + eSpeak-NG (neural speech synthesis)
  * all-MiniLM-L6-v2 ONNX + sqlite-vec (local hybrid semantic memory)
- Real-time WASAPI audio capture with Dynamic Acoustic Echo Cancellation (AEC)
  and multi-delay auto-calibration.
- Sub-5ms instant command fast-path + dynamic routing to Gemini Cloud LLM.
- 3-Tier Security Policy with isolated Win32 Job Object sandbox execution,
  isolated Chromium CDP automation, and cryptographic nonce prompt injection defenses.
- Fail-closed WinRT screen vision & opt-in desktop OCR with visual HUD indicator.

QUICK START:
1. (First run only) Download and verify local neural model weights:
   powershell -ExecutionPolicy Bypass -File scripts\setup-models.ps1

2. Launch Better_Jarvis:
   .\jarvis-host.exe

3. (Optional) Register silent logon autostart via Windows Task Scheduler:
   powershell -ExecutionPolicy Bypass -File scripts\install-startup.ps1

CONTROLS & SHORTCUTS:
- Press and hold Right Control (or configured hotkey) to speak.
- Release to complete utterance.
- Tap Floating HUD Pill button to Pause / Resume audio capture.
- Press Spacebar on Amber Confirmation prompts to approve sensitive actions.
- Double-tap Escape or click Cancel to deny actions or interrupt speech (Barge-In).

To uninstall the scheduled autostart task:
   powershell -ExecutionPolicy Bypass -File scripts\uninstall-startup.ps1
================================================================================
