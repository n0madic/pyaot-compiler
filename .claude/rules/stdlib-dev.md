---
paths:
  - "crates/stdlib-defs/**"
  - "crates/runtime/src/**"
---

# Stdlib Development

## Adding Stdlib Module

1. Create `crates/stdlib-defs/src/modules/newmod.rs` with `StdlibModuleDef`
2. Register in `modules/mod.rs`: add to `ALL_MODULES`
3. Implement runtime functions in `crates/runtime/src/newmod.rs`

No changes needed in lowering or codegen — hints handle everything.

```rust
pub struct StdlibFunctionDef {
    pub name: &'static str,           // Python name
    pub runtime_name: &'static str,   // Runtime function
    pub params: &'static [ParamDef],
    pub return_type: TypeSpec,
    pub min_args: usize,              // Minimum required arguments
    pub max_args: usize,              // Maximum allowed arguments
    pub hints: LoweringHints,         // variadic_to_list, auto_box
}

// Registry API
get_module("sys") -> Option<&StdlibModuleDef>
get_function("sys", "exit") -> Option<&StdlibFunctionDef>
get_constant("math", "pi") -> Option<&StdlibConstDef>
```

## Adding Object Methods

1. Define `StdlibMethodDef` in the module (e.g., `re.rs`)
2. Add to `ObjectTypeDef.methods` array in `object_types.rs`
3. Implement runtime function in `crates/runtime/src/*.rs`

No lowering or codegen changes needed — uses generic `ObjectMethodCall` variant.

```rust
pub struct ObjectTypeDef {
    pub type_tag: TypeTagKind,
    pub name: &'static str,
    pub fields: &'static [ObjectFieldDef],
    pub methods: &'static [&'static StdlibMethodDef],
    pub display_format: DisplayFormat,
}

// Lookup API
lookup_object_type(TypeTagKind::Match) -> Option<&ObjectTypeDef>
lookup_object_field(TypeTagKind::Match, "start") -> Option<&ObjectFieldDef>
lookup_object_method(TypeTagKind::Match, "group") -> Option<&StdlibMethodDef>
```

**Note:** File methods use separate dispatch due to I/O complexity and state management.

## Implementation Guidelines

- Prefer Rust's standard library (`std::*`) over custom implementations
- Use well-established, lightweight crates (e.g., `regex`, `serde_json`) when needed
- Avoid reinventing functionality that already exists in the Rust ecosystem
- Keep dependencies minimal and only add crates with active maintenance
