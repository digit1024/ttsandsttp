# TTS and STT Service v1.1 Release Notes

## What's New

This release focuses on code quality improvements and better developer experience while maintaining the same simple, reliable functionality.

### Improvements

- **Better Logging**: Switched from basic logging to structured tracing for improved debugging and monitoring
- **Cleaner Codebase**: Major refactoring and code cleanup - removed duplicate logic and improved maintainability
- **Dependency Updates**: All dependencies updated to latest stable versions for better security and performance
- **Simplified Typing**: Consolidated typing system - now uses only `wrtype` for consistency
- **Better Signal Handling**: DBus signals are now properly exposed and handled
- **DRY Principle**: Beep functionality refactored to eliminate code duplication

### Technical Details

- Updated to latest Sherpa-rs, Tokio, and other core dependencies
- Improved error handling and logging throughout the codebase
- Better separation of concerns in service architecture
- Enhanced configuration management

### Compatibility

- Fully backward compatible - no breaking changes to DBus API
- Existing configurations continue to work without modification
- Same system requirements as previous version

### Installation

Same simple installation process:
```bash
sudo dpkg -i ttsandsttp_1.1.0-1_*.deb
sudo apt-get install -f
systemctl --user enable ttsandsttp.service
systemctl --user start ttsandsttp.service
```

Thanks for using TTS and STT Service!