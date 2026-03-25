# AGENTS.md

## Project Summary

This project is a **Python AOT (Ahead-of-Time) Compiler** implemented in Rust. It compiles a statically-typed subset of Python into native executables using the Cranelift backend. The compiler features a multi-stage pipeline including parsing, semantic analysis, type inference, HIR-to-MIR lowering, and code generation with a precise garbage collector.

**Key Highlights:**
- Generates standalone native executables from Python code
- Requires full static type annotations
- Implements a shadow-stack based mark-sweep garbage collector
- Built entirely in safe Rust (except runtime FFI boundaries)
- CPython-compatible behavior for the supported subset
- Uses Cranelift for fast, portable code generation

## Documentation Guide

For AI agents and developers getting started with this codebase, please review the documentation in the following order:

### 1. [CLAUDE.md](CLAUDE.md) — **Start Here**
Main development guide containing:
- Project overview and goals
- Key development principles
- Build commands and workflow
- Coding conventions
- Architecture overview
- Guidelines for working with the codebase

### 2. [README.md](README.md)
User-facing documentation with:
- Quick start guide
- Architecture overview
- Feature list
- Build and usage instructions
- Example code

### 3. [COMPILER_STATUS.md](COMPILER_STATUS.md)
Detailed implementation status:
- Complete feature matrix
- Architecture diagrams
- Implementation details for each compiler stage
- Supported Python features and limitations
- Test coverage information

### 4. [CONVENTIONS.md](CONVENTIONS.md)
Coding standards and patterns:
- Code style guidelines
- Naming conventions
- Module organization
- Testing practices
- Documentation requirements

## Project Structure

The compiler is organized as a Rust workspace with the following key crates:

- **`cli/`** — Command-line interface
- **`frontend-python/`** — Python parser and AST to HIR conversion
- **`hir/`** — High-level intermediate representation
- **`semantics/`** — Name resolution and scope analysis
- **`lowering/`** — HIR to MIR transformation (includes `type_planning/` for type inference)
- **`mir/`** — Mid-level IR with control flow graphs
- **`optimizer/`** — MIR optimization passes (function inlining)
- **`codegen-cranelift/`** — Native code generation
- **`linker/`** — Linking with runtime library
- **`runtime/`** — Runtime support and garbage collector
- **`types/`** — Type system definitions
- **`core-defs/`** — Shared definitions (exceptions, type tags) — leaf crate
- **`stdlib-defs/`** — Stdlib module definitions (declarative)
- **`diagnostics/`** — Error reporting
- **`utils/`** — Common utilities

## Quick Reference

```bash
# Build everything
cargo build --workspace --release

# Compile and run a Python program
./target/release/pyaot program.py -o /tmp/program --run

# Run all tests (unit + runtime integration)
cargo test --workspace

# Run only runtime integration tests
cargo test -p pyaot --test runtime

# Format and lint
cargo fmt && cargo clippy --workspace
```

---

For detailed information on any topic, start with [CLAUDE.md](CLAUDE.md) and follow the documentation links above.
