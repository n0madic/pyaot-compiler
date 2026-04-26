# Project Structure

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `cli` | Entry point, orchestrates pipeline | `main.rs` |
| `core-defs` | Shared definitions (leaf crate) | `exceptions.rs`, `tag_kinds.rs`, `value.rs` |
| `stdlib-defs` | Stdlib module definitions | `types.rs`, `registry.rs`, `modules/*.rs` |
| `frontend-python` | Python parsing → HIR | `ast_to_hir/` |
| `hir` | High-level IR | `lib.rs` |
| `types` | Type system; `dunders` is the single source of truth for Python dunder method classification (`DunderKind`, `canonical_dunder_name`, `polymorphic_other_type`) | `lib.rs`, `dunders.rs` |
| `lowering` | HIR → MIR transformation | `context/`, `expressions/`, `statements/`, `generators/` |
| `mir` | Mid-level IR (CFG) | `lib.rs` |
| `optimizer` | MIR optimization passes | `pass.rs`, `devirtualize/`, `flatten_properties/`, `inline/`, `constfold/`, `peephole/`, `dce/` |
| `codegen-cranelift` | Native code generation | `instructions.rs`, `runtime_calls/`, `debug_info.rs` |
| `linker` | Object → Executable | `lib.rs` |
| `runtime` | Runtime library (staticlib) | `gc.rs`, `object.rs`, collections, stdlib |
| `utils` | IDs, string interning, line mapping | `ids.rs`, `interner.rs`, `line_map.rs` |
| `semantics` | Name resolution, control flow | `lib.rs` |
| `lowering` (type_planning) | Bidirectional type inference during HIR→MIR lowering | `type_planning/infer.rs`, `type_planning/check.rs`, `type_planning/closure_scan.rs` |
| `diagnostics` | Error reporting | `lib.rs` |

## Runtime Module Structure

```
crates/runtime/src/
├── gc.rs, object.rs, slab.rs, exceptions.rs, vtable.rs    # Core
├── boxing.rs, conversions.rs, hash.rs, instance.rs, math_ops.rs  # Type ops
├── dict.rs, set.rs, bytes.rs, tuple.rs, list/, string/  # Collections
├── iterator/, sorted.rs, generator.rs  # Iteration
├── globals.rs, cell.rs, class_attrs.rs  # Variable storage
├── print.rs, format.rs, file.rs, stringio.rs  # I/O
├── json.rs, os.rs, re.rs, sys.rs, time.rs  # Standard library
├── random.rs, hashlib.rs, subprocess.rs  # Standard library (cont.)
├── urllib_parse.rs, urllib_request.rs, base64_mod.rs  # Standard library (cont.)
├── copy.rs, functools.rs, abc.rs, builtins.rs  # Standard library (cont.)
├── collections.rs, defaultdict.rs, counter.rs, deque.rs  # Collections module
└── hash_table_utils.rs, minmax_utils.rs, slice_utils.rs, utils.rs  # Utilities
```

## HIR Statement Shape — Unified Binding

All Python binding sites (assignment, `for`, `with ... as`, comprehension
`for`-clauses) share a single recursive `BindingTarget`. HIR is now CFG-only:
functions own `blocks`, `entry_block`, and `try_scopes`; structured control
flow lives in `HirTerminator`, not nested statement variants.

```rust
pub enum BindingTarget {
    Var(VarId),
    Attr { obj: ExprId, field: InternedString, span: Span },
    Index { obj: ExprId, index: ExprId, span: Span },
    ClassAttr { class_id: ClassId, attr: InternedString, span: Span },
    Tuple { elts: Vec<BindingTarget>, span: Span },      // ≤1 Starred per level
    Starred { inner: Box<BindingTarget>, span: Span },   // only inside Tuple
}

pub struct Function {
    pub blocks: IndexMap<HirBlockId, HirBlock>,
    pub entry_block: HirBlockId,
    pub try_scopes: Vec<TryScope>,
    // ...
}

pub struct HirBlock {
    pub stmts: Vec<StmtId>,
    pub terminator: HirTerminator,
    // ...
}

pub enum StmtKind {
    Bind { target: BindingTarget, value: ExprId, type_hint: Option<Type> },
    IterSetup { iter: ExprId },
    IterAdvance { iter: ExprId, target: BindingTarget },
    // ... plus Return, Break, Continue, Pass, Assert, IndexDelete, Raise, Expr
}
```

**Entry points:**
- Frontend: `frontend-python/src/ast_to_hir/variables.rs::bind_target(&py::Expr) -> Result<BindingTarget>`.
- CFG construction: `hir::cfg_builder::{CfgBuilder, CfgStmt}` materializes structured frontend control flow into `Function::{blocks, entry_block, try_scopes}`.
- Lowering: `lowering/src/statements/assign/bind.rs::lower_binding_target` handles recursive stores; `lowering/src/statements/iter_protocol.rs` lowers `IterSetup` / `IterAdvance` and routes loop targets through the same binding path.

**Shared walker:** `BindingTarget::for_each_var<F: FnMut(VarId)>` — enumerates every `Var` leaf for CFG walkers, type-planning passes, and generator analysis.

**Bespoke paths (intentionally not using BindingTarget):**
- Walrus `:=` (PEP 572 — restricted to Name) — `expressions/mod.rs`.
- `except ... as NAME` (grammar restricts to Name) — `statements/exceptions.rs`.
- Match patterns (PEP 634 — separate refutable `Pattern` AST) — `statements/match_stmt/` plus `ExprKind::MatchPattern`.

See `INSIGHTS.md` § "Unified Binding Targets" for the design rationale.
