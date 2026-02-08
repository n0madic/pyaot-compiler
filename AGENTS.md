# AGENTS.md

## Project Summary

This project is a **Python AOT (Ahead-of-Time) Compiler** implemented in Rust. It compiles a statically-typed subset of Python into native executables using the Cranelift backend. The compiler features a multi-stage pipeline including parsing, semantic analysis, type checking, and code generation with a precise garbage collector.

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
- **`typecheck/`** — Type inference and validation
- **`mir/`** — Mid-level IR with control flow graphs
- **`lowering/`** — HIR to MIR transformation
- **`codegen-cranelift/`** — Native code generation
- **`linker/`** — Linking with runtime library
- **`runtime/`** — Runtime support and garbage collector
- **`types/`** — Type system definitions
- **`diagnostics/`** — Error reporting
- **`utils/`** — Common utilities

## Quick Reference

```bash
# Build everything
cargo build --workspace --release

# Compile and run a Python program
./target/release/pyaot program.py -o /tmp/program --run

# Run all unit tests
cargo test --workspace

# Run integration tests
./test_examples.sh

# Format and lint
cargo fmt && cargo clippy --workspace
```

---

For detailed information on any topic, start with [CLAUDE.md](CLAUDE.md) and follow the documentation links above.
