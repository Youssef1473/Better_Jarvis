#pragma once

#include <cstdint>
#include <cstring>
#include <string>
#include <vector>
#include <stdexcept>

namespace jarvis::ipc {

constexpr uint8_t  kMagic[4]           = {'J', 'R', 'V', 'S'};
constexpr uint16_t kProtocolVersion    = 2;

constexpr uint16_t kMsgTypeControlJson = 0x0001;
constexpr uint16_t kMsgTypeAudioPcm    = 0x0002;

constexpr size_t   kHeaderSize         = 16;
constexpr size_t   kMaxControlPayload  = 256 * 1024; // 256 KiB
constexpr size_t   kMaxAudioPayload    = 64 * 1024;  // 64 KiB

struct Header {
    uint16_t version{0};
    uint16_t msg_type{0};
    uint32_t turn_id{0};
    uint32_t payload_len{0};
};

inline size_t MaxPayloadForType(uint16_t msg_type) noexcept {
    switch (msg_type) {
        case kMsgTypeControlJson: return kMaxControlPayload;
        case kMsgTypeAudioPcm:    return kMaxAudioPayload;
        default:                  return 0;
    }
}

/// Decodes and validates a 16-byte JRVS v2 header.
/// Returns true if valid, false if magic/version/msg_type/payload_len are invalid or oversized.
inline bool DecodeHeader(const uint8_t* data, size_t size, Header& out_header) noexcept {
    if (size < kHeaderSize || !data) return false;

    // 1. Verify 4-byte ASCII magic "JRVS"
    if (std::memcmp(data, kMagic, 4) != 0) return false;

    uint16_t version = 0;
    uint16_t msg_type = 0;
    uint32_t turn_id = 0;
    uint32_t payload_len = 0;

    std::memcpy(&version, data + 4, 2);
    std::memcpy(&msg_type, data + 6, 2);
    std::memcpy(&turn_id, data + 8, 4);
    std::memcpy(&payload_len, data + 12, 4);

    // 2. Verify Protocol Version
    if (version != kProtocolVersion) return false;

    // 3. Verify Message Type & Per-Type Payload Bound BEFORE allocating
    const size_t max_allowed = MaxPayloadForType(msg_type);
    if (max_allowed == 0 || payload_len > max_allowed) return false;

    out_header.version = version;
    out_header.msg_type = msg_type;
    out_header.turn_id = turn_id;
    out_header.payload_len = payload_len;
    return true;
}

/// Encodes payload into a JRVS v2 frame buffer.
inline std::vector<uint8_t> EncodeFrame(uint16_t msg_type, const std::string& payload, uint32_t turn_id = 0) {
    const size_t max_allowed = MaxPayloadForType(msg_type);
    if (max_allowed == 0) {
        throw std::invalid_argument("Unknown IPC message type");
    }
    if (payload.size() > max_allowed) {
        throw std::length_error("Payload exceeds maximum permitted size for message type");
    }

    const uint32_t len = static_cast<uint32_t>(payload.size());
    std::vector<uint8_t> buf(kHeaderSize + len);

    std::memcpy(buf.data(), kMagic, 4);
    std::memcpy(buf.data() + 4, &kProtocolVersion, 2);
    std::memcpy(buf.data() + 6, &msg_type, 2);
    std::memcpy(buf.data() + 8, &turn_id, 4);
    std::memcpy(buf.data() + 12, &len, 4);
    if (len > 0) {
        std::memcpy(buf.data() + kHeaderSize, payload.data(), len);
    }
    return buf;
}

} // namespace jarvis::ipc
