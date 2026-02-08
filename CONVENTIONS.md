# CONVENTIONS.md

Coding conventions, patterns, and standards for the Python AOT Compiler project.

## Table of Contents

- [CPython Compatibility](#cpython-compatibility)
- [Safety Policy](#safety-policy)
- [Code Reuse & DRY Principles](#code-reuse--dry-principles)
- [Module Organization](#module-organization)
- [Naming Conventions](#naming-conventions)
- [Type Definitions](#type-definitions)
- [Error Handling](#error-handling)
- [Documentation](#documentation)
- [Function Signatures](#function-signatures)
- [Data Structures](#data-structures)
- [Control Flow Patterns](#control-flow-patterns)
- [Memory & GC](#memory--gc)
- [Testing](#testing)
- [Code Organization Principles](#code-organization-principles)

---

## CPython Compatibility

### Core Principle

**All compiled code must also run correctly in CPython.** This is a fundamental requirement — the compiler implements a subset of Python, and any code that compiles successfully must produce identical behavior when executed with the standard CPython interpreter.

### Guidelines

1. **Semantic equivalence** — Compiled code behavior must match CPython exactly for the supported subset
2. **No compiler-specific extensions** — Do not add syntax or features that CPython doesn't understand
3. **Standard library compatibility** — Use only standard Python library semantics
4. **Type annotations are runtime-compatible** — All type hints must be valid Python 3.8+ syntax

### What This Means in Practice

```python
# Good: Valid Python that works in both CPython and compiler
def factorial(n: int) -> int:
    if n <= 1:
        return 1
    return n * factorial(n - 1)

# Good: Standard collection operations
items: list[int] = [1, 2, 3]
doubled: list[int] = [x * 2 for x in items]

# Avoid: Relying on implementation-specific behavior
# (e.g., dict ordering before Python 3.7, specific error messages)
```

### Verification

Before implementing a new feature:
1. Write the Python code
2. Verify it runs correctly in CPython
3. Implement compiler support
4. Ensure compiled output matches CPython behavior

### AOT-Specific Considerations

While maintaining CPython compatibility, some behaviors differ due to AOT compilation:

| Aspect | CPython | This Compiler |
|--------|---------|---------------|
| Integer size | Arbitrary precision | 64-bit with wrapping |
| Dynamic features | `eval`, `exec`, `getattr` | Not supported |
| Runtime reflection | Full introspection | Limited |
| Type checking | Runtime only | Compile-time + runtime |

These limitations are documented and any code using unsupported features will fail at compile time, not silently produce different behavior.

---

## Safety Policy

### Compiler Crates (No Unsafe)

All compiler crates must forbid unsafe code at the module level:

```rust
#![forbid(unsafe_code)]
```

This applies to: `cli`, `frontend-python`, `hir`, `mir`, `lowering`, `codegen-cranelift`, `linker`, `types`, `utils`, `semantics`, `typecheck`, `diagnostics`.

### Runtime Crate (Controlled Unsafe)

Only `pyaot-runtime` may use unsafe code for FFI and memory management:

```rust
#![allow(unsafe_code)]
```

Unsafe blocks require:
- Clear justification in comments
- Minimal scope (as few lines as possible)
- Safety invariants documented

### Debug Type Assertions

FFI functions that accept `*mut Obj` pointers should validate the type tag in debug builds:

```rust
use crate::debug_assert_type_tag;

#[no_mangle]
pub extern "C" fn rt_list_get(list: *mut Obj, index: i64) -> *mut Obj {
    if list.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        debug_assert_type_tag!(list, TypeTagKind::List, "rt_list_get");
        let list_obj = list as *mut ListObj;
        // ...
    }
}
```

The `debug_assert_type_tag!` macro (defined in `lib.rs`) compiles to nothing in release builds.
To test with assertions enabled, use debug builds: `cargo build --workspace`

---

## Code Reuse & DRY Principles

### Don't Repeat Yourself

Avoid code duplication. Before implementing new functionality:

1. **Search for existing implementations** — Check if similar logic already exists
2. **Extend existing functions** — Modify existing code rather than duplicating
3. **Extract common patterns** — If you see repetition, refactor into shared helpers

### Unified Generic Functions

Prefer generic functions that handle multiple types over type-specific implementations:

```rust
// Good: Single generic function for multiple types
fn lower_collection_len(&mut self, collection: Operand, ty: &Type) -> Result<Operand> {
    let runtime_fn = match ty {
        Type::List(_) => "rt_list_len",
        Type::Dict(_, _) => "rt_dict_len",
        Type::Tuple(_) => "rt_tuple_len",
        Type::Str => "rt_str_len",
        Type::Set(_) => "rt_set_len",
        Type::Bytes => "rt_bytes_len",
        _ => return Err(...),
    };
    self.emit_runtime_call(runtime_fn, &[collection])
}

// Avoid: Separate functions for each type
fn lower_list_len(...) -> Result<Operand> { ... }
fn lower_dict_len(...) -> Result<Operand> { ... }
fn lower_tuple_len(...) -> Result<Operand> { ... }
// ... duplicated logic
```

### Leverage Existing Capabilities

Before writing new code:

1. **Check runtime functions** — `crates/runtime/src/` has many reusable operations
2. **Check lowering helpers** — `crates/lowering/src/utils.rs` and context methods
3. **Check codegen utilities** — `crates/codegen-cranelift/src/utils.rs`
4. **Check type utilities** — `crates/types/src/lib.rs` has type predicates and helpers

### Common Patterns to Reuse

| Need | Look in | Example |
|------|---------|---------|
| Type checking | `types/src/lib.rs` | `is_subtype()`, `is_numeric()` |
| Heap type detection | `lowering/src/utils.rs` | `is_heap_type()` |
| Runtime calls | `codegen-cranelift/src/runtime_calls/` | Existing call patterns |
| Collection operations | `runtime/src/list/`, `dict.rs`, etc. | Reusable runtime functions |
| String operations | `runtime/src/string/` | String manipulation |

### Refactoring Guidelines

When you notice duplication:

```rust
// Before: Duplicated code in multiple places
// In operators.rs:
let left_val = self.lower_expr(left, ...)?;
let right_val = self.lower_expr(right, ...)?;
let result = self.emit_binop(op, left_val, right_val)?;

// In calls.rs (same pattern):
let left_val = self.lower_expr(left, ...)?;
let right_val = self.lower_expr(right, ...)?;
let result = self.emit_binop(op, left_val, right_val)?;

// After: Extract to helper method
impl Lowering<'_> {
    fn lower_binary_operation(
        &mut self,
        op: BinOp,
        left: ExprId,
        right: ExprId,
        ...
    ) -> Result<Operand> {
        let left_val = self.lower_expr(left, ...)?;
        let right_val = self.lower_expr(right, ...)?;
        self.emit_binop(op, left_val, right_val)
    }
}
```

### Macro Usage for Repetitive Patterns

Use macros for truly repetitive patterns:

```rust
// Define similar runtime functions
macro_rules! declare_runtime_fn {
    ($name:ident, $ret:ty, $($arg:ty),*) => {
        // ... implementation
    };
}

declare_runtime_fn!(rt_list_len, i64, *mut Obj);
declare_runtime_fn!(rt_dict_len, i64, *mut Obj);
declare_runtime_fn!(rt_str_len, i64, *mut Obj);
```

### Unified Enum Pattern (Single Source of Truth)

When the same set of variants appears in multiple enums across crates, use a macro to generate a "kind" enum that other enums reference. This is the pattern used for built-in exceptions.

**Problem:** Adding a new variant requires changes in 7+ files.

**Solution:** Define variants once, reference everywhere.

```rust
// crates/core-defs/src/exceptions.rs — SINGLE SOURCE OF TRUTH
macro_rules! define_exceptions {
    ($($variant:ident = $tag:expr => $name:literal),* $(,)?) => {
        /// Unified enum for all built-in exception kinds
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum BuiltinExceptionKind {
            $($variant = $tag,)*
        }

        impl BuiltinExceptionKind {
            pub const fn tag(self) -> u8 { self as u8 }
            pub const fn name(self) -> &'static str {
                match self { $(Self::$variant => $name,)* }
            }
            pub fn from_name(name: &str) -> Option<Self> {
                match name { $($name => Some(Self::$variant),)* _ => None }
            }
            pub const fn from_tag(tag: u8) -> Option<Self> {
                match tag { $($tag => Some(Self::$variant),)* _ => None }
            }
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }
    };
}

define_exceptions! {
    Exception = 0 => "Exception",
    ValueError = 1 => "ValueError",
    // ... add new exceptions here
}
```

**Usage in other crates:**

```rust
// crates/types/src/lib.rs — Type enum
pub enum Type {
    Int, Float, Bool, Str,
    // Instead of 14 separate exception variants:
    BuiltinException(BuiltinExceptionKind),  // ONE variant wrapping the kind
    // ...
}

// crates/hir/src/lib.rs — Builtin enum
pub enum Builtin {
    Print, Len, Range,
    // Instead of 14 separate exception variants:
    BuiltinException(BuiltinExceptionKind),  // ONE variant wrapping the kind
    // ...
}

// Pattern matching becomes simple:
match builtin {
    Builtin::BuiltinException(kind) => kind.tag(),  // Works for ALL exceptions
    _ => 0,
}
```

**Benefits:**
- Adding new exception: 2 files instead of 7
- No match arm explosion in lowering/codegen
- Compile-time guarantees that all variants are handled
- Self-documenting via `BuiltinExceptionKind::ALL`

---

## Module Organization

### File Structure

```rust
//! Module-level documentation (2-3 line summary)
//!
//! Detailed description of purpose and organization.

#![forbid(unsafe_code)]  // Safety declaration (if applicable)

// Standard library imports
use std::collections::HashMap;

// External crate imports (aliased for clarity)
use pyaot_hir as hir;
use pyaot_mir as mir;

// Internal module imports
use crate::context::Lowering;

// Submodule declarations
mod expressions;
mod statements;

// Re-exports for public API
pub use expressions::lower_expr;
```

### Crate Aliasing

Use short aliases for frequently used crates:

```rust
use pyaot_hir as hir;
use pyaot_mir as mir;
use pyaot_types::Type;
use pyaot_utils::{FuncId, VarId, ClassId};
```

### Visibility

- `pub` — Public API exposed to other crates
- `pub(crate)` — Internal API within the crate
- Private (no modifier) — Module-internal only

---

## Naming Conventions

### Variables

| Pattern | Convention | Example |
|---------|------------|---------|
| Snake case | All variables | `var_types`, `current_block_idx` |
| Plural for collections | Vectors, maps, sets | `globals`, `cell_vars`, `locals` |
| Mapping prefix | Map variables | `var_to_local`, `func_to_closure` |
| Type suffix | Type-specific vars | `expr_type`, `result_local` |

### Functions

| Pattern | Convention | Example |
|---------|------------|---------|
| Action verb prefix | All functions | `lower_expr`, `emit_instruction` |
| Phase prefix | Pipeline stage | `lower_`, `compile_`, `emit_` |
| Getter prefix | Accessors | `get_expr_type`, `get_local` |
| Conditional suffix | Optional actions | `_if_needed`, `_or_default` |

```rust
// Good
fn lower_binary_op(...) -> Result<Operand>
fn emit_instruction(...)
fn get_expr_type(...) -> Type
fn add_block_if_needed(...) -> BlockId

// Avoid
fn binary_op(...)  // Missing action verb
fn do_thing(...)   // Too vague
```

### Types and Structs

| Pattern | Convention | Example |
|---------|------------|---------|
| PascalCase | Types, structs, enums | `FuncDef`, `BlockId`, `TypeTag` |
| Id suffix | Index/handle types | `FuncId`, `VarId`, `LocalId` |
| Context suffix | State containers | `LoweringContext`, `CodegenContext` |

### Macros

```rust
// snake_case with descriptive names
define_id!(FuncId);
declare_function!(rt_print_int);
```

---

## Type Definitions

### ID Types (Macro-Based)

Use the `define_id!` macro for consistent ID types:

```rust
// In crates/utils/src/ids.rs
define_id!(FuncId);
define_id!(VarId);
define_id!(ClassId);
define_id!(LocalId);
define_id!(BlockId);
```

Each ID type automatically gets:
- `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`, `Hash`
- `::new(u32)` constructor
- `.index() -> usize` accessor
- `Display` implementation

### Enums

Always derive common traits and document variants:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TypeTagKind {
    /// 64-bit signed integer
    Int = 0,
    /// 64-bit IEEE 754 float
    Float = 1,
    /// Boolean (0 or 1)
    Bool = 2,
    /// Heap-allocated string
    Str = 3,
    // ... all variants documented (defined in core-defs/src/type_tags.rs)
}
```

### Structs

Document all public fields:

```rust
/// MIR local variable
#[derive(Debug, Clone)]
pub struct Local {
    /// Unique identifier within function
    pub id: LocalId,
    /// Original variable name (if any)
    pub name: Option<InternedString>,
    /// Static type of variable
    pub ty: Type,
    /// True if holds GC-managed pointer
    pub is_gc_root: bool,
}
```

### Arena Pattern

Use arenas for large collections of similar items:

```rust
pub struct Module {
    /// Arena for all expressions
    pub exprs: Arena<Expr>,
    /// Arena for all statements
    pub stmts: Arena<Stmt>,
}

// Access via strongly-typed indices
pub type ExprId = la_arena::Idx<Expr>;
pub type StmtId = la_arena::Idx<Stmt>;
```

### Repr Attributes

```rust
// FFI objects (runtime)
#[repr(C)]
pub struct ObjHeader { ... }

// Discriminant control
#[repr(u8)]
pub enum TypeTagKind { ... }
```

---

## Error Handling

### Result Type

All fallible operations return `Result<T>`:

```rust
use pyaot_diagnostics::Result;

pub fn lower_expr(...) -> Result<mir::Operand> {
    // Use ? for propagation
    let value = self.lower_value(expr)?;
    Ok(value)
}
```

### Error Creation

Use descriptive error messages with context:

```rust
use pyaot_diagnostics::CompilerError;

// Codegen errors
return Err(CompilerError::codegen_error(format!(
    "Unsupported operation: {} on type {}",
    op, ty
)));

// Type errors
return Err(CompilerError::type_error(format!(
    "Expected {}, found {}",
    expected, actual
)));
```

### Error Propagation

Prefer `?` over explicit matching:

```rust
// Good
let result = self.lower_expr(expr)?;

// Avoid
let result = match self.lower_expr(expr) {
    Ok(v) => v,
    Err(e) => return Err(e),
};
```

---

## Documentation

### Module-Level Documentation

```rust
//! Expression lowering from HIR to MIR
//!
//! This module handles lowering of all expression types from HIR to MIR.
//! It is organized into submodules by expression category:
//! - `literals`: Int, Float, Bool, Str, None, Var
//! - `operators`: BinOp, Compare, UnOp, LogicalOp
//! - `calls`: Call, ClassRef (instantiation)
```

### Function Documentation

Document complex functions with purpose and parameters:

```rust
/// Lower a binary operation to MIR instructions.
///
/// Handles arithmetic, comparison, and logical operations on
/// all supported types including int, float, str, and collections.
pub(crate) fn lower_binop(
    &mut self,
    op: hir::BinOp,
    left: hir::ExprId,
    right: hir::ExprId,
    hir_module: &hir::Module,
    mir_func: &mut mir::Function,
) -> Result<mir::Operand> {
```

### Inline Comments

Comments explain "why", not "what":

```rust
// Closures need special handling: prepend captured variables to arguments
// before the actual call since closure captures are passed implicitly
hir::ExprKind::Closure { func, captures } => {
    self.lower_closure(*func, captures, hir_module, mir_func)
}

// Skip bounds check for known-safe slice operations
// (slice bounds already validated by type checker)
let element = unsafe { slice.get_unchecked(idx) };
```

### Comment Style

```rust
// Single-line comments for brief notes

// Multi-line comments use
// multiple single-line prefixes
// (not block comments)

/// Doc comments for public API
/// Can span multiple lines

//! Module-level documentation
//! At the top of files
```

---

## Function Signatures

### Parameter Order

Follow consistent parameter ordering:

```rust
pub(crate) fn lower_assign(
    &mut self,                     // 1. Self (mutable context)
    target: VarId,                 // 2. Primary input (HIR entity)
    value: hir::ExprId,            // 3. Secondary input
    type_hint: Option<Type>,       // 4. Optional parameters
    hir_module: &hir::Module,      // 5. Read-only context
    mir_func: &mut mir::Function,  // 6. Output accumulator
) -> Result<()> {
```

### Method Visibility

```rust
impl Lowering<'_> {
    // Public API for other crates
    pub fn lower_module(...) -> Result<mir::Module>

    // Internal API within crate
    pub(crate) fn lower_expr(...) -> Result<mir::Operand>

    // Private helpers
    fn emit_instruction(...)
}
```

### Generic Constraints

Place constraints on separate lines for readability:

```rust
pub fn compile_function<'a, M>(
    ctx: &mut CodegenContext<'a, M>,
    func: &mir::Function,
) -> Result<()>
where
    M: Module,
{
```

---

## Data Structures

### Collection Choice

| Use Case | Collection | Reason |
|----------|------------|--------|
| Ordered map, deterministic iteration | `IndexMap` | HIR/MIR definitions |
| Ordered set, no duplicates | `IndexSet` | Globals, imports |
| Unordered lookup, performance | `HashMap` | Runtime caches |
| Ordered sequence | `Vec` | Parameters, instructions |
| Small fixed-size | `SmallVec` | Inline storage optimization |

### Map Initialization

```rust
use indexmap::IndexMap;

// Empty map
let mut var_types: IndexMap<VarId, Type> = IndexMap::new();

// With capacity hint
let mut locals: IndexMap<LocalId, Local> = IndexMap::with_capacity(16);
```

### Stack-Based State

```rust
pub struct Lowering<'a> {
    /// Stack of loop contexts: (continue_target, break_target)
    pub(crate) loop_stack: Vec<(BlockId, BlockId)>,

    /// Stack of exception handlers
    pub(crate) exception_stack: Vec<ExceptionHandler>,
}

// Push/pop for scope management
self.loop_stack.push((continue_block, break_block));
// ... loop body ...
self.loop_stack.pop();
```

---

## Control Flow Patterns

### Block Management

```rust
// Create new blocks
let then_block = self.add_block();
let else_block = self.add_block();
let merge_block = self.add_block();

// Lower condition
let cond = self.lower_expr(condition, hir_module, mir_func)?;

// Set terminator for current block
self.set_terminator(mir::Terminator::Branch {
    cond,
    then_block,
    else_block,
});

// Switch to then block and lower
self.current_block_idx = then_block.0 as usize;
self.lower_stmt(then_stmt, hir_module, mir_func)?;
self.set_terminator(mir::Terminator::Jump(merge_block));

// Switch to else block and lower
self.current_block_idx = else_block.0 as usize;
self.lower_stmt(else_stmt, hir_module, mir_func)?;
self.set_terminator(mir::Terminator::Jump(merge_block));

// Continue from merge block
self.current_block_idx = merge_block.0 as usize;
```

### Instruction Emission

```rust
// Emit instruction to current block
self.emit_instruction(mir::Instruction::Assign {
    dest: result_local,
    value: operand,
});

// With helper method
fn emit_instruction(&mut self, instr: mir::Instruction) {
    self.current_blocks[self.current_block_idx]
        .instructions
        .push(instr);
}
```

### Pattern Matching

Prefer exhaustive matching without wildcards:

```rust
// Good: explicit handling of all variants
match expr.kind {
    hir::ExprKind::Int(n) => self.lower_int(n),
    hir::ExprKind::Float(f) => self.lower_float(f),
    hir::ExprKind::Bool(b) => self.lower_bool(b),
    hir::ExprKind::Str(s) => self.lower_str(s),
    hir::ExprKind::None => self.lower_none(),
    // ... all variants listed
}

// Avoid: wildcard hides new variants
match expr.kind {
    hir::ExprKind::Int(n) => ...,
    _ => unimplemented!(),  // Hides missing cases
}
```

---

## Memory & GC

### Heap Type Detection

```rust
/// Returns true if the type requires GC management
pub fn is_heap_type(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Str
            | Type::List(_)
            | Type::Dict(_, _)
            | Type::Tuple(_)
            | Type::Set(_)
            | Type::Bytes
            | Type::Class { .. }
            | Type::Iterator(_)
            | Type::Generator { .. }
    )
}
```

### GC Root Marking

```rust
// When creating locals for heap types
let local = mir::Local {
    id: local_id,
    name: Some(name),
    ty: ty.clone(),
    is_gc_root: is_heap_type(&ty),  // Mark for GC tracking
};
```

### Object Layout (Runtime)

```rust
/// Header for all heap objects
#[repr(C)]
pub struct ObjHeader {
    /// Type discriminator (from core-defs TypeTagKind)
    pub type_tag: TypeTagKind,
    /// GC mark bit
    pub marked: bool,
    /// Size in bytes
    pub size: usize,
}

/// Base object type (all heap objects start with ObjHeader)
#[repr(C)]
pub struct Obj {
    pub header: ObjHeader,
    // Data follows...
}
```

---

## Testing

### Mandatory Testing Requirements

**Every change must be verified by running the full test suite.** This is non-negotiable.

```bash
# After ANY code change, run:
./test_examples.sh        # All Python example tests
cargo test --workspace    # All Rust unit tests
```

### Edge Cases Are Required

When implementing or modifying features, **always add tests for edge cases**:

1. **Boundary conditions** — Empty collections, zero values, maximum values
2. **Type variations** — All applicable types for generic operations
3. **Error conditions** — Invalid inputs, type mismatches
4. **Nested structures** — Lists of lists, dicts in tuples, etc.
5. **Interaction with other features** — Closures + exceptions, generators + loops

```python
# Example: Testing list.pop() edge cases
# ===== SECTION: List Pop Edge Cases =====

# Basic pop
basic_list: list[int] = [1, 2, 3]
popped: int = basic_list.pop()
assert popped == 3
assert basic_list == [1, 2]

# Pop with index
indexed_list: list[int] = [1, 2, 3, 4]
popped_first: int = indexed_list.pop(0)
assert popped_first == 1
assert indexed_list == [2, 3, 4]

# Pop negative index
neg_list: list[int] = [1, 2, 3]
popped_neg: int = neg_list.pop(-2)
assert popped_neg == 2

# Pop from single-element list
single: list[int] = [42]
only_elem: int = single.pop()
assert only_elem == 42
assert len(single) == 0

# Pop with different types
str_list: list[str] = ["a", "b", "c"]
popped_str: str = str_list.pop()
assert popped_str == "c"
```

### Regression Prevention

1. **Run tests before and after changes** — Compare results
2. **Add regression tests** — When fixing bugs, add tests that would have caught it
3. **Don't remove failing tests** — Fix the code, not the tests
4. **Document test failures** — If a test must be skipped temporarily, add `TODO:` comment

### CPython Verification

For new features, verify behavior matches CPython:

```bash
# 1. Run in CPython first
python3 examples/test_new_feature.py

# 2. Compile and run
./target/release/pyaot examples/test_new_feature.py -o /tmp/test && /tmp/test

# 3. Both should produce identical output/behavior
```

### Test File Organization

Add tests to existing files in `examples/` by category:

| File | Purpose |
|------|---------|
| `test_core_types.py` | Core types and basic operations |
| `test_functions.py` | Function definitions, calls, closures |
| `test_classes.py` | Class definitions, methods, inheritance |
| `test_collections_list_tuple.py` | Lists and tuples |
| `test_collections_dict_set_bytes.py` | Dicts, sets, and bytes |
| `test_control_flow.py` | If, while, for, match |
| `test_exceptions.py` | Exception handling |
| `test_strings.py` | String operations |
| `test_builtins.py` | Built-in functions |
| `test_generators.py` | Generators and yield |
| `test_iteration.py` | Iterators, map, filter, zip |
| `test_types_system.py` | Type system features |
| `test_stdlib_*.py` | Standard library modules |

### Test Structure

```python
# Section header
# ===== SECTION: Feature Name =====

# Test description in comments
x: int = 42
assert x == 42

# Use descriptive variable names
list_with_duplicates: list[int] = [1, 2, 2, 3]
unique_set: set[int] = set(list_with_duplicates)
assert len(unique_set) == 3
```

### Running Tests

```bash
# Run all example tests
./test_examples.sh

# Run specific test file
./target/release/pyaot examples/test_basic.py -o /tmp/test && /tmp/test

# Unit and integration tests
cargo test --workspace
```

### When to Create New Test Files

**Prefer adding tests to existing files.** Only create new files when:
- Feature requires special flags (e.g., `--module-path`)
- Test has side effects affecting other tests
- Feature is fundamentally different from existing categories

This minimizes compilation overhead and keeps related tests together.

---

## Code Organization Principles

### Single Responsibility

Each file focuses on one task:

```
expressions/
├── mod.rs          # Dispatch to submodules
├── literals.rs     # Int, Float, Bool, Str, None
├── operators.rs    # Binary, unary, comparison
├── calls.rs        # Function calls
└── collections.rs  # List, dict, tuple literals
```

### Hierarchical Organization

Clear layering from high-level to low-level:

```
HIR (high-level)
  ↓ lowering
MIR (mid-level CFG)
  ↓ codegen
Cranelift IR
  ↓ compile
Machine Code
```

### No Circular Dependencies

Dependency graph flows in one direction:

```
utils ← types ← hir ← lowering → mir → codegen-cranelift
          ↑                              ↓
     diagnostics                      linker
```

### Import Organization

```rust
// 1. Standard library
use std::collections::HashMap;

// 2. External crates
use cranelift_codegen::ir::InstBuilder;
use indexmap::IndexMap;

// 3. Workspace crates (aliased)
use pyaot_hir as hir;
use pyaot_mir as mir;

// 4. Current crate
use crate::context::Lowering;
use crate::utils::is_heap_type;
```

---

## Summary

Key principles:

1. **CPython compatibility** — All compiled code must run identically in CPython
2. **Safety first** — No unsafe code in compiler crates
3. **DRY (Don't Repeat Yourself)** — Reuse existing code, avoid duplication
4. **Generic over specific** — Prefer unified functions for multiple types
5. **Comprehensive testing** — Test all edge cases, run full suite after changes
6. **Explicit over implicit** — Avoid wildcards, document all variants
7. **Strong typing** — Use ID types, enums, Result for type safety
8. **Clear naming** — Descriptive names that explain purpose
9. **Consistent structure** — Follow established patterns
10. **Documentation** — Comments explain "why", not "what"
11. **Maintainability** — Easy to extend without breaking existing code
