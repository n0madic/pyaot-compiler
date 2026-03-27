# Project Structure

| Crate | Purpose | Key Files |
|-------|---------|-----------|
| `cli` | Entry point, orchestrates pipeline | `main.rs` |
| `core-defs` | Shared definitions (leaf crate) | `exceptions.rs`, `type_tags.rs` |
| `stdlib-defs` | Stdlib module definitions | `types.rs`, `registry.rs`, `modules/*.rs` |
| `frontend-python` | Python parsing → HIR | `ast_to_hir/` |
| `hir` | High-level IR | `lib.rs` |
| `types` | Type system | `lib.rs` |
| `lowering` | HIR → MIR transformation | `context/`, `expressions/`, `statements/` |
| `mir` | Mid-level IR (CFG) | `lib.rs` |
| `optimizer` | MIR optimization passes | `devirtualize/`, `flatten_properties/`, `inline/`, `constfold/`, `peephole/`, `dce/` |
| `codegen-cranelift` | Native code generation | `instructions.rs`, `runtime_calls/`, `debug_info.rs` |
| `linker` | Object → Executable | `lib.rs` |
| `runtime` | Runtime library (staticlib) | `gc.rs`, `object.rs`, collections, stdlib |
| `utils` | IDs, string interning, line mapping | `ids.rs`, `interner.rs`, `line_map.rs` |
| `semantics` | Name resolution, control flow | `lib.rs` |
| `lowering` (type_planning) | Bidirectional type inference during HIR→MIR lowering | `type_planning/infer.rs`, `type_planning/check.rs`, `type_planning/pre_scan.rs` |
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
└── hash_table_utils.rs, minmax_utils.rs, slice_utils.rs, utils.rs  # Utilities
```
