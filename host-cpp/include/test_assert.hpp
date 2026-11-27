#pragma once

#include <cstdlib>
#include <iostream>

// Always-active test assertion macro (never compiled out under -DNDEBUG).
#define TEST_ASSERT(cond)                                                     \
    do {                                                                      \
        if (!(cond)) {                                                        \
            std::cerr << "[TEST_ASSERT FAILED] " << #cond                     \
                      << " at " << __FILE__ << ":" << __LINE__ << std::endl;  \
            std::abort();                                                     \
        }                                                                     \
    } while (false)
