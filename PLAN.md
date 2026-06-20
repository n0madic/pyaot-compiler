# Implementation Plan — broadening the language subset

The compiler reached the original definition of "working": Phases 1–9 + the
post-Phase-9 hardening backlog are complete (the gated `corpus/` + `microgpt.py`
diff clean vs CPython; bignum; table-based zero-cost unwinding + real tracebacks;
raw-int specialization; MRO-aware lattice joins; cached `StrObj.char_len`; the
release-safe pre-codegen MIR verifier; the uniform value-call convention + its
`devirt` recovery pass). That completed phase log lives in git history (commits +
each crate's `lib.rs` doc) and the auto-memory.

**The breadth goal is met: every aspirational feature is lifted onto the
differential gate** (`test_classes`, `test_collections`, `test_generics`,
`test_strings`, `test_file_io`, … — all diff clean vs CPython in debug +
release). The gate is now organized as one consolidated `corpus/test_*.py` per
large feature category: the former per-feature point files (`pN_*.py`) and the
finer-grained `test_*` splits (`test_types_system`, `test_collections_*`,
`test_format_spec`, `test_dead_code_warnings`, …) were folded into their
category file, every check is an `assert` that passes on both pyaot and CPython,
and the allowlist in `crates/cli/tests/differential.rs` records each fold. The
closures (§3, §5, §7, §10–§12, §14) are recorded in git history + auto-memory
and removed from the backlog.

What remains is **deferred precision**, not breadth, and it **blocks no corpus
file** (every item is on the safe Tagged baseline today). None of it widens the
"Out of scope" list. (Two recent closures: the numeric-tower int→float seams —
§8 — coerce every `float` slot, including parameters, globals, and fields,
through the checked `rt_unbox_float` path; and the §9 builtin-method scope-limits
— `replace` `count`, `find`/`index` `start`/`end`, encoding-honoring
`encode`/`decode` with the Unicode error hierarchy, `dict.fromkeys` class form,
in-place `&=`/`-=`/`^=` — are closed with differential parity, corpus p49–p51.
See git history + auto-memory.)

`test_stdlib_urllib.py` is **not** a feature gap — it exercises the live
`urlopen`/`urlretrieve` network paths and runs (self-checking) only under
`PYAOT_NET_TESTS` (the offline `test_stdlib_urllib_core.py` sibling is on the
gate).

---

## Guiding principles

These are *why* features are built the way they are, and they bind every backlog
item below.

1. **Vertical slice before breadth.** Get the thinnest program through the
   *entire* pipeline first, then widen feature-by-feature. Building each layer
   deeply in isolation strands you in inference work before a stable end-to-end
   contract exists.

2. **★ Correctness lives on the Tagged baseline; inference precision is a
   performance lever, not a correctness requirement. ★** The central enabler.
   `repr_of` gives a *safe* representation for every `SemTy`, and `Repr::Tagged`
   is always correct, so a weak `typeck` yields *slower* code, never *wrong* code.
   A working (slow) compiler can ship on minimal inference and improve afterwards
   with no correctness risk. (See PITFALLS A2 for the failure mode this inverts.)

3. **Safe representation is free; only two narrowings carry a proof obligation.**
   `repr_of` already maps `Float→Raw(F64)`, `Bool→Raw(I8)`, `Str→Heap`,
   containers/classes→`Heap`. The *only* representation optimizations needing a
   proof: (a) `int`: `Tagged` (fixnum-or-bignum) → `Raw(I64)` under a
   range/no-overflow proof; (b) narrowing `Dyn`/`Union` → a concrete type.

4. **One coercion pass, one verifier.** All box/unbox/tag/untag/numeric-widen
   coercions are inserted by `lowering::legalize` and nowhere else. The MIR
   verifier runs in debug at *every* pass boundary and rejects representation
   mismatch.

5. **Inference is one algorithm: collect → solve → materialize.** A single
   constraint pass over a lattice with union-find. Never a fixpoint of mutually
   recursive monotone passes (PITFALLS A3).

6. **ABI = f(Repr), deterministic.** No per-function ABI flags, no marker bits,
   no ABI-repair stage. HOF callbacks are uniform `fn(Value)→Value` at baseline;
   unboxed call paths are a proven optimization, not an escape hatch (PITFALLS A4).

7. **Build the front-half fresh from the design; the substrate is a stable
   base.** Front-half crates are implemented from ARCHITECTURE.md + each crate's
   `lib.rs`, using established algorithms (constraint solving, C3 MRO, standard
   optimizer passes) on their own merits. The substrate (`runtime`, `core-defs`,
   …) is a stable dependency, edited deliberately rather than rewritten.

8. **The runtime contract evolves deliberately with the compiler.** Its `Value`
   ABI + `rt_*` signatures are the seam the whole compiler targets, so changes
   are deliberate, never casual. When compiler development requires a runtime
   change — fixing a runtime bug, a new `rt_*`/ABI, a layout extension — make
   it, document it as a contract change, and back it with corpus coverage
   (precedents: bignum, `StrObj.char_len`). What stays forbidden is editing the
   runtime to paper over a front-half bug or to dodge front-half work.

9. **Differential testing is the spine.** The corpus-vs-CPython harness gates
   every feature. A backlog item is not done until its construct diffs clean vs
   CPython on the corpus (lift the aspirational `test_*.py` files onto the gate
   allowlist as their gaps close).

### Anti-patterns to avoid (do not reintroduce)

A fixpoint-of-passes inference • per-function ABI flags / argument-marker bits /
a separate ABI-repair stage • a dual logical/physical type field with an optional,
dual-meaning sentinel • GC per-slot side-table tag masks • a 61-bit tagged int •
any representation-ambiguous "could be raw or pointer" type. Full catalogue with
rationale in **[PITFALLS.md](PITFALLS.md)** — Part A.

### Where the work lands

The remaining work is declarative breadth: additional builtin **functions** and
**methods** on builtin/stdlib types (§9 scope-limits, and the stdlib-breadth layer
below) follow the declarative two-file pattern of Principle 8 (`stdlib-defs`
descriptor + `runtime` `rt_*`), unless they need true HOF/representation handling.
Keep every gradual/raw seam on the checked-coerce path (PITFALLS A2/A3).

### Reference material — use these, don't re-derive

- **Corpus: fully absorbed.** Every `examples/test_*.py` of the previous
  compiler exists in `corpus/`; three were deliberately cleaned of old-compiler
  workarounds (`test_exceptions.py` `exc_type: int` hack, `test_match.py`,
  `test_stdlib_sys.py`) — the `corpus/` versions are authoritative.
- **`crates/types/src/dunders.rs`** — the dunder classification tables
  (`DunderKind`, `dunder_kind`, `canonical_dunder_name`, `reflected_name`). Any
  work touching operators or dunders consumes this table instead of hardcoding
  name lists. The old `polymorphic_other_type` helper was deliberately **not**
  ported (its blind `Self`-injection into the `other` Union was the microgpt
  `loss=NaN` root cause — type `other` in the solver instead).
- **Runtime registries already in the fork.** `vtable.rs` method registry
  (incl. `rt_obj_has_method` for structural `Protocol` isinstance) +
  `ops/dunder_dispatch.rs` (FNV-1a name-hash probes) — reuse these; no new
  registry mechanism.
- **Builtin-signature reference.** The old repo's `crates/stdlib-defs/`
  (`../python-compiler-rust`) is the kwargs/signature catalogue to consult when
  authoring §9 descriptors / the stdlib-breadth layer — read-only reference, not a
  code source.

---

## Remaining backlog

Only the *open* items remain below; everything completed has been removed (it
lives in git history + auto-memory). **None of these block a corpus file** — they
are deferred precision/seam notes, all safe on the Tagged baseline today. Section
numbers are kept stable so the PITFALLS notes still reference them.

### 9. Methods on builtin types — closed (narrower residuals only)
The shipped scope-limits are **closed** with differential parity (corpus
p49–p51): `replace` `count`, `find`/`index`/`rfind`/`rindex` `start`/`end`
(codepoint bounds, raw i64 slots), encoding-honoring `encode`/`decode`
(utf-8/ascii/latin-1 + the `Unicode{,En,De}codeError`/`LookupError` hierarchy),
the `dict.fromkeys(...)` class form, and in-place `&=`/`-=`/`^=` for sets. The
only residuals left (all safe, none blocking): str predicates use Rust
`char::is_*`, so `isdigit` diverges from CPython on obscure Numeric_Type
codepoints (`½`, `Ⅷ`, superscripts) that need Unicode data the std lacks;
`encode`/`decode` ignore the `errors=` argument and recognize only
utf-8/ascii/latin-1 (other codecs raise `LookupError`).

### 1. Calls & arguments — residual
- **Heap-arg seam** (`list`/`str` param of a genuinely-`Dyn` callee) keeps the
  existing `TaggedToHeap` trust — a deferred precision note, not a blocker.

---

## Known traps — read the matching note before starting an item

Each construct below works in the previous compiler; these notes record where it
got it *wrong first*. Only the traps for **open** items remain (the rest moved to
git history with their fixes). No open backlog item currently carries a dedicated
trap note — the §8 numeric-tower trap retired with its fix (int→float at a slot is
a real `legalize` coercion with a bignum arm, never a noop; the contract lives in
`check_reinterpret` / `coerce_value` / `box_float_for_slot` and the corpus probes
`p16`/`p44`).

---

## Cross-cutting

- **Differential harness (the spine):** every corpus file is the spec. A feature
  isn't done until its corpus entry diffs clean vs CPython; close a backlog item
  by lifting the relevant `test_*.py` onto the gate allowlist
  (`crates/cli/tests/differential.rs::PHASE_CORPUS`).
- **Verifier discipline:** the MIR verifier is a debug-build invariant at every
  pass boundary, plus one mandatory release-safe pass at final pre-codegen.
- **Specialization is always optional:** if a representation optimization is
  unsure, it must fall back to `Tagged` / safe `repr_of` — never guess.
- **Before adding a flag/side-table/special-case:** stop and re-read PITFALLS
  Part A — that is the smell. Fix the representation or the constraint instead.
- **Interaction probes:** nearly every late bug in the previous compiler was a
  *pair* of features (kwargs × closure capture, dunder × Union, decorator ×
  varargs), not a single feature. When closing a backlog item, add at least one
  corpus probe that crosses it with an already-green feature.
- **Stdlib breadth is the layer after this backlog.** The runtime fork already
  carries the previous compiler's `rt_*` surface for `json`/`re`/`os`/`time`/
  `random`/`hashlib`/file I/O, so closing it is mostly `stdlib-defs` descriptors
  (Principle 8) — but only with corpus probes added per module. Never declare a
  module supported on the strength of inherited runtime code alone.
