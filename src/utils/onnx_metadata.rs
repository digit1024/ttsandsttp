//! ONNX model metadata utilities
//!
//! Adds missing metadata to ONNX model files.
//! Since ONNX files are Protocol Buffer format, modifying them requires
//! protobuf parsing. The simplest approach without adding heavy dependencies
//! is to use a system command or accept the warning (which is non-fatal).

use anyhow::{Context, Result};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

/// Add sample_rate metadata to an ONNX model file using a system command
///
/// This tries to use Python's onnx package if available, otherwise
/// the warning will remain (but the model will still work).
pub fn add_sample_rate_metadata(model_path: &Path, sample_rate: u32) -> Result<()> {
    // Check if Python with onnx is available
    let python_script = format!(
        r#"
import sys
try:
    import onnx
    model = onnx.load('{}')
    # Check if sample_rate already exists
    has_sample_rate = any(prop.key == 'sample_rate' for prop in model.metadata_props)
    if not has_sample_rate:
        meta = model.metadata_props.add()
        meta.key = 'sample_rate'
        meta.value = '{}'
        onnx.save(model, '{}')
        print('Added sample_rate metadata')
    else:
        print('sample_rate metadata already exists')
    sys.exit(0)
except ImportError:
    print('onnx package not available', file=sys.stderr)
    sys.exit(1)
except Exception as e:
    print(f'Error: {{e}}', file=sys.stderr)
    sys.exit(1)
"#,
        model_path.display(),
        sample_rate,
        model_path.display()
    );

    let output = Command::new("python3")
        .arg("-c")
        .arg(&python_script)
        .output()
        .context("Failed to run Python command")?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If onnx is not available, that's okay - the warning is non-fatal
        if stderr.contains("onnx package not available") {
            eprintln!("⚠️  Python 'onnx' package not available. The warning is non-fatal - model will use default sample_rate (22050 Hz)");
            Ok(())
        } else {
            anyhow::bail!("Failed to add metadata: {}", stderr);
        }
    }
}

/// Check if sample_rate metadata exists in ONNX file
pub fn has_sample_rate_metadata(model_path: &Path) -> Result<bool> {
    // Try using Python first (more reliable)
    let python_script = format!(
        r#"
import sys
try:
    import onnx
    model = onnx.load('{}')
    has_sample_rate = any(prop.key == 'sample_rate' for prop in model.metadata_props)
    print('true' if has_sample_rate else 'false')
    sys.exit(0)
except ImportError:
    # Fallback: simple binary check
    with open('{}', 'rb') as f:
        data = f.read(8192)
        has_metadata = b'sample_rate' in data
        print('true' if has_metadata else 'false')
    sys.exit(0)
except Exception as e:
    print('false', file=sys.stderr)
    sys.exit(1)
"#,
        model_path.display(),
        model_path.display()
    );

    let output = Command::new("python3")
        .arg("-c")
        .arg(&python_script)
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            Ok(stdout.trim() == "true")
        }
        _ => {
            // Fallback: simple binary check
            let mut file = fs::File::open(model_path)
                .with_context(|| format!("Failed to open ONNX file: {}", model_path.display()))?;
            
            let mut buffer = vec![0u8; 8192];
            let bytes_read = file.read(&mut buffer)
                .context("Failed to read ONNX file")?;
            buffer.truncate(bytes_read);
            
            Ok(buffer.windows(11).any(|window| window == b"sample_rate"))
        }
    }
}

