#include "diagnostics/crash_handler.hpp"

#include "test_assert.hpp"
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <thread>
#include <vector>

namespace fs = std::filesystem;

void TestDirectoryCreationAndDumpHeader() {
    std::cout << "[TEST] Running TestDirectoryCreationAndDumpHeader...\n";

    fs::path temp_crash_dir = fs::temp_directory_path() / "jarvis_test_crashes_header";
    std::error_code ec;
    fs::remove_all(temp_crash_dir, ec);

    std::wstring dump_path = jarvis::diagnostics::WriteDiagnosticDump(temp_crash_dir.wstring());
    TEST_ASSERT(!dump_path.empty() && "WriteDiagnosticDump must return non-empty path");
    TEST_ASSERT(fs::exists(dump_path) && "Generated dump file must exist on disk");

    // Verify MDMP Magic Header (0x4D 0x44 0x4D 0x50 = "MDMP")
    std::ifstream dump_file(fs::path(dump_path), std::ios::binary);
    TEST_ASSERT(dump_file.is_open() && "Must be able to open dump file in binary mode");

    char header[4]{};
    dump_file.read(header, 4);
    TEST_ASSERT(dump_file.gcount() == 4 && "Must read at least 4 header bytes");
    TEST_ASSERT(header[0] == 'M' && header[1] == 'D' && header[2] == 'M' && header[3] == 'P' &&
           "Dump header must match MDMP magic signature (0x504D444D)");

    std::cout << "  [PASS] MDMP signature verified: " << header[0] << header[1] << header[2] << header[3] << "\n";
    dump_file.close();

    fs::remove_all(temp_crash_dir, ec);
    std::cout << "  [PASS] TestDirectoryCreationAndDumpHeader passed cleanly.\n";
}

void TestRetentionPruning() {
    std::cout << "[TEST] Running TestRetentionPruning...\n";

    fs::path temp_crash_dir = fs::temp_directory_path() / "jarvis_test_crashes_retention";
    std::error_code ec;
    fs::remove_all(temp_crash_dir, ec);
    fs::create_directories(temp_crash_dir, ec);

    // Create 8 dummy .dmp files with distinctly staggered last_write_times
    std::vector<fs::path> dummy_files;
    auto base_time = fs::file_time_type::clock::now();
    for (int i = 1; i <= 8; ++i) {
        fs::path p = temp_crash_dir / ("dummy_crash_" + std::to_string(i) + ".dmp");
        std::ofstream ofs(p, std::ios::binary);
        ofs << "DUMMY DUMP DATA " << i;
        ofs.close();

        // Stagger modification times by 10 seconds per file (file 1 is oldest, file 8 is newest)
        fs::last_write_time(p, base_time + std::chrono::seconds(10 * i), ec);
        dummy_files.push_back(p);
    }

    // Verify all 8 files exist
    size_t initial_count = 0;
    for (const auto& entry : fs::directory_iterator(temp_crash_dir, ec)) {
        if (entry.path().extension() == ".dmp") initial_count++;
    }
    TEST_ASSERT(initial_count == 8 && "Must have created exactly 8 dummy dump files");

    // Prune to 5
    jarvis::diagnostics::PruneCrashDumps(temp_crash_dir.wstring(), 5);

    size_t post_prune_count = 0;
    for (const auto& entry : fs::directory_iterator(temp_crash_dir, ec)) {
        if (entry.path().extension() == ".dmp") post_prune_count++;
    }
    TEST_ASSERT(post_prune_count == 5 && "Pruning must retain strictly the 5 newest files");

    // Verify oldest files (1, 2, 3) were pruned and newest (4, 5, 6, 7, 8) remain
    TEST_ASSERT(!fs::exists(dummy_files[0]) && "Oldest file 1 must be deleted");
    TEST_ASSERT(!fs::exists(dummy_files[1]) && "Oldest file 2 must be deleted");
    TEST_ASSERT(!fs::exists(dummy_files[2]) && "Oldest file 3 must be deleted");
    TEST_ASSERT(fs::exists(dummy_files[7]) && "Newest file 8 must be retained");
    TEST_ASSERT(fs::exists(dummy_files[6]) && "Newest file 7 must be retained");

    std::cout << "  [PASS] Exactly 5 newest dumps retained out of 8.\n";

    fs::remove_all(temp_crash_dir, ec);
    std::cout << "  [PASS] TestRetentionPruning passed cleanly.\n";
}

int main() {
    std::cout << "--- Running C++ Crash Handler & Minidump Tests ---\n";
    TestDirectoryCreationAndDumpHeader();
    TestRetentionPruning();
    std::cout << "--- All Crash Handler Tests Passed Successfully! ---\n";
    return 0;
}
