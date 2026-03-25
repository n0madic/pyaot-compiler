# ROADMAP

Development roadmap for the Python AOT Compiler. Items are grouped by area and roughly ordered by impact within each section. No timelines — priorities may shift based on real-world usage.

**Legend**: 🔴 High impact · 🟡 Medium impact · 🟢 Nice to have

---

## 1. Debugging & Diagnostics

### 🔴 DWARF Debug Information

**Why**: Without source-level debugging, users can only debug at the assembly level with `lldb`/`gdb`. This is the single biggest usability gap — you can't set breakpoints on Python lines, inspect variables, or step through code.

**Current state**: The `--debug` flag preserves symbols and disables optimizations, but generates no DWARF sections. Span information (source locations) exists in HIR but is not propagated through MIR to Cranelift.

**Implementation plan**:
1. **Propagate Span through the pipeline**: HIR already has `Span` (byte offset + length). Add `span: Option<Span>` to MIR `Instruction` and `BasicBlock`. During HIR→MIR lowering, copy spans from HIR expressions/statements to corresponding MIR instructions.
2. **Byte offset → line/column mapping**: Build a `LineMap` from the source file (scan for newline positions). Convert byte offsets to `(line, column)` pairs. This should live in the `diagnostics` crate.
3. **Set Cranelift source locations**: Use `FuncBuilder::set_srcloc(SourceLoc)` to attach source locations to Cranelift instructions. Encode line numbers into `SourceLoc` values.
4. **Generate DWARF sections**: Use `gimli::write` (or Cranelift's built-in DWARF support via `cranelift-object`) to emit:
   - `DW_TAG_compile_unit` — source file info
   - `DW_TAG_subprogram` — function entries with line ranges
   - `.debug_line` — line number table mapping code addresses to Python source lines
   - `DW_TAG_variable` — local variable names and locations (stretch goal)
5. **Emit debug sections in object file**: The `cranelift-object` crate supports adding custom sections. Write DWARF sections (`.debug_info`, `.debug_line`, `.debug_abbrev`, `.debug_str`) into the object file before linking.

**Complexity**: High. Touches every stage of the pipeline. Start with line mappings only (no variable info) for an MVP.

**References**: Cranelift has `SourceLoc` on instructions. The `gimli` crate is the standard Rust library for DWARF reading/writing. See also `wasmtime`'s DWARF generation for a real-world example with Cranelift.

---

### 🔴 Stack Traces / Tracebacks

**Why**: When a compiled program crashes or raises an exception, you see only an error message with no indication of where it happened. This makes debugging compiled programs painful.

**Current state**: `longjmp`-based exceptions carry only a message string. No call stack is recorded.

**Implementation plan**:
1. **Runtime call stack tracking**: Maintain a lightweight call stack in the runtime — each function entry pushes `(func_name, source_line)` onto a thread-local stack, each exit pops. Since the project is single-threaded, a global stack suffices.
2. **MIR instrumentation**: Add `StackPush(func_name, line)` / `StackPop` MIR instructions. Emit `StackPush` at function entry (after `gc_push`) and `StackPop` before every return and before `gc_pop`.
3. **Exception formatting**: When an exception is raised, the runtime captures the current call stack snapshot and formats it as a Python-style traceback:
   ```
   Traceback (most recent call last):
     File "main.py", line 15, in main
     File "main.py", line 8, in process
   ValueError: invalid value
   ```
4. **Overhead control**: The push/pop is cheap (pointer bump on a pre-allocated array). For release builds, could be gated behind a `--traceback` flag if overhead is measurable.

**Complexity**: Medium. Mostly runtime work + MIR instrumentation. Benefits from DWARF work (source line numbers).

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

### 🔴 Dead Code Elimination (DCE)

**Why**: After constant folding and inlining, dead basic blocks and unused computations remain. Removing them reduces code size and improves cache behavior.

**Implementation plan**:
1. **Unreachable block elimination**: After constant folding turns conditional branches into unconditional ones, remove unreachable blocks from the CFG.
2. **Dead instruction elimination**: If an instruction's result is never used and has no side effects, remove it. Mark instructions as side-effect-free (arithmetic, comparisons) vs. side-effectful (calls, stores, prints).
3. **Dead variable elimination**: If a local is assigned but never read, remove the assignment (unless it has side effects).

**Complexity**: Low. Standard compiler optimization. Pairs naturally with constant folding.

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

### 🟢 SIMD for Collection Operations

**Why**: Operations like `sum(list)`, `"".join(strings)`, string search, and list comparison can benefit from SIMD (ARM NEON on Apple Silicon, SSE/AVX on x86).

**Implementation**: Use `std::simd` (nightly) or manual intrinsics for hot paths. Start with `sum()` on `list[int]` and string search.

---

## 4. Language Features

### 🔴 Full Exception Objects

**Why**: Currently `except E as e` binds only the message string. Real Python code accesses `e.args`, `e.__class__.__name__`, and custom exception attributes. This limits interoperability with idiomatic Python patterns.

**Current state**: Exception handling uses setjmp/longjmp. Exception data is stored in a global `ExceptionState` with type tag, class ID, message, `__cause__`, `__context__`, and `suppress_context`. But the bound variable `e` receives only the message string.

**Implementation plan**:
1. **Exception as heap object**: Allocate exception as a regular class instance with `ObjHeader`. Store type tag, message, args tuple, and custom attributes.
2. **`as e` binding**: Bind `e` to the exception object pointer, not the message string. Allow attribute access: `e.args`, `e.message`, custom fields.
3. **Custom exception fields**: User-defined exceptions with `__init__` that store attributes should work:
   ```python
   class HttpError(Exception):
       def __init__(self, status: int, msg: str):
           self.status = status
           super().__init__(msg)
   ```
4. **Backward compatibility**: Continue to support `str(e)` for the message string (via `__str__`).

**Complexity**: Medium-high. Requires changes to exception raising, catching, and the longjmp protocol to pass an object pointer.

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
2. **Stack traces** — moderate effort, huge usability improvement
3. **List ordering comparisons** — low effort, closes a common feature gap
4. **Escape analysis** — high effort, but the single biggest performance unlock
5. **DWARF debug info** — high effort, but transforms the debugging experience
6. **Full exception objects** — medium effort, needed for idiomatic Python patterns
7. **Incremental compilation** — medium effort, quality-of-life for larger projects
8. **Collection optimizations (SSO, allocator)** — medium effort, closes the CPython gap
9. **Generational GC** — high effort, long-term performance foundation
10. **Remaining dunders** (`__radd__`, `__int__`, `__pos__`, etc.) — low effort each, incremental value
