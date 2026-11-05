//! Linker for combining object files with runtime

#![forbid(unsafe_code)]

use pyaot_diagnostics::{CompilerError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Linker {
    runtime_lib: PathBuf,
    debug: bool,
    /// Run a second, post-link `strip` pass for minimal binary size. Off by
    /// default: the link step already strips fully, so this only trims the
    /// residual global symbol table (~7%) at the cost of a whole-binary rewrite
    /// in a fresh process (~8-10ms) on every compile. Opt-in for size-conscious
    /// builds (`--opt-level speed-and-size`); the default fast path skips it.
    minimize_size: bool,
}

impl Linker {
    /// Create a new linker with debug flag
    pub fn with_debug(runtime_lib: impl Into<PathBuf>, debug: bool) -> Self {
        Self {
            runtime_lib: runtime_lib.into(),
            debug,
            minimize_size: false,
        }
    }

    /// Enable the extra post-link `strip` pass for minimal binary size.
    pub fn minimize_size(mut self, yes: bool) -> Self {
        self.minimize_size = yes;
        self
    }

    /// Build the linker invocation (without running it).
    ///
    /// Argument order matters for single-pass archive linkers (GNU ld): an
    /// archive only contributes members that resolve symbols *already*
    /// undefined when it is scanned. So the order is object file → package
    /// archives (which may reference runtime symbols) → runtime archive →
    /// system libraries.
    ///
    /// Errors (instead of silently producing an unlinkable command) when the
    /// runtime library or a package archive is missing, or the host OS has no
    /// supported linker driver.
    pub fn command(
        &self,
        object_file: &Path,
        output: &Path,
        extra_archives: &[PathBuf],
    ) -> Result<Command> {
        if cfg!(windows) {
            return Err(CompilerError::link_error(
                "linking native executables is not supported on Windows yet \
                 (the runtime and linker driver target Unix-like systems)",
            ));
        }

        if !self.runtime_lib.is_file() {
            return Err(CompilerError::link_error(format!(
                "runtime library not found: {}. Build it with \
                 `cargo build -p pyaot-runtime`.",
                self.runtime_lib.display()
            )));
        }
        for archive in extra_archives {
            if !archive.is_file() {
                return Err(CompilerError::link_error(format!(
                    "package archive not found: {}. Build the package or check \
                     the runtime-lib directory.",
                    archive.display()
                )));
            }
        }

        // The C compiler driver supplies the platform's crt/libc linkage.
        let driver = if cfg!(target_os = "macos") {
            "clang"
        } else if cfg!(target_os = "linux") {
            "gcc"
        } else {
            "cc"
        };
        let mut cmd = Command::new(driver);
        cmd.arg(object_file);
        cmd.args(extra_archives);
        cmd.arg(&self.runtime_lib);
        cmd.arg("-o").arg(output);

        if cfg!(target_os = "macos") {
            if self.debug {
                // Debug mode: preserve symbols, only remove truly dead code.
                cmd.arg("-Wl,-dead_strip");
            } else {
                // Release mode: strip all local/debug symbols and dead code.
                cmd.arg("-Wl,-x,-S,-dead_strip");
            }
            cmd.arg("-lSystem");
            // `rustls-platform-verifier` (enabled via `stdlib-network`) talks
            // to the macOS trust store through `security-framework`, which
            // pulls in both Security.framework and CoreFoundation.framework.
            cmd.arg("-framework").arg("Security");
            cmd.arg("-framework").arg("CoreFoundation");
        } else {
            // Linux and other ELF Unix-likes (GNU ld / lld / BSD ld).
            if !self.debug {
                cmd.arg("-s"); // Strip debug symbols
                cmd.arg("-Wl,--gc-sections"); // Remove unused sections
            }
            cmd.arg("-lm"); // Math library
            cmd.arg("-lpthread"); // Thread library
            if cfg!(target_os = "linux") {
                // `dlopen` & co. live in libc on the BSDs; glibc < 2.34 needs -ldl.
                cmd.arg("-ldl");
            }
        }
        Ok(cmd)
    }

    /// Link object file with runtime to create executable.
    ///
    /// `extra_archives` are additional `.a` files (typically per-package
    /// static libraries like `libpyaot_pkg_requests.a`) that are passed to the
    /// linker *before* the runtime archive (see [`Linker::command`]). They are
    /// included only when the compiled source references the corresponding
    /// package.
    pub fn link(
        &self,
        object_file: &Path,
        output: &Path,
        extra_archives: &[PathBuf],
    ) -> Result<()> {
        let mut cmd = self.command(object_file, output, extra_archives)?;

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

        // Post-link `strip` for minimal size — opt-in only (see `minimize_size`).
        // The link step above already strips (`-Wl,-x,-S,-dead_strip` /
        // `-s --gc-sections`); this second pass only trims the residual global
        // symbol table (~7%) and costs ~8-10ms (a whole-binary rewrite in a fresh
        // process), so the default fast path skips it.
        if !self.debug && self.minimize_size {
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
        let linker = Linker::with_debug("libruntime.a", false);
        assert_eq!(linker.runtime_lib, PathBuf::from("libruntime.a"));
    }

    /// A scratch directory with empty stand-in archive files.
    fn scratch(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pyaot_linker_{name}_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for f in files {
            fs::write(dir.join(f), b"").unwrap();
        }
        dir
    }

    #[test]
    #[cfg(unix)]
    fn package_archives_precede_the_runtime_archive() {
        let dir = scratch("order", &["libpyaot_runtime.a", "libpkg.a"]);
        let runtime = dir.join("libpyaot_runtime.a");
        let pkg = dir.join("libpkg.a");
        let linker = Linker::with_debug(&runtime, false);
        let cmd = linker
            .command(
                Path::new("main.o"),
                Path::new("main"),
                std::slice::from_ref(&pkg),
            )
            .expect("command builds");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_owned()).collect();
        let pos = |p: &Path| args.iter().position(|a| a == p.as_os_str()).unwrap();
        assert!(pos(Path::new("main.o")) < pos(&pkg), "{args:?}");
        assert!(pos(&pkg) < pos(&runtime), "GNU ld is single-pass: {args:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn missing_runtime_library_is_a_clear_error() {
        let dir = scratch("missing_rt", &[]);
        let linker = Linker::with_debug(dir.join("libpyaot_runtime.a"), false);
        let err = linker
            .command(Path::new("main.o"), Path::new("main"), &[])
            .expect_err("a missing runtime must not be silently skipped");
        assert!(
            err.to_string().contains("runtime library not found"),
            "{err}"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn missing_package_archive_is_an_error() {
        let dir = scratch("missing_pkg", &["libpyaot_runtime.a"]);
        let linker = Linker::with_debug(dir.join("libpyaot_runtime.a"), false);
        let err = linker
            .command(
                Path::new("main.o"),
                Path::new("main"),
                &[dir.join("nope.a")],
            )
            .expect_err("a missing package archive is an error");
        assert!(
            err.to_string().contains("package archive not found"),
            "{err}"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
