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

### 🟢 Stack Traces / Tracebacks *(Implemented)*

Python-style tracebacks are now displayed for unhandled exceptions:
```
Traceback (most recent call last):
  File "main.py", line 10, in <module>
  File "main.py", line 7, in level1
  File "main.py", line 4, in level2
ZeroDivisionError: division by zero
```

**Implementation**: Codegen-level instrumentation (no MIR changes). `rt_stack_push`/`rt_stack_pop` emitted alongside GC prologue/epilogue. Always-on — overhead is a pointer bump on a pre-allocated 256-entry array. Traceback is captured at raise time and stored in `ExceptionObject`. Exception chaining preserves tracebacks. `ExceptionFrame` carries `traceback_depth` for correct unwinding on longjmp.

---

### 🟡 Improved Error Messages with Source Context

**Why**: Compiler errors currently show type names and descriptions but don't always point to the exact source location with context (like `rustc` does with `^^^` underlines).

**Implementation plan**: Extend the `diagnostics` crate to render source snippets with span highlighting. The `ariadne` or `miette` crates provide beautiful error rendering out of the box and integrate with byte-offset spans.

---

## 2. Optimizations

### 🔴 Escape Analysis + Stack Allocation

**Why**: The GC is the biggest runtime overhead. Many objects (especially small temporaries like tuples, short strings, iterators) are created and die within the same function. Allocating them on the stack instead of the heap eliminates GC pressure entirely.

**Current state**: All heap types go through `gc_alloc`. No escape analysis exists.

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

**Expected impact**: 2-5x improvement on code with heavy temporary allocation (loops creating short-lived objects). This is the single highest-impact optimization for collection benchmarks.

**Complexity**: High. Requires a new MIR analysis pass. But the payoff is enormous.

---

### 🔴 Constant Folding & Propagation

**Why**: Low-hanging fruit. Expressions like `x = 2 + 3`, `y = "hello" + " world"`, `n = len("abc")` can be evaluated at compile time. This reduces instruction count and enables further optimizations.

**Current state**: No constant folding exists. All arithmetic on literals generates runtime code.

**Implementation plan**:
1. **MIR constant folding pass**: Walk MIR instructions. For binary/unary ops where both operands are `Constant`, evaluate at compile time and replace with a single `Constant`. Handle:
   - Integer arithmetic: `+`, `-`, `*`, `//`, `%`, `**`, bitwise ops
   - Float arithmetic: `+`, `-`, `*`, `/`
   - String concatenation of literals
   - Boolean logic: `and`, `or`, `not` on constants
   - Comparison of constants
2. **Constant propagation**: If a local variable is assigned a constant and never reassigned, replace all uses with the constant value. This feeds back into folding (transitive constants).
3. **Builtin constant evaluation**: Evaluate pure builtins on constant args: `len("abc")` → `3`, `int("42")` → `42`, `abs(-5)` → `5`.

**Complexity**: Low-medium. A clean MIR pass with no architectural changes.

---

### 🟢 Dead Code Elimination (DCE) — Completed

**Why**: After constant folding and inlining, dead basic blocks and unused computations remain. Removing them reduces code size and improves cache behavior.

**Implementation** (`crates/optimizer/src/dce/`):
1. **Unreachable block elimination**: BFS from entry block, removes blocks not reachable via CFG edges.
2. **Dead instruction elimination**: Removes pure instructions (Const, Copy, FuncAddr, type conversions) whose results are never used. BinOp/UnOp are conservatively kept because they can raise OverflowError/ZeroDivisionError.
3. **Dead variable elimination**: Cleans up unused local variable entries from `func.locals`.

Iterates to fixpoint for cascading dead code removal. Enabled via `--dce` CLI flag.

---

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

### 🟡 Register Allocation Hints

**Why**: Cranelift handles register allocation automatically, but providing hints (e.g., keeping loop induction variables in registers, preferring specific registers for call arguments) can improve performance.

**Implementation plan**: Use Cranelift's `regalloc2` hints API to suggest register placement for hot variables. Profile-guided: identify frequently-accessed locals in loops.

**Complexity**: Low. Cranelift API already supports this.

---

### 🟢 Peephole Optimizations

**Why**: Small local patterns that Cranelift might miss:
- `x * 1` → `x`, `x + 0` → `x`, `x * 0` → `0`
- `x * 2` → `x << 1` (strength reduction)
- Consecutive `box`/`unbox` elimination

**Complexity**: Low. Can be added incrementally as patterns are discovered.

---

## 3. Runtime Performance

### 🔴 Collection Operations Optimization

**Why**: Benchmarks show 3-13x slower than CPython on collections. This is the biggest real-world performance gap.

**Current state**: Timsort, triangular probing, SplitMix64 hashing, and StringBuilder already implemented. But allocation overhead and GC pressure remain high.

**Specific targets**:

1. **Small String Optimization (SSO)**: Strings under ~22 bytes stored inline in the object header, no separate heap allocation. Most strings in real programs are short (variable names, keys, error messages). This eliminates a heap allocation + pointer indirection per small string.

2. **Custom allocator for small objects**: A slab/arena allocator for objects under 256 bytes (which covers most runtime objects). Reduces malloc overhead and improves cache locality. Could use a bump allocator for nursery-like behavior.

3. **Method dispatch optimization**: Currently vtable dispatch goes through an indirect call for every method. For monomorphic call sites (one type seen at runtime), inline caching or call-site specialization would eliminate the indirection.

4. **Dict key comparison fast path**: When both keys are interned strings, use pointer comparison instead of `memcmp`. This is already partially done for dict keys but could be extended to all string comparisons when both operands are known-interned.

5. **List growth factor tuning**: Profile real workloads to find optimal growth factor (currently likely 2x like `Vec`). CPython uses ~1.125x for large lists to reduce memory waste.

---

### 🟡 Generational GC

**Why**: Mark-sweep scans all live objects every collection. Most objects die young (generational hypothesis). A nursery generation with bump allocation + minor collection is much cheaper for short-lived objects.

**Current state**: Single-generation shadow-stack mark-sweep. Global mutex (uncontended, single-threaded). See INSIGHTS.md for GC architecture details.

**Implementation plan**:
1. **Nursery (young generation)**: A contiguous memory region with bump-pointer allocation. No free-list overhead, allocation is just a pointer increment.
2. **Minor collection**: When the nursery fills, scan shadow stack roots. Copy surviving nursery objects to the old generation. Dead nursery objects need no work (just reset the bump pointer).
3. **Write barrier**: When an old-gen object stores a reference to a nursery object, record it in a "remembered set". Minor collection scans remembered set as additional roots.
4. **Fallback to full collection**: If old-gen grows too large, do a full mark-sweep as today.

**Expected impact**: 2-10x improvement on allocation-heavy code. Nursery allocation (pointer bump) is essentially free compared to `malloc`.

**Complexity**: High. Requires write barriers in codegen (every pointer store to a heap object must check if it crosses generation boundary). But it's the standard approach for high-performance GCs.

---

### 🟡 Object Model Optimization

**Why**: Class instantiation is 13x slower than CPython. The current approach allocates a generic object and sets fields individually.

**Targets**:
1. **Inline instance fields**: Allocate instance as a single blob with fields at fixed offsets (already done via vtable). Optimize the allocation path to a single `gc_alloc` + field initialization without per-field function calls.
2. **Monomorphic inline caches**: At call sites that always see the same class, cache the vtable pointer and skip the lookup on subsequent calls.
3. **Flatten property access**: For simple `@property` getters that just return a field, inline the field access directly instead of going through a function call.

---

## 4. Language Features

### ✅ Full Exception Objects (done)

**Implemented**: `except E as e` now binds `e` to a heap-allocated exception instance:
- Built-in exceptions: lazy instance creation at catch time with `.args` tuple (field 0)
- Custom exceptions: eager instance creation at raise time via `lower_class_instantiation` — `__init__` is called, custom fields preserved
- `str(e)` extracts the message from `ExceptionState` (matching by instance pointer) or falls back to `.args[0]`
- `print(e)` works for both built-in and custom exception types
- `e.args` accessible on built-in exceptions (requires type annotation: `args: tuple[str] = e.args`)
- Custom fields accessible: `e.status`, `e.msg`, etc.
- GC root scanning of `ExceptionState` keeps exception instances alive across `longjmp`
- Fixed class ID space: all 28 built-in exceptions registered (0-27), `FIRST_USER_CLASS_ID = 28`

**What remains (not yet supported)**:
- `e.__class__.__name__` (requires class name attribute on instances)
- `e.__traceback__` (traceback stored in `ExceptionObject` but not exposed as attribute)
- `e.__cause__`, `e.__context__` (stored in `ExceptionObject` but not exposed as attributes)

---

### 🔴 List Ordering Comparisons

**Why**: `list1 < list2` (lexicographic comparison) doesn't work. This is a common Python pattern used in sorting, priority queues, and general comparisons. There is a TODO in `crates/lowering/src/expressions/operators.rs:731`.

**Implementation plan**: Add `rt_list_compare(a, b, op) -> bool` to the runtime that does element-wise comparison with short-circuit. Handle nested lists recursively. Support `<`, `<=`, `>`, `>=`, `==`, `!=`.

**Complexity**: Low-medium. Straightforward runtime function + codegen hookup.

---

### 🟡 Truthiness for Heap Types in Generators

**Why**: `not []`, `not {}` etc. inside generators incorrectly evaluate because generators use raw pointer comparison instead of calling truthiness. There is a TODO in `crates/lowering/src/generators/utils.rs:159`.

**Implementation plan**: Add `rt_truthiness(obj) -> bool` runtime function that dispatches on type tag: empty list/dict/set/string → false, zero int/float → false, None → false, else true. Use this instead of raw pointer comparison in generator lowering.

---

### 🟡 `__exit__` with Exception Info

**Why**: Context managers receive `(exc_type, exc_val, exc_tb)` in `__exit__`. Currently we pass `(0, 0, 0)` or `(1, 0, 0)`. This prevents context managers from inspecting the exception. See TODO in `crates/frontend-python/src/ast_to_hir/statements/context_managers.rs:292`.

**Implementation plan**: Pass the actual exception type tag and message to `__exit__`. Full traceback object is not needed initially — `exc_type` (as int class ID) and `exc_val` (as string message or exception object if feature #4.1 is done) are sufficient for most use cases.

---

### 🟡 Reverse Arithmetic Dunders

**Why**: `__radd__`, `__rsub__`, `__rmul__` etc. enable expressions like `2 + custom_obj` where the left operand doesn't know how to handle the right type.

**Implementation plan**: When the normal dunder (`__add__`) returns `NotImplemented` (or doesn't exist for the left operand's type), try the right operand's reverse dunder (`__radd__`). This requires a two-step dispatch in the operator lowering.

**Already in COMPILER_STATUS.md roadmap.**

---

### 🟡 More Unary Dunders: `__pos__`, `__abs__`, `__invert__`

**Why**: `+obj`, `abs(obj)`, `~obj` on custom classes. Straightforward to implement — same pattern as existing `__neg__`.

**Already in COMPILER_STATUS.md roadmap.**

---

### 🟡 Conversion Dunders: `__int__`, `__float__`, `__bool__`

**Why**: `int(obj)`, `float(obj)`, `bool(obj)` on custom classes. Needed for numeric-like user types.

**Already in COMPILER_STATUS.md roadmap.**

---

### 🟡 Match Statement: Class Patterns

**Why**: `case Point(x=0, y=y)` — matching against class instances by attribute values. Currently only literal, sequence, mapping, and OR patterns are supported.

**Implementation plan**: At match time, check `isinstance(subject, Point)` then extract attributes by name. Requires `getattr`-style field access in the match lowering.

**Already in COMPILER_STATUS.md roadmap.**

---

### 🟡 Container Dunders: `__len__` + MutableSequence Protocol

**Why**: User-defined classes that implement `__len__`, `__contains__` (already done), `__getitem__`/`__setitem__`/`__delitem__` (already done) should work with `len()`, `in`, indexing. `__len__` is the main missing piece — currently `len(custom_obj)` raises a compile error.

**Already in COMPILER_STATUS.md roadmap.**

---

### 🟢 Three-Level Closure Nesting

**Why**: Currently closures only capture from their immediate parent. A closure inside a closure inside a function cannot capture variables from the outermost function. See INSIGHTS.md "Closure Cell Variables".

**Implementation plan**: When a middle function captures a cell from its parent and an inner closure needs it, the middle function must forward the cell reference as part of the inner closure's capture tuple. Requires the frontend to transitively discover captured variables across nesting levels.

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

### 🟡 `collections` Module

**Targets**:
- `defaultdict(factory)` — very commonly used, avoids `setdefault` boilerplate
- `Counter(iterable)` — frequency counting, a common pattern
- `deque(iterable, maxlen=None)` — efficient double-ended queue
- `OrderedDict` — less critical since `dict` preserves order, but used in older code
- `namedtuple(name, fields)` — could map to compiled classes with named fields

---

### 🟡 `typing` Module Extensions

**Targets**:
- `TypeVar` — even limited support (e.g., for generic user functions) would improve ergonomics
- `Literal[value]` — restricts a type to specific literal values, useful for config/flags
- `TypeAlias` — `Name = SomeType` for readability
- `Protocol` — structural subtyping (check if an object has required methods without inheritance)

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

### 🟡 Language Server Protocol (LSP) Server

**Why**: IDE integration (VS Code, etc.) with autocompletion, go-to-definition, type hover, and error highlighting for the Python subset. Makes the developer experience much better.

**Implementation plan**: Build on the existing frontend (parser + type system). The LSP server would:
- Parse on every keystroke (incremental parsing)
- Report type errors as diagnostics
- Provide hover info (types of variables/expressions)
- Go-to-definition for functions, classes, imports

**Complexity**: Medium-high. A separate binary/crate. Can use the `tower-lsp` crate.

---

### 🟢 Benchmark Suite Improvements

**Why**: Current benchmarks have ~0.25s startup overhead that dominates small workloads, making results misleading (e.g., Primes shows 11x slower but the actual compute might be similar).

**Targets**:
- Increase workload sizes so compute dominates startup
- Add warm-up iterations
- Measure compilation time separately
- Add benchmarks for specific optimizations (escape analysis, constant folding)
- Memory usage benchmarks (peak RSS, allocation count)

---

## 7. Architecture Improvements

### 🟡 Thread Safety Preparation

**Why**: The current GC uses a single global mutex and single shadow stack (see INSIGHTS.md "GC Global State Limitations"). Any future work on parallelism requires addressing this first.

**Implementation plan** (no need to implement threading yet, but prepare the architecture):
1. **Thread-local shadow stacks**: Replace global `stack_top` with `thread_local!` storage.
2. **Per-thread nursery**: Each thread gets its own nursery for allocation without locking.
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

| Location | Issue | Priority |
|----------|-------|----------|
| `lowering/expressions/operators.rs:731` | List ordering comparisons (`<`, `<=`, `>`, `>=`) not implemented — need lexicographic runtime function | 🔴 |
| `lowering/generators/utils.rs:159` | Truthiness check for heap types in generators uses raw pointer comparison instead of proper truthiness | 🟡 |
| `frontend-python/ast_to_hir/statements/context_managers.rs:292` | `__exit__` receives `(0,0,0)` / `(1,0,0)` instead of actual `(exc_type, exc_val, exc_tb)` | 🟡 |
| `lowering/type_planning/pre_scan.rs:433` | Decorated function ID found but unused — should link decorated function to its wrapper for better type inference | 🟢 |
| `lowering/expressions/calls.rs:101` | Full list unpacking for all call paths not yet complete | 🟢 |

---

## Summary: Suggested Priority Order

For maximum impact with reasonable effort:

1. **Constant folding + DCE** — low effort, immediate wins, enables other optimizations
2. ~~**Stack traces**~~ — ✅ done (Python-style tracebacks with exception chaining)
3. **List ordering comparisons** — low effort, closes a common feature gap
4. **Escape analysis** — high effort, but the single biggest performance unlock
5. ~~**DWARF debug info**~~ — ✅ done (MVP: line tables + function entries)
6. ~~**Full exception objects**~~ — ✅ done (heap instances, `.args`, custom fields, `str(e)`)
7. **Incremental compilation** — medium effort, quality-of-life for larger projects
8. **Collection optimizations (SSO, allocator)** — medium effort, closes the CPython gap
9. **Generational GC** — high effort, long-term performance foundation
10. **Remaining dunders** (`__radd__`, `__int__`, `__pos__`, etc.) — low effort each, incremental value
