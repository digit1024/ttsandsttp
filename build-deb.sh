#!/bin/bash
# Build script for creating Debian package

set -e

echo "🔨 Building Debian package for ttsandsttp..."
echo ""

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ] || [ ! -d "debian" ]; then
    echo "❌ Error: Must be run from project root directory"
    exit 1
fi

# Check for required tools
if ! command -v dpkg-buildpackage >/dev/null 2>&1; then
    echo "❌ Error: dpkg-buildpackage not found. Install with:"
    echo "   sudo apt-get install build-essential debhelper"
    exit 1
fi

# Check for Rust (cargo/rustc) - can be from rustup or system packages
if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
    echo "❌ Error: cargo and/or rustc not found in PATH"
    echo "   Install Rust with: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

echo "✅ Found Rust toolchain:"
echo "   cargo: $(which cargo)"
echo "   rustc: $(rustc --version)"
echo ""

# Clean previous builds
echo "🧹 Cleaning previous builds..."
rm -f ../ttsandsttp_*.deb ../ttsandsttp_*.dsc ../ttsandsttp_*.changes ../ttsandsttp_*.buildinfo ../ttsandsttp_*.tar.*

# Build the package
# Use -d flag to skip build dependency checks (rustup-installed Rust works fine)
echo "📦 Building package (skipping build dependency checks for rustup-installed Rust)..."
dpkg-buildpackage -b -uc -us -d

echo ""
echo "✅ Package built successfully!"
echo ""
echo "📦 Install with:"
echo "   sudo dpkg -i ../ttsandsttp_*.deb"
echo "   sudo apt-get install -f  # if there are missing dependencies"
echo ""
echo "🚀 Then enable the service:"
echo "   systemctl --user daemon-reload"
echo "   systemctl --user enable ttsandsttp.service"
echo "   systemctl --user start ttsandsttp.service"

