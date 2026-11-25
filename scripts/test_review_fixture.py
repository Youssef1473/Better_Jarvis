#!/usr/bin/env python3
"""
Test Suite for Review Fixture
Verifies acceptance criteria and exact boundary conditions for token validation.
Equality convention:
- current_timestamp <= expiry_timestamp is valid (True)
- current_timestamp > expiry_timestamp is expired (False)
"""

import sys
import os

# Add scripts directory to path if run standalone
sys.path.insert(0, os.path.dirname(__file__))
from review_fixture import validate_token

def test_token_boundary_conditions():
    valid_token_16 = "1234567890123456" # exactly 16 chars
    valid_token_long = "abcdefghijklmnopqrstuvwxyz1234567890"

    print("  [1] Testing timestamp boundary conditions (current vs expiry)...")
    # Boundary: current < expiry (unexpired) -> True
    assert validate_token(valid_token_16, expiry_timestamp=1000, current_timestamp=999) is True, \
        "Boundary failed: current < expiry must be valid"
    
    # Boundary: current == expiry (exact expiration instant: defined as unexpired/valid) -> True
    assert validate_token(valid_token_16, expiry_timestamp=1000, current_timestamp=1000) is True, \
        "Boundary failed: current == expiry must be valid per equality convention"
    
    # Boundary: current > expiry (expired) -> False
    assert validate_token(valid_token_16, expiry_timestamp=1000, current_timestamp=1001) is False, \
        "Boundary failed: current > expiry must be invalid"

    print("  [2] Testing token length boundary conditions...")
    # Boundary: len == 15 (one below threshold) -> False
    assert validate_token("123456789012345", expiry_timestamp=1000, current_timestamp=500) is False, \
        "Boundary failed: len 15 must be invalid"
    
    # Boundary: len == 16 (exact threshold) -> True
    assert validate_token("1234567890123456", expiry_timestamp=1000, current_timestamp=500) is True, \
        "Boundary failed: len 16 must be valid"
    
    # Boundary: len == 17 (one above threshold) -> True
    assert validate_token("12345678901234567", expiry_timestamp=1000, current_timestamp=500) is True, \
        "Boundary failed: len 17 must be valid"

    print("  [3] Testing invalid formats and types...")
    # None
    assert validate_token(None, expiry_timestamp=1000, current_timestamp=500) is False, \
        "None token must be invalid"
    # Empty string
    assert validate_token("", expiry_timestamp=1000, current_timestamp=500) is False, \
        "Empty string token must be invalid"
    # Integer
    assert validate_token(1234567890123456, expiry_timestamp=1000, current_timestamp=500) is False, \
        "Non-string integer token must be invalid"
    # List of chars
    assert validate_token(list("1234567890123456"), expiry_timestamp=1000, current_timestamp=500) is False, \
        "Non-string list token must be invalid"

if __name__ == "__main__":
    print("=== Running Comprehensive Review Fixture Boundary Verification ===")
    test_token_boundary_conditions()
    print("=== All Token Validation Boundary Tests Passed Successfully (10/10 checks)! ===")
