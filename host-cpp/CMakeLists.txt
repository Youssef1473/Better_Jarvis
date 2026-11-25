if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/include/nlohmann/json.hpp")
    message(STATUS "Using local nlohmann/json header")
else()
    include(FetchContent)
    FetchContent_Declare(
        nlohmann_json
        URL https://github.com/nlohmann/json/releases/download/v3.11.3/json.tar.xz
    )
    FetchContent_MakeAvailable(nlohmann_json)
endif()

# Mandatory Native Inference Linkage (Zero Stubs in Production)
set(UCRT64_ROOT "C:/msys64/ucrt64")
set(ONNXRUNTIME_DLL "${UCRT64_ROOT}/bin/onnxruntime.dll")
set(WHISPER_LIB "${UCRT64_ROOT}/lib/libwhisper.dll.a")
set(GGML_LIB "${UCRT64_ROOT}/lib/libggml.dll.a")
set(ESPEAK_NG_LIB "${UCRT64_ROOT}/lib/libespeak-ng.dll.a")

if(NOT EXISTS "${ONNXRUNTIME_DLL}" OR NOT EXISTS "${WHISPER_LIB}" OR NOT EXISTS "${ESPEAK_NG_LIB}")
    message(FATAL_ERROR "TICKET-12 HARD FAILURE: Mandatory native inference libraries missing from ${UCRT64_ROOT}! Ensure onnxruntime, whisper.cpp, and espeak-ng are installed.")
endif()

message(STATUS "Verified Native Inference Libraries: ONNX Runtime, Whisper.cpp, eSpeak-NG")

add_compile_definitions(
    JARVIS_HAS_ONNXRUNTIME=1
    JARVIS_HAS_WHISPER=1
    JARVIS_HAS_PIPER=1
)

include_directories(
    include
    "${UCRT64_ROOT}/include"
    "${UCRT64_ROOT}/include/onnxruntime"
    "${UCRT64_ROOT}/include/espeak-ng"
)

link_directories(
    "${UCRT64_ROOT}/lib"
    "${UCRT64_ROOT}/bin"
)

add_executable(jarvis-host WIN32
    src/main.cpp
    src/audio/wasapi_capture.cpp
    src/audio/aec_processor.cpp
    src/audio/aec_delay_estimator.cpp
    src/audio/silero_vad.cpp
    src/audio/whisper_stt.cpp
    src/audio/piper_tts.cpp
    src/input/hotkey_listener.cpp
    src/diagnostics/crash_handler.cpp
)

target_include_directories(jarvis-host PRIVATE include)
target_compile_definitions(jarvis-host PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

if(MINGW)
    target_link_options(jarvis-host PRIVATE -municode -s)
endif()

if(TARGET nlohmann_json::nlohmann_json)
    target_link_libraries(jarvis-host PRIVATE nlohmann_json::nlohmann_json)
endif()

target_link_libraries(jarvis-host PRIVATE
    "${ONNXRUNTIME_DLL}"
    whisper
    ggml
    espeak-ng
    user32
    gdi32
    dwmapi
    shell32
    avrt
    ole32
    uuid
    advapi32
)

add_executable(test_frame_codec tests/test_frame_codec.cpp)
target_include_directories(test_frame_codec PRIVATE include)
target_compile_definitions(test_frame_codec PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(test_ring_buffer tests/test_ring_buffer.cpp)
target_include_directories(test_ring_buffer PRIVATE include)
target_compile_definitions(test_ring_buffer PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(test_resampler tests/test_resampler.cpp)
target_include_directories(test_resampler PRIVATE include)
target_compile_definitions(test_resampler PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(test_aec
    tests/test_aec.cpp
    src/audio/aec_processor.cpp
    src/audio/aec_delay_estimator.cpp
    src/audio/piper_tts.cpp
)
target_include_directories(test_aec PRIVATE include)
target_compile_definitions(test_aec PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_aec PRIVATE
    "${ONNXRUNTIME_DLL}"
    espeak-ng
    user32
    ole32
    uuid
    avrt
)

add_executable(test_hotkey_listener
    tests/test_hotkey_listener.cpp
    src/input/hotkey_listener.cpp
)
target_include_directories(test_hotkey_listener PRIVATE include)
target_compile_definitions(test_hotkey_listener PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_hotkey_listener PRIVATE
    user32
)

add_executable(test_latency_tracker
    tests/test_latency_tracker.cpp
)
target_include_directories(test_latency_tracker PRIVATE include)
target_compile_definitions(test_latency_tracker PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(test_vad
    tests/test_vad.cpp
    src/audio/silero_vad.cpp
)
target_include_directories(test_vad PRIVATE include)
target_compile_definitions(test_vad PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_vad PRIVATE
    "${ONNXRUNTIME_DLL}"
)

add_executable(test_piper_tts
    tests/test_piper_tts.cpp
    src/audio/piper_tts.cpp
)
target_include_directories(test_piper_tts PRIVATE include)
target_compile_definitions(test_piper_tts PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_piper_tts PRIVATE
    "${ONNXRUNTIME_DLL}"
    espeak-ng
    ole32
    avrt
    user32
)

add_executable(test_whisper_stt
    tests/test_whisper_stt.cpp
    src/audio/whisper_stt.cpp
)
target_include_directories(test_whisper_stt PRIVATE include)
target_compile_definitions(test_whisper_stt PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_whisper_stt PRIVATE
    whisper
    ggml
)

add_executable(test_crash_handler
    tests/test_crash_handler.cpp
    src/diagnostics/crash_handler.cpp
)
target_include_directories(test_crash_handler PRIVATE include)
target_compile_definitions(test_crash_handler PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(test_hud_watchdog
    tests/test_hud_watchdog.cpp
)
target_include_directories(test_hud_watchdog PRIVATE include)
target_compile_definitions(test_hud_watchdog PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)


