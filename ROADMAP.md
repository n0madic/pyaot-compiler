# ROADMAP

Development roadmap for the Python AOT Compiler. Items are grouped by area and roughly ordered by impact within each section. No timelines — priorities may shift based on real-world usage.

**Legend**: 🔴 High impact · 🟡 Medium impact · 🟢 Nice to have

---

## 1. Debugging & Diagnostics

### ✅ DWARF Debug Information (MVP — done)

**Implemented**: The `--debug` flag generates DWARF debug information:
- `Span` propagated from HIR → MIR → Cranelift via ambient span pattern in lowering context
- `LineMap` utility converts byte offsets to line+column (`crates/utils/src/line_map.rs`)
- `FunctionBuilder::set_srcloc()` called per instruction during codegen (stores byte offset for line+column)
- `gimli::write` generates `.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str` sections
- `DW_TAG_compile_unit` (Python language, pyaot producer) + `DW_TAG_subprogram` per user-defined function
- `DW_TAG_formal_parameter` with `DW_AT_name` and `DW_AT_type` for each function parameter
- `DW_TAG_base_type` for `int`, `float`, `bool`, `str` with correct sizes and DWARF encodings
- Compiler-internal functions (`__pyaot_*`, `__module_*`) filtered from DWARF output
- macOS: `dsymutil` runs automatically after linking; `.o` file preserved for debug map

**What remains (follow-up work)**:

### 🟡 DWARF Variable Location Tracking

**Why**: Function parameters and their types are now visible in DWARF (`DW_TAG_formal_parameter` with `DW_AT_name` and `DW_AT_type`), but without `DW_AT_location` the debugger can't print their values at runtime (`p my_var` shows "no location"). Local variables similarly need `DW_TAG_variable` entries with locations.

**What's done**: Parameter names/types, base type definitions (`int`, `float`, `bool`, `str`).

**What remains**: Track where each variable lives (register or stack slot) at each code address. Cranelift's `ValueLabelsRanges` in `CompiledCode` maps `ValueLabel → Vec<ValueLocRange { loc: Reg|CFAOffset, start, end }>`. To use it:
1. Call `func.dfg.collect_debug_info()` before codegen to enable value label tracking
2. Call `builder.set_val_label(value, label)` after each `def_var` / `use_var`
3. Read `compiled_code.value_labels_ranges` after compilation
4. Encode as DWARF `DW_AT_location` with `DW_OP_reg*` / `DW_OP_fbreg` operations (architecture-dependent: ARM64 vs x86-64)

**Complexity**: Medium-high. Steps 1-3 are straightforward; step 4 requires platform-specific register mapping.

### 🟡 Multi-File DWARF

**Why**: In multi-module compilation, only the main source file gets debug info. Functions from imported modules have no DWARF entries.

**Implementation plan**: Each module needs its own `SourceInfo` (filename + source). During `MirMerger`, propagate per-module source info alongside the MIR. In codegen, track `file_index` per MIR function and register multiple files in the DWARF line program. The `SourceLoc` encoding could use `file_index << 20 | line_number` for multi-file support.

**Complexity**: Medium. The DWARF generation already supports multiple files; the work is plumbing source info through the multi-module pipeline.

### 🟡 macOS Source-Level Breakpoints

**Why**: On macOS, `lldb` source-level breakpoints (`b file.py:10`) don't work because the macOS linker doesn't copy `__DWARF` sections from `.o` files to the executable. It relies on debug maps (STABS entries) + `dsymutil`. Cranelift doesn't generate STABS entries.

**Possible approaches**:
1. **Generate N_OSO stab entries** in the object file so `dsymutil` can find the `.o` file and extract DWARF
2. **Embed DWARF in the executable directly** by adding sections to the linked binary after linking (post-processing)
3. **Use `ld -r` (relocatable link)** to merge DWARF sections before final linking

Function-name breakpoints (`b add`) already work on macOS. Source-level breakpoints work on Linux (ELF embeds DWARF directly).

---

## 2. Optimizations

### 🟡 Escape Analysis + Stack Allocation

**Why**: In allocation-heavy tight loops, heap allocation still accounts for 50-75% of execution time even with the slab allocator. Stack-allocating non-escaping temporaries would eliminate this overhead entirely.

**Current state**: All heap types go through `gc_alloc` → slab allocator (≤64 bytes) or system malloc (>64 bytes). Slab bump allocation costs ~5ns per object. Stack allocation would cost ~0ns (just a stack pointer adjustment) and eliminate GC root registration and sweep overhead.

**Measured overhead** (100K iterations):

| Scenario | Total time | Allocation overhead | Savings with escape analysis |
|----------|-----------|--------------------|-----------------------------|
| String concat loop | 4.6ms | 2.6ms (56%) | ~2.6ms → **1.5-2x faster** |
| Dict create loop | 7.9ms | 5.9ms (74%) | ~5.9ms → **2-3x faster** |

**Implementation plan**:
1. **MIR-level escape analysis**: For each allocation in a function, track whether the allocated value:
   - Is stored to a global/class attribute → escapes
   - Is passed to a function call (except known non-capturing builtins) → escapes
   - Is returned → escapes
   - Is stored only in locals and used within the function → does NOT escape
2. **Stack allocation lowering**: For non-escaping objects, replace `gc_alloc` with a stack `alloca` in Cranelift. The object still gets the same `ObjHeader` layout but lives on the stack. No GC root registration needed.
3. **Scope-limited optimization**: Start with the easy cases:
   - Temporary tuples (e.g., `for a, b in items:` creates a tuple per iteration)
   - Iterator objects from `range()`, `enumerate()`, `zip()` that don't escape the loop
   - String concatenation intermediates

**Expected impact**: 1.5-3x improvement on code with heavy temporary allocation in tight loops. The absolute time savings are modest (ms-level for 100K iterations) since the slab allocator already handles the common case efficiently.

**Complexity**: High. Requires a new MIR analysis pass.

### 🟡 Loop-Invariant Code Motion (LICM)

**Why**: Expressions computed inside a loop that produce the same result every iteration waste cycles. Moving them before the loop runs them once.

**Example**:
```python
for i in range(1000000):
    x = len(items) * 2  # len(items) doesn't change in the loop
    result += x
```

**Implementation plan**: Identify loop headers and back edges in the CFG. For each instruction in a loop body, check if all operands are defined outside the loop (or are themselves loop-invariant). If so, and the instruction is side-effect-free, hoist it to the loop preheader.

**Complexity**: Medium. Requires loop detection (dominance frontiers or natural loop analysis) in MIR.

---

## 3. Runtime Performance

### 🟡 Generational GC

**Why**: The current mark-sweep GC scans ALL slab pages and large objects every collection. For programs with many long-lived objects and few short-lived allocations, this is wasteful. A generational approach (nursery + old gen) would scope minor collections to the nursery only.

**Current state**: Single-generation mark-sweep with slab allocator. The slab already provides bump-pointer allocation (similar to a nursery), but sweep still iterates all pages.

**Implementation plan**:
1. **Nursery**: Repurpose the slab allocator's bump region as a nursery. Minor collection only scans nursery pages + remembered set.
2. **Promotion**: Objects surviving a minor collection move to the old generation (system malloc tracked in Vec).
3. **Write barrier**: When an old-gen object stores a reference to a nursery object, record it in a remembered set. Minor collection scans remembered set as additional roots.
4. **Full collection fallback**: If old-gen grows too large, do a full mark-sweep.

**Expected impact**: Reduced GC pause times for programs with many long-lived objects. For short-lived programs (current benchmarks), minimal benefit since collections are rare.

**Complexity**: High. Requires write barriers in codegen (every pointer store to a heap object must check generation boundary).

---

## 4. Language Features

---

### 🟢 `async`/`await` (Basic)

**Why**: Async I/O is increasingly important. Even basic coroutine support would open up I/O-bound use cases.

**Implementation challenges**: AOT-compiled coroutines are fundamentally different from generators (which use setjmp/longjmp). Options:
- **Stackful coroutines**: Each coroutine gets its own stack. Simple but memory-heavy.
- **State machine transformation**: Transform `async def` into a state machine (like Rust's async). Complex but efficient.
- **Integration with tokio/smol**: Use a Rust async runtime for I/O multiplexing.

**Complexity**: Very high. This is a long-term feature. Generator support is a stepping stone.

---

## 5. Standard Library

### 🟡 `itertools` Expansion

**Current**: `chain`, `islice` implemented.

**Missing** (high value):
- `zip_longest(iter1, iter2, fillvalue=None)` — common in data processing
- `product(*iterables)` — cartesian product
- `combinations(iterable, r)` / `permutations(iterable, r)` — combinatorics
- `groupby(iterable, key=None)` — grouping consecutive elements
- `starmap(func, iterable)` — like `map` but unpacks arguments
- `accumulate(iterable, func)` — running totals

**Already partially in COMPILER_STATUS.md roadmap.**

---

### ✅ `typing` Module Extensions (done)

**Implemented**:
- `TypeAlias` — PEP 613 (`x: TypeAlias = T`) and PEP 695 (`type X = T`) for type alias readability
- `Literal[value]` — type erasure to base type (`Literal[42]` → `int`, `Literal["r", "w"]` → `str`)
- `TypeVar` — type erasure: unconstrained → untyped (inference), constrained → Union, bounded → bound type
- `Protocol` — structural subtyping with name-based virtual dispatch (FNV-1a hash); works across different vtable layouts

**What remains (future)**:
- Structural type checking at call sites (verify methods/signatures match Protocol definition)
- Protocol inheritance (Protocol extending Protocol)

---

### 🟢 Additional Stdlib Modules

Low priority but occasionally needed:
- `pathlib` — path manipulation (can delegate to `os.path` runtime functions)
- `datetime` — date/time arithmetic (complex but commonly used)
- `csv` — CSV reading/writing
- `argparse` — argument parsing (complex; maybe support a simpler alternative)
- `enum` — enumerations (could map to classes with int constants)
- `dataclasses` — auto-generated `__init__`, `__repr__`, `__eq__` (syntactic sugar over existing class support)

---

## 6. Build System & Tooling

### 🔴 Incremental Compilation

**Why**: Recompiling the entire program for every change is wasteful. For multi-module projects, only changed modules should be recompiled.

**Implementation plan**:
1. **Module-level object files**: Compile each module to a separate `.o` file. Cache them with a hash of the source + dependencies.
2. **Dependency tracking**: Track which modules import which. If module A imports from B, and B changes, recompile both. If only A changes, recompile only A.
3. **Cache directory**: Store compiled `.o` files and metadata in a `.pyaot_cache/` directory.
4. **Linking**: Link all object files together with the runtime library. Only the link step runs every time.

**Complexity**: Medium. The compiler already handles multi-module compilation. The main work is caching infrastructure and dependency hashing.

---

### 🟡 Cross-Compilation

**Why**: Cranelift already supports multiple targets (x86-64, AArch64, RISC-V). Adding a `--target` flag would allow compiling for different platforms.

**Implementation plan**:
1. **Target triple parsing**: Accept `--target x86_64-unknown-linux-gnu` etc.
2. **Cranelift target configuration**: Pass the target to `isa::lookup()` instead of using `native`.
3. **Cross-runtime**: Pre-compile the runtime library for target platforms, or cross-compile it on demand.
4. **Cross-linker**: Use the appropriate linker for the target (e.g., `x86_64-linux-gnu-gcc`).

**Complexity**: Medium. Cranelift side is easy; the hard part is runtime cross-compilation and linker setup.

---

### ✅ Benchmark Suite Improvements (done)

- Memory usage benchmarks (peak RSS, allocation count)
- Benchmarks for specific optimizations (escape analysis, constant folding)

---

## 7. Architecture Improvements

### 🟡 Thread Safety Preparation

**Why**: The runtime uses `UnsafeCell`/`AtomicPtr` for zero-overhead single-threaded access. All synchronization was removed. Any future work on parallelism requires re-adding thread-safe primitives.

**Implementation plan** (no need to implement threading yet, but prepare the architecture):
1. **Thread-local shadow stacks**: Replace global `stack_top` with `thread_local!` storage.
2. **Per-thread nursery**: Each thread gets its own slab allocator for allocation without locking.
3. **Stop-the-world for major GC**: When a full collection is needed, pause all threads.

**Complexity**: High. Invasive change to the runtime. But preparation (thread-local storage) can be done incrementally.

---

### 🟡 MIR Verification Pass

**Why**: MIR bugs (type mismatches, missing GC roots, dangling references) cause hard-to-debug segfaults at runtime. A verification pass would catch them at compile time.

**Targets**:
- Type consistency: every instruction's operand types match its expected signature
- GC root completeness: every heap-allocated local is registered as a GC root
- CFG well-formedness: every block has a terminator, no unreachable blocks (after DCE)
- Shadow stack balance: every function entry has a matching exit on all paths

**Complexity**: Medium. A standalone MIR pass that runs in debug builds or with `--verify`.

---

### 🟢 Intermediate Representation Serialization

**Why**: Serializing MIR to disk enables:
- Incremental compilation (cache compiled modules)
- Debugging tools (inspect MIR offline)
- Future link-time optimization (LTO) across modules

**Implementation**: Derive `Serialize`/`Deserialize` on MIR types (most are simple enums/structs). Use `bincode` for compact binary format.

---

## 8. Code TODOs from Source

These are specific issues found in the codebase that should be addressed:

| Location | Issue | Status |
|----------|-------|--------|
| `lowering/type_planning/pre_scan.rs:433` | Decorated function ID found but unused — should link decorated function to its wrapper for better type inference | 🟢 Open |
| `lowering/expressions/calls.rs:101` | Full list unpacking for all call paths not yet complete | 🟢 Open |

Previously resolved:

| Location | Issue | Resolved In |
|----------|-------|-------------|
| `lowering/expressions/operators.rs` | List ordering comparisons (`<`, `<=`, `>`, `>=`) | ✅ `3b39c77` — lexicographic comparison via `rt_list_lt/lte/gt/gte` |
| `lowering/generators/utils.rs` | Truthiness check for heap types in generators | ✅ `2e5b6ae` — proper truthiness via `convert_to_bool_in_block` |
| `frontend-python/ast_to_hir/statements/context_managers.rs` | `__exit__` receives `(0,0,0)` instead of exception info | ✅ `6104e35` — real exception objects passed |
