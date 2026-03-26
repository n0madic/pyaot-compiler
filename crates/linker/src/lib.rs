//! Linker for combining object files with runtime

#![forbid(unsafe_code)]

use pyaot_diagnostics::{CompilerError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Linker {
    runtime_lib: PathBuf,
    debug: bool,
}

impl Linker {
    pub fn new(runtime_lib: impl Into<PathBuf>) -> Self {
        Self {
            runtime_lib: runtime_lib.into(),
            debug: false,
        }
    }

    /// Create a new linker with debug flag
    pub fn with_debug(runtime_lib: impl Into<PathBuf>, debug: bool) -> Self {
        Self {
            runtime_lib: runtime_lib.into(),
            debug,
        }
    }

    /// Link object file with runtime to create executable
    pub fn link(&self, object_file: &Path, output: &Path) -> Result<()> {
        // Determine linker command
        #[cfg(target_os = "linux")]
        let linker = "gcc";
        #[cfg(target_os = "macos")]
        let linker = "clang";
        #[cfg(target_os = "windows")]
        let linker = "link.exe";

        let mut cmd = Command::new(linker);
        cmd.arg(object_file);

        // Add runtime library if it exists
        if self.runtime_lib.exists() {
            cmd.arg(&self.runtime_lib);
        }

        cmd.arg("-o").arg(output);

        // Add platform-specific flags
        #[cfg(target_os = "linux")]
        {
            if !self.debug {
                cmd.arg("-s"); // Strip debug symbols
                cmd.arg("-Wl,--gc-sections"); // Remove unused sections (like macOS -dead_strip)
            }
            cmd.arg("-lm"); // Math library
            cmd.arg("-lpthread"); // Thread library
            cmd.arg("-ldl"); // Dynamic linking library
        }

        #[cfg(target_os = "macos")]
        {
            if self.debug {
                // Debug mode: preserve symbols, don't strip
                cmd.arg("-Wl,-dead_strip"); // Only remove truly dead code
            } else {
                // Release mode: strip all symbols and dead code
                cmd.arg("-Wl,-x,-S,-dead_strip"); // Strip all local/debug symbols and dead code
            }
            cmd.arg("-lSystem");
        }

        let link_output = cmd
            .output()
            .map_err(|e| CompilerError::link_error(format!("Failed to execute linker: {}", e)))?;

        if !link_output.status.success() {
            let stderr = String::from_utf8_lossy(&link_output.stderr);
            return Err(CompilerError::link_error(format!(
                "Link failed: {}",
                stderr
            )));
        }

        // Post-link strip for maximum size reduction (removes symbol table entries
        // that the linker's -Wl,-x,-S cannot remove)
        if !self.debug {
            let _ = Command::new("strip").arg(output).output();
        }

        // On macOS with debug info, run dsymutil to create .dSYM bundle
        // dsymutil must run BEFORE deleting the .o file (it reads debug map from it)
        #[cfg(target_os = "macos")]
        if self.debug {
            let dsym_output = Command::new("dsymutil").arg(output).output();
            match dsym_output {
                Ok(result) if !result.status.success() => {
                    eprintln!(
                        "Warning: dsymutil failed: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
                Err(e) => {
                    eprintln!("Warning: failed to run dsymutil: {}", e);
                }
                _ => {}
            }
        }

        // Clean up the object file after successful linking (and dsymutil)
        // On macOS debug builds, keep the .o file — dsymutil references it from the debug map
        let should_keep_object = cfg!(target_os = "macos") && self.debug;
        if !should_keep_object {
            if let Err(e) = fs::remove_file(object_file) {
                eprintln!(
                    "Warning: Failed to remove object file {}: {}",
                    object_file.display(),
                    e
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linker_creation() {
        let linker = Linker::new("libruntime.a");
        assert_eq!(linker.runtime_lib, PathBuf::from("libruntime.a"));
    }
}
