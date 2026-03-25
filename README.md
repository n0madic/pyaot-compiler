# Python AOT Compiler

An ahead-of-time (AOT) compiler for a statically-typed subset of Python, implemented in Rust.

## Quick Start

```bash
# Build the compiler
cargo build --workspace --release

# Create a Python file
cat > hello.py << 'EOF'
x: int = 42
y: int = 13
assert x + y == 55, "Math is broken!"
EOF

# Compile and run in one command
pyaot hello.py -o hello --run
# Exit code: 0 (assertions passed)

# Or compile first, then run
pyaot hello.py -o hello
./hello
```

## Architecture

This compiler follows a multi-stage compilation pipeline:

1. **Frontend** (`frontend-python`): Python parsing and AST to HIR conversion
2. **HIR** (`hir`): High-level intermediate representation
3. **Semantic Analysis** (`semantics`): Name resolution and scope checking
4. **Type Inference** (`lowering/type_planning`): Bidirectional type inference and validation
5. **Lowering** (`lowering`): HIR to MIR transformation
6. **MIR** (`mir`): Mid-level IR with control flow graph (CFG)
7. **Optimizer** (`optimizer`): MIR optimization passes (function inlining)
8. **Codegen** (`codegen-cranelift`): Code generation using Cranelift
9. **Linking** (`linker`): Linking with runtime library
10. **Runtime** (`runtime`): Runtime support with precise GC

## Features

- **Static typing**: Full type annotations required
- **Precise GC**: Shadow-stack based mark-sweep garbage collector
- **AOT compilation**: Generates native executables
- **Safe Rust**: Compiler is implemented entirely in safe Rust (except runtime FFI)
- **Cranelift backend**: Fast, portable code generation
- **Standard library**: 20+ modules implemented in Rust

## Building

```bash
# Build the entire workspace
cargo build --workspace --release

# Build the runtime library
cargo build -p pyaot-runtime --release

# Build the CLI
cargo build -p pyaot --release
```

## Usage

```bash
# Compile a Python file
pyaot input.py -o output

# Compile and run immediately
pyaot input.py -o output --run

# With verbose output
pyaot input.py -o output --verbose

# Compile with function inlining optimization
pyaot input.py -o output --inline

# Compile with debug information (DWARF line tables, symbols preserved)
pyaot input.py -o output --debug

# Compile with module search paths (for imports)
pyaot input.py --module-path /path/to/libs -o output

# Emit intermediate representations (for debugging the compiler)
pyaot input.py --emit-hir
pyaot input.py --emit-mir
```

## Examples

The `examples/` directory contains test programs demonstrating compiler features. These are run as Rust integration tests via `cargo test`:

```bash
# Run all runtime integration tests
cargo test -p pyaot --test runtime

# Run a single example test
cargo test -p pyaot --test runtime runtime_builtins
```

You can also compile and run examples manually:
```bash
pyaot examples/test_builtins.py -o /tmp/test --run
```

All examples use `assert` statements for testing. Programs exit with code 0 on success, code 1 on assertion failure.

## Performance

The compiler excels at **computation-heavy workloads**, providing significant speedups over CPython for pure algorithmic code:

| Workload Type | Performance vs CPython |
|---------------|------------------------|
| Deep recursion (Fibonacci) | **7.76x faster** ✅ |
| Pure arithmetic | ~1x (equal) |
| Collection operations | 3-13x slower* |

*Collection operations (list, dict, string) are currently slower because CPython's highly optimized C implementations outperform the Rust runtime. Future optimizations will focus on improving these operations.

**Best use cases:**
- CPU-intensive numerical algorithms
- Deep recursion
- Scientific computing
- Long-running computational processes

See the [benchmarks](benchmarks/) directory for detailed performance analysis and comparison methodology.

## Supported Python Subset

For a complete and up-to-date list of supported features, see **[COMPILER_STATUS.md](COMPILER_STATUS.md)**.

Key highlights:
- **Types**: int, float, bool, str, bytes, None, list[T], tuple[T...], dict[K,V], set[T], Union types, Iterator[T], File
- **Operators**: Arithmetic, comparison, identity (is/is not), logical, bitwise, membership (in/not in), all augmented assignments
- **Control flow**: if/elif/else, while, for (with unpacking), break/continue, try/except/else/finally, match statements (incl. mapping patterns), with (context managers)
- **Functions**: Type-annotated functions, default parameters, *args/**kwargs, keyword-only params, lambda expressions, generators, user decorators
- **Classes**: Single inheritance, @property/@staticmethod/@classmethod/@abstractmethod, virtual dispatch, dunder methods (incl. explicit calls)
- **Built-ins**: print, len, range, map, filter, zip, sorted, enumerate, open, input, and 55+ total
- **Standard library**: 20+ modules
- **String operations**: F-strings with format specs (width, alignment, fill, grouping), slicing, string interning, 40+ methods
- **Collections**: Full list/dict/tuple/set/bytes support with comprehensive methods

## Type System

The compiler uses a structural type system with:

- **Primitives**: `int`, `float`, `bool`, `str`, `None`
- **Generics**: `list[T]`, `dict[K,V]`, `tuple[T1, ..., Tn]`
- **Union types**: `T | U` or `Union[T, U]`
- **Optional**: `Optional[T]` (sugar for `T | None`)
- **Function types**: `(T1, T2) -> R`

## Garbage Collection

The runtime uses a precise mark-sweep garbage collector with shadow stack:

- **Shadow stack**: Explicit root tracking (no conservative scanning)
- **Mark-sweep**: Simple, predictable collection
- **Precise**: Compiler knows exact locations of all pointers

### Shadow Stack Protocol

Each compiled function:
1. Allocates a shadow frame on entry
2. Registers GC roots (local variables holding heap objects)
3. Updates roots before any allocation
4. Unregisters frame on exit

## Debug Information

The `--debug` flag generates DWARF debug information for source-level debugging:

```bash
pyaot program.py -o program --debug

# Inspect DWARF sections in the object file
dwarfdump program.o

# Debug with lldb (set breakpoint on function name)
lldb -o "b my_function" -o "r" program
```

**What `--debug` provides:**
- DWARF sections (`.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str`) in object files
- Source line+column mappings: machine code addresses mapped to Python source lines with column precision
- Function entries (`DW_TAG_subprogram`) with declaration file and line; compiler internals filtered out
- Function parameter info (`DW_TAG_formal_parameter`) with names and type references
- Base type definitions (`DW_TAG_base_type`) for `int`, `float`, `bool`, `str`
- Preserved symbols and frame pointers, optimizations disabled
- macOS: automatic `dsymutil` invocation and `.o` file preservation

**Limitations:**
- Variable locations not tracked — names and types appear in DWARF, but debugger can't print values (`p my_var` won't work)
- Only the main source file gets debug info (imported modules excluded)
- macOS: source-level breakpoints (`b file.py:10`) require the `.o` file to remain available; function-name breakpoints (`b func_name`) work without it
- Linux (ELF): DWARF sections are embedded in the executable — source-level breakpoints work directly

## Runtime Library

The runtime (`libpyaot_runtime.a`) provides:

- Object representation and allocation
- Garbage collection (shadow-stack based mark-sweep)
- String operations (concat, slice, methods, interning)
- Collection operations (list, tuple, dict, set, bytes)
- Exception handling (setjmp/longjmp based)
- Iterator support (list, tuple, dict, range, generators)
- File I/O operations
- Standard library implementations (json, re, os, sys, time, random, subprocess, hashlib, functools, itertools, io, copy, abc, base64, urllib, string)

## Safety

The compiler is implemented in safe Rust:
- `#![forbid(unsafe_code)]` in all compiler crates
- Only the `runtime` crate uses `unsafe` (for FFI and memory management)

## Example Programs

### Factorial (recursion)
```python
def factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)

result: int = factorial(5)
assert result == 120, "5! should be 120"
```

### List operations
```python
nums: list[int] = [1, 2, 3, 4, 5]
nums.append(6)
nums.reverse()
assert nums[0] == 6
assert len(nums) == 6

# Slicing
evens: list[int] = nums[::2]
assert len(evens) == 3
```

### Class with methods
```python
class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def distance_squared(self) -> int:
        return self.x * self.x + self.y * self.y

p = Point(3, 4)
assert p.distance_squared() == 25
```

### Module imports
```python
# utils.py
def add(a: int, b: int) -> int:
    return a + b

# main.py
from utils import add
result: int = add(2, 3)
assert result == 5
```

Compile and run:
```bash
pyaot example.py -o example --run
# Exit code: 0 (success)
```

## Testing

```bash
# Run all tests (unit + integration)
cargo test --workspace

# Run unit tests for a specific crate
cargo test -p pyaot-types

# Run only runtime integration tests (compile + execute Python examples)
cargo test -p pyaot --test runtime

# Run a single runtime test
cargo test -p pyaot --test runtime runtime_classes
```

## Contributing

This project follows standard Rust development practices:

1. Run `cargo fmt` before committing
2. Ensure `cargo clippy` passes
3. Add tests for new features
4. Update documentation

## License

MIT
