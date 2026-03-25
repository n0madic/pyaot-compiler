---
paths:
  - "examples/**"
  - "tests/**"
---

# Testing Conventions

Tests organized by domain in `examples/` (~1000+ assertions). Add to **existing** files by section.

**Use descriptive variable names** to avoid conflicts (prefix with test context if needed).

Only create a new test file if:
- The feature requires special compilation flags (like `--module-path`)
- The test has side effects that could affect other tests
- The feature is fundamentally different from all existing categories

Use direct file editing instead of cat with heredocs.

**Debug builds** include type tag assertions:
```bash
cargo build --workspace  # Assertions enabled
# Type mismatch → panic: "rt_list_get: expected List, got Dict"
```

**Note:** `--debug` flag is different — it preserves symbols in generated executable for assembly-level debugging.
