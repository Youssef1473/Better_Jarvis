#include "ipc/frame_codec.hpp"

#include "test_assert.hpp"
#include <iostream>
#include <vector>

void TestEncodeDecodeRoundTrip() {
    const std::string payload = "{\"method\":\"system.ping\",\"params\":{\"client\":\"test\"}}";
    const uint32_t turn_id = 42;

    auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, payload, turn_id);
    TEST_ASSERT(frame.size() == jarvis::ipc::kHeaderSize + payload.size());

    // Verify magic bytes
    TEST_ASSERT(frame[0] == 'J' && frame[1] == 'R' && frame[2] == 'V' && frame[3] == 'S');

    jarvis::ipc::Header hdr{};
    bool ok = jarvis::ipc::DecodeHeader(frame.data(), frame.size(), hdr);
    TEST_ASSERT(ok);
    TEST_ASSERT(hdr.version == jarvis::ipc::kProtocolVersion);
    TEST_ASSERT(hdr.msg_type == jarvis::ipc::kMsgTypeControlJson);
    TEST_ASSERT(hdr.turn_id == turn_id);
    TEST_ASSERT(hdr.payload_len == payload.size());

    std::string recovered_payload(reinterpret_cast<const char*>(frame.data() + jarvis::ipc::kHeaderSize), hdr.payload_len);
    TEST_ASSERT(recovered_payload == payload);

    std::cout << "[PASS] TestEncodeDecodeRoundTrip" << std::endl;
}

void TestDecodeHeaderTruncated() {
    uint8_t short_buf[10]{'J', 'R', 'V', 'S'};
    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(short_buf, 10, hdr));
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(nullptr, 16, hdr));

    std::cout << "[PASS] TestDecodeHeaderTruncated" << std::endl;
}

void TestDecodeHeaderBadMagic() {
    auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, "{}", 1);
    frame[0] = 'X'; // Corrupt magic

    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(frame.data(), frame.size(), hdr));

    std::cout << "[PASS] TestDecodeHeaderBadMagic" << std::endl;
}

void TestDecodeHeaderWrongVersion() {
    auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, "{}", 1);
    frame[4] = 3; // Corrupt version

    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(frame.data(), frame.size(), hdr));

    std::cout << "[PASS] TestDecodeHeaderWrongVersion" << std::endl;
}

void TestDecodeHeaderUnknownMsgType() {
    auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, "{}", 1);
    frame[6] = 0xFF; // Unknown msg type 0x00FF

    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(frame.data(), frame.size(), hdr));

    std::cout << "[PASS] TestDecodeHeaderUnknownMsgType" << std::endl;
}

void TestDecodeHeaderOversizedJson() {
    uint8_t buf[16]{};
    std::memcpy(buf, jarvis::ipc::kMagic, 4);
    uint16_t version = jarvis::ipc::kProtocolVersion;
    uint16_t msg_type = jarvis::ipc::kMsgTypeControlJson;
    uint32_t turn_id = 0;
    uint32_t oversized_len = static_cast<uint32_t>(jarvis::ipc::kMaxControlPayload + 1);

    std::memcpy(buf + 4, &version, 2);
    std::memcpy(buf + 6, &msg_type, 2);
    std::memcpy(buf + 8, &turn_id, 4);
    std::memcpy(buf + 12, &oversized_len, 4);

    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(buf, sizeof(buf), hdr));

    std::cout << "[PASS] TestDecodeHeaderOversizedJson" << std::endl;
}

void TestDecodeHeaderOversizedPcm() {
    uint8_t buf[16]{};
    std::memcpy(buf, jarvis::ipc::kMagic, 4);
    uint16_t version = jarvis::ipc::kProtocolVersion;
    uint16_t msg_type = jarvis::ipc::kMsgTypeAudioPcm;
    uint32_t turn_id = 0;
    uint32_t oversized_len = static_cast<uint32_t>(jarvis::ipc::kMaxAudioPayload + 1);

    std::memcpy(buf + 4, &version, 2);
    std::memcpy(buf + 6, &msg_type, 2);
    std::memcpy(buf + 8, &turn_id, 4);
    std::memcpy(buf + 12, &oversized_len, 4);

    jarvis::ipc::Header hdr{};
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(buf, sizeof(buf), hdr));

    std::cout << "[PASS] TestDecodeHeaderOversizedPcm" << std::endl;
}

void TestDecodeHeaderMaxU32NoOom() {
    uint8_t buf[16]{};
    std::memcpy(buf, jarvis::ipc::kMagic, 4);
    uint16_t version = jarvis::ipc::kProtocolVersion;
    uint16_t msg_type = jarvis::ipc::kMsgTypeControlJson;
    uint32_t turn_id = 0;
    uint32_t max_u32_len = 0xFFFFFFFF;

    std::memcpy(buf + 4, &version, 2);
    std::memcpy(buf + 6, &msg_type, 2);
    std::memcpy(buf + 8, &turn_id, 4);
    std::memcpy(buf + 12, &max_u32_len, 4);

    jarvis::ipc::Header hdr{};
    // Verification: Must reject without attempting allocation
    TEST_ASSERT(!jarvis::ipc::DecodeHeader(buf, sizeof(buf), hdr));

    std::cout << "[PASS] TestDecodeHeaderMaxU32NoOom" << std::endl;
}

void TestEncodeThrowsOnOversizedPayload() {
    std::string huge_payload(jarvis::ipc::kMaxControlPayload + 1, 'a');
    bool threw = false;
    try {
        auto frame = jarvis::ipc::EncodeFrame(jarvis::ipc::kMsgTypeControlJson, huge_payload);
    } catch (const std::length_error&) {
        threw = true;
    }
    TEST_ASSERT(threw);

    std::cout << "[PASS] TestEncodeThrowsOnOversizedPayload" << std::endl;
}

void TestEncodeThrowsOnUnknownMsgType() {
    bool threw = false;
    try {
        auto frame = jarvis::ipc::EncodeFrame(0x00FF, "{}");
    } catch (const std::invalid_argument&) {
        threw = true;
    }
    TEST_ASSERT(threw);

    std::cout << "[PASS] TestEncodeThrowsOnUnknownMsgType" << std::endl;
}

int main() {
    std::cout << "--- Running C++ JRVS v2 Frame Codec Tests ---" << std::endl;
    TestEncodeDecodeRoundTrip();
    TestDecodeHeaderTruncated();
    TestDecodeHeaderBadMagic();
    TestDecodeHeaderWrongVersion();
    TestDecodeHeaderUnknownMsgType();
    TestDecodeHeaderOversizedJson();
    TestDecodeHeaderOversizedPcm();
    TestDecodeHeaderMaxU32NoOom();
    TestEncodeThrowsOnOversizedPayload();
    TestEncodeThrowsOnUnknownMsgType();
    std::cout << "--- All 10 C++ Codec Tests Passed Successfully ---" << std::endl;
    return 0;
}
