#!/bin/bash
# Debug script for STT with gdb and enhanced logging

set -e

echo "🔍 Debugging STT with enhanced logging..."
echo ""
echo "Environment variables:"
echo "  SHERPA_ONNX_LOG_LEVEL=DEBUG"
echo "  RUST_BACKTRACE=full"
echo "  RUST_LOG=debug"
echo ""

# Set environment variables for debugging
export SHERPA_ONNX_LOG_LEVEL=DEBUG
export RUST_BACKTRACE=full
export RUST_LOG=debug

# Set SKIP_DECODE=1 to test audio capture without calling decode (avoids C++ exception crash)
# This allows testing the 7-second wait and audio capture functionality
# Uncomment the line below to enable skip decode mode:
export SKIP_DECODE=0

# Check if gdb is available
if command -v gdb &> /dev/null; then
    echo "🐛 Running with gdb..."
    echo ""
    echo "Useful gdb commands:"
    echo "  (gdb) run -- stt --language en-US --pause-duration 2.0"
    echo "  (gdb) bt          # backtrace when it crashes"
    echo "  (gdb) info registers"
    echo "  (gdb) frame <N>   # switch to frame N"
    echo "  (gdb) print <var> # print variable"
    echo ""
    
    # Build first
    unset ARGV0
    cargo build
    
    # Run with gdb
    gdb --args target/debug/ttsandsttp stt --language en-US --pause-duration 2.0
else
    echo "⚠️  gdb not found, running without debugger..."
    echo "   Install with: sudo apt-get install gdb"
    echo ""
    
    # Run normally with enhanced logging
    unset ARGV0
    cargo run -- stt --language en-US --pause-duration 2.0
fi
