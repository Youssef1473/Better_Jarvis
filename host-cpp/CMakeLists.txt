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

add_executable(jarvis-host WIN32
    src/main.cpp
    src/audio/wasapi_capture.cpp
    src/audio/aec_processor.cpp
    src/audio/silero_vad.cpp
    src/audio/whisper_stt.cpp
    src/audio/piper_tts.cpp
    src/input/hotkey_listener.cpp
)

target_include_directories(jarvis-host PRIVATE include)
target_compile_definitions(jarvis-host PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)

if(TARGET nlohmann_json::nlohmann_json)
    target_link_libraries(jarvis-host PRIVATE nlohmann_json::nlohmann_json)
endif()

target_link_libraries(jarvis-host PRIVATE
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
    src/audio/piper_tts.cpp
)
target_include_directories(test_aec PRIVATE include)
target_compile_definitions(test_aec PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_aec PRIVATE
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

add_executable(test_piper_tts
    tests/test_piper_tts.cpp
    src/audio/piper_tts.cpp
)
target_include_directories(test_piper_tts PRIVATE include)
target_compile_definitions(test_piper_tts PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN NOMINMAX)
target_link_libraries(test_piper_tts PRIVATE
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

