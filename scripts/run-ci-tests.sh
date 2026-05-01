#!/bin/bash
# Run CI tests locally
# This script replicates the exact test commands run in GitHub Actions CI

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Set environment variables to match CI
export CARGO_TERM_COLOR=always
export RUST_BACKTRACE=1
export RUSTFLAGS="-D warnings"  # Makes all compiler warnings into errors (matches CI)

echo -e "${GREEN}Running CI tests locally...${NC}"
echo ""

# Step 1: Check formatting
echo -e "${YELLOW}[1/6] Checking code formatting...${NC}"
if cargo fmt --all -- --check; then
    echo -e "${GREEN}✓ Formatting check passed${NC}"
else
    echo -e "${RED}✗ Formatting check failed. Run 'cargo fmt --all' to fix.${NC}"
    exit 1
fi
echo ""

# Step 2: Run Clippy
echo -e "${YELLOW}[2/6] Running Clippy lints...${NC}"
if cargo clippy -- -D warnings; then
    echo -e "${GREEN}✓ Clippy check passed${NC}"
else
    echo -e "${RED}✗ Clippy check failed${NC}"
    exit 1
fi
echo ""

# Step 3: Run cargo audit
echo -e "${YELLOW}[3/6] Running cargo audit...${NC}"
if cargo audit; then
    echo -e "${GREEN}✓ No known vulnerabilities${NC}"
else
    echo -e "${RED}✗ cargo audit found advisories${NC}"
    exit 1
fi
echo ""

# Step 4: Build release binary
echo -e "${YELLOW}[4/6] Building release binary...${NC}"
if cargo build --release --verbose; then
    echo -e "${GREEN}✓ Release build succeeded${NC}"
else
    echo -e "${RED}✗ Release build failed${NC}"
    exit 1
fi
echo ""

# Step 5: Run all tests
echo -e "${YELLOW}[5/6] Running all tests...${NC}"
if cargo test --verbose; then
    echo -e "${GREEN}✓ All tests passed${NC}"
else
    echo -e "${RED}✗ Tests failed${NC}"
    exit 1
fi
echo ""

# Step 6: Run integration tests
echo -e "${YELLOW}[6/6] Running integration tests...${NC}"
if cargo test --test '*' --verbose; then
    echo -e "${GREEN}✓ Integration tests passed${NC}"
else
    echo -e "${RED}✗ Integration tests failed${NC}"
    exit 1
fi
echo ""

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}All CI checks passed! ✓${NC}"
echo -e "${GREEN}========================================${NC}"

