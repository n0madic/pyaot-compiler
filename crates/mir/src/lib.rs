//! # MIR — Mid-level IR (CFG), representation-typed
//!
//! Every MIR value carries a [`pyaot_types::Repr`] **by value, not `Option`**:
//! there is exactly one representation field and it is total. (A dual
//! logical/physical type field with an optional, dual-meaning sentinel is the
//! anti-pattern this design exists to prevent — see PITFALLS A1.)
//!
//! The model is **locals-with-a-Repr-table, not SSA**: the runtime's GC roots
//! are frame slots, so a locals model maps 1:1 to rootable slots. (SSA would
//! need a separate spill pass — exactly the side-table the invariants forbid.)
//!
//! ## What lives here
//!
//! * the IR shapes ([`MirProgram`] / [`MirFunction`] / [`MirInst`] / …);
//! * the **coercion legality table** ([`classify_coercion`] / [`Coercion`]).
//!   It lives here, not in `lowering`, because the verifier must enforce it and
//!   `mir` cannot depend on `lowering`. `lowering::legalize` is still the *only*
//!   place that *emits* a [`MirInst::Coerce`]; this is merely the shared
//!   predicate that makes "coercions only via legalize" structurally checkable.
//! * [`verify`] — mandatory from commit #1, run in debug at *every* pass
//!   boundary; rejects any instruction whose operand/result `Repr`s violate its
//!   typed signature.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pyaot_types::{HeapShape, RawKind, Repr, SigRepr};
use pyaot_utils::{BlockId, ClassId, FuncId, InternedString, LocalId};

mod coerce;
pub mod cold;
pub mod liveness;
pub mod verify;
pub use coerce::CoerceInst;
pub use cold::cold_blocks;
pub use liveness::roots_needed;
pub use verify::{verify, VerifyError};

// Re-exported so consumers (`lowering`, `codegen`) can name builtin kinds.
pub use pyaot_core_defs::BuiltinFunctionKind;
// Re-exported so consumers can name container ops without a direct `hir` dep.
// `ContainerCmpOp` is the HIR comparison operator carried by `ContainerOp`'s
// ordering variants (aliased to avoid clashing with this crate's own `CmpOp`).
pub use pyaot_hir::{CmpOp as ContainerCmpOp, ContainerArg, ContainerOp, ContainerResult};
// Generator op surface (Phase 6E), shared so lowering/codegen name it.
pub use pyaot_hir::{GenOp, GenResult};
// Exception op surface (Phase 7), shared so lowering/codegen name it.
pub use pyaot_hir::{ExcOp, ExcQuery};

// ============================================================================
// Program / function structure
// ============================================================================

/// A whole compiled program: functions (indexed by [`FuncId`]) plus the pool of
/// string-literal bytes the codegen backend materializes into data objects.
#[derive(Debug)]
pub struct MirProgram {
    pub funcs: Vec<MirFunction>,
    /// The synthetic `__main__` function codegen wraps in C `main`.
    pub entry: FuncId,
    pub str_pool: StrPool,
    /// User-defined classes (Phase 5). Codegen emits one `__pyaot_classinit`
    /// from these: `rt_register_class`/`_field_count`/`_qualname` (5A), the static
    /// vtable + `rt_register_method_name` (5B), and `rt_register_dunder_func` (5C).
    pub classes: Vec<MirClass>,
    /// Generator resume functions indexed by dense `gen_id` (Phase 6E). Codegen
    /// emits `__pyaot_generator_resume` dispatching on the generator's stored
    /// `func_id` to the matching resume fn.
    pub generators: Vec<FuncId>,
}

/// A class's codegen-facing registration data. The lowering-resolved subset of
/// [`pyaot_hir::ClassInfo`] that `__pyaot_classinit` needs (no field/method *types*
/// — those were consumed upstream; only identities/slots/FuncIds remain).
#[derive(Debug, Clone)]
pub struct MirClass {
    pub class_id: ClassId,
    /// The bare class name (`Cls`; bytes in `str_pool`) — registered via
    /// `rt_exc_register_class_name` for exception classes (Phase 7C).
    pub name: InternedString,
    /// The `__main__.Cls` qualified-name string (its bytes are in `str_pool`).
    pub qualname: InternedString,
    /// Direct runtime parent (255 = none), for `rt_register_class`.
    pub parent: Option<ClassId>,
    /// The builtin exception tag this class derives from (Phase 7C). With no
    /// user `parent`, `__pyaot_classinit` registers the tag as the runtime
    /// parent (builtin tags ARE runtime class ids), so `rt_exc_isinstance_class`
    /// walks into the pre-seeded builtin hierarchy.
    pub exception_base: Option<u8>,
    pub field_count: usize,
    /// Vtable layout: `slot → FuncId` (Phase 5B). `vtable[slot]` is the function
    /// address codegen materializes into the static vtable data object.
    pub vtable: Vec<FuncId>,
    /// `(method_name_hash, slot)` for `rt_register_method_name` (Phase 5B).
    pub method_names: Vec<(u64, usize)>,
    /// `(field_name_hash, slot)` for `rt_register_field_name` (Phase 8H, D4) —
    /// the by-name attribute path for `Dyn` receivers. Same slot order as the
    /// static `GetField`/`SetField` path.
    pub field_names: Vec<(u64, usize)>,
    /// `(dunder_name_hash, FuncId)` for `rt_register_dunder_func` (Phase 5C).
    pub dunders: Vec<(u64, FuncId)>,
    /// `(attr_idx, const)` class-attribute initializers — codegen materializes each
    /// and stores it via `rt_class_attr_set_ptr` in `__pyaot_classinit` (Phase 5D).
    pub class_attr_inits: Vec<(u32, Const)>,
}

/// A function. `locals` is the Repr table; every [`LocalId`] indexes it.
/// `params.len()` leading locals are the parameters (ABI = f(param Repr)).
#[derive(Debug)]
pub struct MirFunction {
    pub name: InternedString,
    pub params: Vec<Repr>,
    pub ret: Repr,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<MirBlock>,
    pub entry: BlockId,
}

impl MirFunction {
    /// The representation of a local.
    pub fn local_repr(&self, id: LocalId) -> &Repr {
        &self.locals[id.index()].repr
    }

    /// The representation an operand evaluates to.
    pub fn operand_repr(&self, op: &Operand) -> &Repr {
        match op {
            Operand::Local(id) => self.local_repr(*id),
        }
    }
}

/// A local slot's declaration. `Repr` is mandatory and by value (never
/// `Option`); GC-rootness is derived from it via [`Repr::is_gc_root`], never
/// stored here.
#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub repr: Repr,
}

/// A basic block: straight-line instructions ending in exactly one terminator.
#[derive(Debug)]
pub struct MirBlock {
    pub insts: Vec<MirInst>,
    pub term: MirTerminator,
    /// The exception-handler block protecting this block (table-based
    /// unwinding). When `Some`, every raising instruction in this block —
    /// a `Raise` or any call that can raise — transfers control to the
    /// handler block; codegen emits such calls as `try_call` with the
    /// handler as the exceptional edge. Handler blocks themselves carry the
    /// *outer* handler (or `None`): a raise inside an `except` body
    /// propagates outward.
    pub handler: Option<BlockId>,
}

// ============================================================================
// Instructions / operands / terminators
// ============================================================================

#[derive(Debug, Clone)]
pub enum MirInst {
    /// Materialize a constant into `dst`.
    Const { dst: LocalId, val: Const },
    /// Bridge a value's representation from `from` to `to`. **Only**
    /// `lowering::legalize` emits this, and only when [`classify_coercion`]
    /// accepts `(from, to)` — enforced by the type system: the payload
    /// [`CoerceInst`] is constructible outside `mir` only through its
    /// validating constructors. The verifier re-checks as defense-in-depth.
    ///
    /// `checked: true` (Phase 8H, D3) marks a RUNTIME-validated unbox at a
    /// stdlib raw-ABI boundary: a `Tagged` value whose static type is gradual
    /// (`Dyn`, or int-where-float-expected) unboxes through
    /// `rt_unbox_float`/`rt_unbox_int`, which raise `TypeError` on a
    /// mismatched tag instead of silently reinterpreting bits. Only legal for
    /// `(Tagged, Raw(F64))` and `(Tagged, Raw(I64))`.
    Coerce(CoerceInst),
    /// A binary op. On the tagged baseline ALL ops (arithmetic *and* bitwise /
    /// shift) take and produce `Tagged` and dispatch on the tag in the runtime
    /// (`rt_obj_*`), so they are bignum-safe: an `int` operand may dynamically be
    /// a heap `BigInt`, and unboxing it to a raw `i64` would be a silent
    /// miscompile (Invariant 2). The unboxed fast paths (operands and `dst` share
    /// one `Raw` repr, the verifier enforces it) are: `Raw(F64)` for
    /// `Add`/`Sub`/`Mul`, and `Raw(I64)` for `Add`/`Sub`/`Mul`/`Mod`/`FloorDiv`
    /// (Phase 3c) — each proven by typeck's interval pass (no i64 overflow, and a
    /// statically-positive divisor for `Mod`/`FloorDiv`). A range-proven raw fast
    /// path for bitwise/shift is a future optimization, not the correct default.
    BinOp {
        dst: LocalId,
        op: BinOp,
        l: Operand,
        r: Operand,
    },
    /// Unary `Neg`/`Pos`/`Invert` on the tagged baseline; `Not` is truthiness
    /// negation (tagged operand → `Raw(I8)` result).
    Unary {
        dst: LocalId,
        op: UnaryOp,
        operand: Operand,
    },
    /// A single comparison (tagged operands → `Raw(I8)` result).
    Compare {
        dst: LocalId,
        op: CmpOp,
        l: Operand,
        r: Operand,
    },
    /// Truthiness test (tagged operand → `Raw(I8)` result).
    Truthy { dst: LocalId, operand: Operand },
    /// Call a compiled function. Args coerced to the callee's param `Repr`s.
    Call {
        dst: Option<LocalId>,
        func: FuncId,
        args: Vec<Operand>,
    },
    /// Call a runtime builtin (`abs`/`len`/`int`/`float`/`str`/`bool`/…). The
    /// runtime shims take and return tagged `Value`s.
    CallBuiltin {
        dst: Option<LocalId>,
        kind: BuiltinFunctionKind,
        args: Vec<Operand>,
    },
    /// Call a container / iterator runtime op (Phase 4). Parallels `CallBuiltin`
    /// but with a per-op argument/result representation signature
    /// ([`ContainerOp::arg_kinds`] / [`ContainerOp::result`]) the verifier
    /// enforces. Element/key/value args are `Tagged` (uniform tagged storage,
    /// PITFALLS A5); index/count/size args are `Raw(I64)`; the `dst` repr is the
    /// op's result category. The concrete `rt_*` to call is selected at codegen
    /// from `op` plus the receiver representation, exactly as the verifier sees it.
    CallContainer {
        dst: Option<LocalId>,
        op: ContainerOp,
        args: Vec<Operand>,
    },
    /// Call a stdlib runtime function through its declarative descriptor
    /// (Phase 8B). The single generic seam for the frozen runtime's stdlib
    /// surface: the descriptor carries the symbol, the Cranelift register
    /// classes ([`pyaot_core_defs::runtime_func_def::ParamType`]), and per-slot
    /// [`pyaot_core_defs::runtime_func_def::MirSemantic`] when annotated.
    /// Lowering has already coerced each arg to the repr the descriptor's
    /// `(TypeSpec, ParamType)` pair demands; the verifier re-checks every slot
    /// against the descriptor's register class (and semantic, when annotated).
    /// An opaque side-effecting barrier for the optimizer.
    CallRuntime {
        dst: Option<LocalId>,
        def: &'static pyaot_core_defs::RuntimeFuncDef,
        args: Vec<Operand>,
    },
    /// Allocate a fresh class instance (Phase 5) — `rt_make_instance(class_id,
    /// field_count)`. `dst` is `Heap(Class(class_id))` so the instance is
    /// GC-rooted automatically and accepted as a `GetField`/`SetField`/`Call`-self
    /// operand. Fields are zero-filled; `__init__` (a normal `Call`) runs after.
    MakeInstance {
        dst: LocalId,
        class_id: ClassId,
        field_count: i64,
    },
    /// Read instance field `slot` — `rt_instance_get_field(base, slot)`. `base` is
    /// `Heap(Class(_))`/`Tagged`; the result is the uniform tagged field `Value`
    /// (A5), then legalized to the field's repr by the caller.
    GetField {
        dst: LocalId,
        base: Operand,
        slot: usize,
    },
    /// Write instance field `slot` — `rt_instance_set_field(base, slot, value)`.
    /// `base` is `Heap(Class(_))`/`Tagged`; `value` is coerced to `Tagged` (the A5
    /// uniform-storage seam) before the store. No result.
    SetField {
        base: Operand,
        slot: usize,
        value: Operand,
    },
    /// Read instance field by NAME hash (Phase 8H, D4) —
    /// `rt_getattr_name(base, name_hash)`. Used when the receiver's static
    /// type is `Dyn`: the slot is resolved at runtime through the
    /// FIELD_NAME_REGISTRY (AttributeError on a miss/non-instance). `base`
    /// and `dst` are `Tagged`.
    GetFieldNamed {
        dst: LocalId,
        base: Operand,
        name_hash: u64,
    },
    /// Write instance field by NAME hash (Phase 8H, D4) —
    /// `rt_setattr_name(base, name_hash, value)`. `base` and `value` are
    /// `Tagged`. No result.
    SetFieldNamed {
        base: Operand,
        name_hash: u64,
        value: Operand,
    },
    /// Polymorphic method dispatch (Phase 5B): `rt_vtable_lookup_by_name(recv,
    /// name_hash)` → fn ptr → `call_indirect`. Used when a base-typed receiver may
    /// dispatch to an override (D7). `recv` is the `self` (instance base); `args`
    /// are the remaining params, already coerced to the resolved method's reprs;
    /// `ret` is that method's return repr (the indirect-call signature is built
    /// from the operand reprs + `ret`). A concrete receiver devirtualizes to `Call`.
    CallVirtual {
        dst: Option<LocalId>,
        recv: Operand,
        name_hash: u64,
        args: Vec<Operand>,
        ret: Repr,
    },
    /// `isinstance(value, class_id)` with inheritance (Phase 5B) —
    /// `rt_isinstance_class_inherited`. `value` is `Tagged`; `dst` is `Raw(I8)`.
    IsInstance {
        dst: LocalId,
        value: Operand,
        class_id: ClassId,
    },
    /// Read class-level attribute `attr_idx` of `class_id` (Phase 5D) —
    /// `rt_class_attr_get_ptr`. Uniform tagged storage: `dst` is `Tagged`.
    GetClassAttr {
        dst: LocalId,
        class_id: ClassId,
        attr_idx: u32,
    },
    /// Write class-level attribute `attr_idx` (Phase 5D) — `rt_class_attr_set_ptr`.
    /// `value` is coerced to `Tagged`.
    SetClassAttr {
        class_id: ClassId,
        attr_idx: u32,
        value: Operand,
    },
    /// Raise `AssertionError` (no message in Phase 2). Followed by `Unreachable`.
    AssertFail,
    /// A parameterized print op — one variant covers every print form rather
    /// than one runtime-call variant per symbol.
    Print {
        kind: PrintKind,
        arg: Option<Operand>,
    },

    // ── closures / cells / globals (Phase 6) ──
    /// Build a closure env tuple over `func` (Phase 6A): `rt_make_tuple(1+N)`,
    /// slot 0 = `func`'s code address int-tagged (`(addr << 3) | 1`, so the GC's
    /// `is_ptr` skips it), slots `1..=N` = `captures` (each `Tagged` — a cell
    /// pointer, the P6-2 rule). `dst` is `Closure(s)` where `s` is `func`'s
    /// signature minus its env param 0 (which must itself be `Tagged`).
    MakeClosure {
        dst: LocalId,
        func: FuncId,
        captures: Vec<Operand>,
    },
    /// Indirect call through a closure value (Phase 6A): load slot 0 of
    /// `callee`, untag (`>> 3`), and `call_indirect` with the env tuple itself
    /// as arg 0. `callee` is `Closure(sig)`; `args` match `sig.params`; `dst`
    /// (if present) is `sig.ret`. The Cranelift signature is built from `sig`
    /// alone — one calling convention, no marker bits (PITFALLS A4).
    CallIndirect {
        dst: Option<LocalId>,
        callee: Operand,
        args: Vec<Operand>,
        sig: SigRepr,
    },
    /// Allocate a fresh cell holding `init` — `rt_make_cell_ptr` (P6-2: every
    /// cell is a Ptr-cell of full tagged `Value` bits; the typed int/float/bool
    /// cell variants are never emitted). `init` and `dst` are `Tagged`.
    MakeCell { dst: LocalId, init: Operand },
    /// Read a cell's current value — `rt_cell_get_ptr`. Both `Tagged`.
    CellGet { dst: LocalId, cell: Operand },
    /// Store into a cell — `rt_cell_set_ptr`. Both `Tagged`.
    CellSet { cell: Operand, value: Operand },
    /// Read promoted module-global `var_id` (Phase 6B) — `rt_global_get_ptr`
    /// (GC-rooted, full tagged bits). `dst` is `Tagged`.
    GlobalGet { dst: LocalId, var_id: u32 },
    /// Write promoted module-global `var_id` — `rt_global_set_ptr`. `value` is
    /// `Tagged`.
    GlobalSet { var_id: u32, value: Operand },

    // ── generators (Phase 6E) ──
    /// Build a generator object — `rt_make_generator(gen_id, num_locals)`. `dst`
    /// is `Tagged` (the generator is a heap value flowing through tagged slots /
    /// the iterator protocol).
    MakeGenerator {
        dst: LocalId,
        gen_id: u32,
        num_locals: u32,
    },
    /// A generator state-machine op (Phase 6E). The `gen` operand is `Tagged`;
    /// `value` (for `SetLocal`) is `Tagged` (P6-3: tagged slot storage); `imm`
    /// is the slot index / state number; `dst` (if present) carries the op's
    /// result category. The verifier enforces every repr.
    GenOpInst {
        dst: Option<LocalId>,
        op: GenOp,
        gen: Operand,
        imm: u32,
        value: Option<Operand>,
    },

    // ── exceptions (Phase 7) ──
    /// Exception-frame bookkeeping — `rt_exc_pop_frame` /
    /// `rt_exc_start_handling` / `rt_exc_end_handling`. Opaque side-effecting
    /// barrier for the optimizer.
    ExcOp(ExcOp),
    /// A query against the current exception. `Current` → `dst` is `Tagged`
    /// (B5: rooted); `Matches*` → `dst` is `Raw(I8)`.
    ExcQuery { dst: LocalId, query: ExcQuery },
    /// `rt_exc_instance_str(value)` — `value` is `Tagged`, `dst` is `Heap(Str)`.
    ExcInstanceStr { dst: LocalId, value: Operand },
    /// Raise an exception (never returns). Must be the last instruction of its
    /// block, with terminator [`MirTerminator::Unreachable`] (the AssertFail
    /// shape — both are now verifier-enforced). All operands are `Tagged`;
    /// message operands are `StrObj` values (lowering converts via the builtin
    /// `str`), read out with `rt_str_data`/`rt_str_len` at the call (B2-safe:
    /// the runtime copies the bytes; the StrObj itself is a rooted MIR temp).
    Raise(MirRaise),
}

impl MirInst {
    /// May executing this instruction trigger a GC allocation (and therefore a
    /// collection)? **The single source of truth** for the liveness-based GC
    /// root narrowing ([`liveness::roots_needed`]): a local must hold a root
    /// slot exactly while it is live across (or used by) an instruction this
    /// returns `true` for.
    ///
    /// The match is exhaustive with NO catch-all arm — adding a `MirInst`
    /// variant is a compile error here, never a silently-unrooted local.
    /// `locals` resolves operand representations (the tagged baseline of an
    /// arithmetic op can promote to a heap `BigInt`; the raw fast paths
    /// cannot allocate).
    pub fn may_allocate(&self, locals: &[LocalDecl]) -> bool {
        let repr_of = |op: &Operand| match op {
            Operand::Local(id) => &locals[id.index()].repr,
        };
        // A non-Raw operand routes the op through the runtime's tag dispatch
        // (`rt_obj_*`), which may allocate (bignum promotion, float results).
        let non_raw = |op: &Operand| !matches!(repr_of(op), Repr::Raw(_));
        match self {
            // Str/Bytes/BigInt literals materialize heap objects; scalar
            // constants are immediates.
            MirInst::Const { dst: _, val } => {
                matches!(val, Const::Str(_) | Const::Bytes(_) | Const::BigIntStr(_))
            }
            // `checked` unboxes call `rt_unbox_float/int`, which can RAISE
            // (TypeError allocates the exception); `BoxFloat` allocates a
            // `FloatObj`. Every other bridge is bit ops / a load.
            MirInst::Coerce(c) => {
                c.checked() || classify_coercion(c.from(), c.to()) == Some(Coercion::BoxFloat)
            }
            MirInst::BinOp {
                dst: _,
                op: _,
                l,
                r,
            } => non_raw(l) || non_raw(r),
            MirInst::Compare {
                dst: _,
                op: _,
                l,
                r,
            } => non_raw(l) || non_raw(r),
            MirInst::Unary {
                dst: _,
                op: _,
                operand,
            } => non_raw(operand),
            // Truthiness never allocates (tag test / len check).
            MirInst::Truthy { dst: _, operand: _ } => false,
            // Any call can re-enter user code or the allocating runtime.
            MirInst::Call { .. }
            | MirInst::CallBuiltin { .. }
            | MirInst::CallContainer { .. }
            | MirInst::CallVirtual { .. }
            | MirInst::CallIndirect { .. } => true,
            // `RuntimeFuncDef` carries no allocation flag and the substrate is
            // frozen — conservatively allocating.
            MirInst::CallRuntime { .. } => true,
            MirInst::MakeInstance { .. }
            | MirInst::MakeClosure { .. }
            | MirInst::MakeCell { .. }
            | MirInst::MakeGenerator { .. } => true,
            // Field/cell/global/class-attr traffic is load/store on existing
            // objects (uniform tagged storage, no allocation).
            MirInst::GetField { .. } | MirInst::SetField { .. } => false,
            // The by-name forms can RAISE AttributeError (allocates).
            MirInst::GetFieldNamed { .. } | MirInst::SetFieldNamed { .. } => true,
            MirInst::IsInstance { .. } => false,
            MirInst::GetClassAttr { .. } | MirInst::SetClassAttr { .. } => false,
            // Raises AssertionError through the runtime.
            MirInst::AssertFail => true,
            // `rt_print_obj` of a bignum/container formats through the
            // runtime — conservatively allocating.
            MirInst::Print { .. } => true,
            MirInst::CellGet { .. } | MirInst::CellSet { .. } => false,
            MirInst::GlobalGet { .. } | MirInst::GlobalSet { .. } => false,
            // Pure state reads/writes on the generator object are loads/
            // stores; `Next`/`Send`/`Close` re-enter the generator body
            // (arbitrary user code).
            MirInst::GenOpInst {
                dst: _,
                op,
                gen: _,
                imm: _,
                value: _,
            } => match op {
                GenOp::GetLocal
                | GenOp::SetLocal
                | GenOp::GetState
                | GenOp::SetState
                | GenOp::GetSentValue
                | GenOp::SetExhausted
                | GenOp::IsClosing => false,
                GenOp::Next | GenOp::Send | GenOp::Close => true,
            },
            // Frame bookkeeping — no allocation.
            MirInst::ExcOp(_) => false,
            // `Current` (`rt_exc_get_current`) LAZILY materializes the
            // exception instance (rt_make_instance/str/tuple) on first query;
            // the `Matches*` forms are pure class-hierarchy walks.
            MirInst::ExcQuery { dst: _, query } => match query {
                ExcQuery::Current => true,
                ExcQuery::MatchesBuiltin(_) | ExcQuery::MatchesClass(_) => false,
            },
            // Builds a StrObj.
            MirInst::ExcInstanceStr { .. } => true,
            // Every raise allocates the exception value / traceback entry.
            MirInst::Raise(_) => true,
        }
    }

    /// Does executing this instruction have an effect observable outside its
    /// own `dst`? "Effect" = may raise, performs I/O, or stores to memory
    /// another instruction can read. **The single source of truth** for DCE
    /// (Phase 9): an instruction with no side effects whose `dst` is never
    /// read is removable. Allocation by itself is NOT an effect (an
    /// unobserved fresh object is garbage from birth) — this predicate is
    /// deliberately distinct from [`MirInst::may_allocate`].
    ///
    /// The match is exhaustive with NO catch-all arm — adding a `MirInst`
    /// variant is a compile error here, never a silently-deleted effect.
    /// `locals` resolves operand representations: a non-`Raw` operand routes
    /// an arithmetic/comparison op through the runtime's tag dispatch, which
    /// can raise (TypeError, ZeroDivisionError, …).
    pub fn has_side_effects(&self, locals: &[LocalDecl]) -> bool {
        let repr_of = |op: &Operand| match op {
            Operand::Local(id) => &locals[id.index()].repr,
        };
        let non_raw = |op: &Operand| !matches!(repr_of(op), Repr::Raw(_));
        match self {
            // Materializing a constant (even a heap Str/Bytes/BigInt) only
            // produces a fresh value in `dst`.
            MirInst::Const { .. } => false,
            // A `checked` unbox raises TypeError on a bad tag; every
            // unchecked bridge is bit ops / a box allocation.
            MirInst::Coerce(c) => c.checked(),
            // Raw +,-,* and bitwise and/or/xor cannot raise; raw
            // Div/FloorDiv/Mod (ZeroDivisionError), Pow / shifts (negative
            // operand raises) can. Any non-raw operand tag-dispatches through
            // the runtime, which can raise.
            MirInst::BinOp { dst: _, op, l, r } => {
                if non_raw(l) || non_raw(r) {
                    return true;
                }
                match op {
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::BitAnd
                    | BinOp::BitOr
                    | BinOp::BitXor => false,
                    BinOp::Div
                    | BinOp::FloorDiv
                    | BinOp::Mod
                    | BinOp::Pow
                    | BinOp::Shl
                    | BinOp::Shr => true,
                }
            }
            MirInst::Unary {
                dst: _,
                op: _,
                operand,
            } => non_raw(operand),
            // A raw compare is a machine instruction; tagged comparison can
            // re-enter user `__eq__` through the runtime.
            MirInst::Compare {
                dst: _,
                op: _,
                l,
                r,
            } => non_raw(l) || non_raw(r),
            // Truthiness is a tag test / len check — never raises.
            MirInst::Truthy { .. } => false,
            // Calls run arbitrary code.
            MirInst::Call { .. }
            | MirInst::CallBuiltin { .. }
            | MirInst::CallContainer { .. }
            | MirInst::CallRuntime { .. }
            | MirInst::CallVirtual { .. }
            | MirInst::CallIndirect { .. } => true,
            // Fresh-object construction observable only through `dst`.
            MirInst::MakeInstance { .. }
            | MirInst::MakeClosure { .. }
            | MirInst::MakeCell { .. }
            | MirInst::MakeGenerator { .. } => false,
            // Reads of existing objects.
            MirInst::GetField { .. }
            | MirInst::GetClassAttr { .. }
            | MirInst::IsInstance { .. }
            | MirInst::CellGet { .. }
            | MirInst::GlobalGet { .. } => false,
            // Stores observable elsewhere.
            MirInst::SetField { .. }
            | MirInst::SetClassAttr { .. }
            | MirInst::CellSet { .. }
            | MirInst::GlobalSet { .. } => true,
            // The by-name forms raise AttributeError on a miss (and the set
            // form is a store besides).
            MirInst::GetFieldNamed { .. } | MirInst::SetFieldNamed { .. } => true,
            MirInst::AssertFail => true,
            MirInst::Print { .. } => true,
            // Generator state reads are loads off the generator object; the
            // writes mutate it; Next/Send/Close re-enter the body.
            MirInst::GenOpInst {
                dst: _,
                op,
                gen: _,
                imm: _,
                value: _,
            } => match op {
                GenOp::GetLocal | GenOp::GetState | GenOp::GetSentValue | GenOp::IsClosing => false,
                GenOp::SetLocal
                | GenOp::SetState
                | GenOp::SetExhausted
                | GenOp::Next
                | GenOp::Send
                | GenOp::Close => true,
            },
            // Exception-frame bookkeeping mutates the runtime frame stack.
            MirInst::ExcOp(_) => true,
            // `Current` lazily materializes + caches the exception instance
            // (a store into runtime state); `Matches*` are pure walks.
            MirInst::ExcQuery { dst: _, query } => match query {
                ExcQuery::Current => true,
                ExcQuery::MatchesBuiltin(_) | ExcQuery::MatchesClass(_) => false,
            },
            // Formats through the runtime (may invoke user __str__ via the
            // registered dunder for custom exception classes).
            MirInst::ExcInstanceStr { .. } => true,
            MirInst::Raise(_) => true,
        }
    }

    /// The destination local this instruction writes, if any. The complement
    /// of [`MirInst::has_side_effects`] for DCE: a side-effect-free
    /// instruction whose `dst` (this) is never read is removable.
    pub fn dst(&self) -> Option<LocalId> {
        match self {
            MirInst::Const { dst, .. }
            | MirInst::BinOp { dst, .. }
            | MirInst::Unary { dst, .. }
            | MirInst::Compare { dst, .. }
            | MirInst::Truthy { dst, .. }
            | MirInst::MakeInstance { dst, .. }
            | MirInst::GetField { dst, .. }
            | MirInst::GetFieldNamed { dst, .. }
            | MirInst::IsInstance { dst, .. }
            | MirInst::GetClassAttr { dst, .. }
            | MirInst::MakeClosure { dst, .. }
            | MirInst::MakeCell { dst, .. }
            | MirInst::CellGet { dst, .. }
            | MirInst::GlobalGet { dst, .. }
            | MirInst::MakeGenerator { dst, .. }
            | MirInst::ExcQuery { dst, .. }
            | MirInst::ExcInstanceStr { dst, .. } => Some(*dst),
            MirInst::Coerce(c) => Some(c.dst()),
            MirInst::Call { dst, .. }
            | MirInst::CallBuiltin { dst, .. }
            | MirInst::CallContainer { dst, .. }
            | MirInst::CallRuntime { dst, .. }
            | MirInst::CallVirtual { dst, .. }
            | MirInst::CallIndirect { dst, .. }
            | MirInst::GenOpInst { dst, .. } => *dst,
            MirInst::SetField { .. }
            | MirInst::SetFieldNamed { .. }
            | MirInst::SetClassAttr { .. }
            | MirInst::CellSet { .. }
            | MirInst::GlobalSet { .. }
            | MirInst::AssertFail
            | MirInst::Print { .. }
            | MirInst::ExcOp(_)
            | MirInst::Raise(_) => None,
        }
    }

    /// Rewrite every `LocalId` this instruction mentions — `dst`s and operand
    /// reads alike — through `f`. The inliner's splice remap (callee locals
    /// shift by the caller's local count). Lives here because
    /// [`CoerceInst`]'s fields are crate-private by design.
    pub fn map_locals(&mut self, mut f: impl FnMut(LocalId) -> LocalId) {
        let map_op = |op: &mut Operand, f: &mut dyn FnMut(LocalId) -> LocalId| {
            let Operand::Local(id) = op;
            *id = f(*id);
        };
        match self {
            MirInst::Const { dst, .. } => *dst = f(*dst),
            MirInst::Coerce(c) => {
                c.dst = f(c.dst);
                map_op(&mut c.src, &mut f);
            }
            MirInst::BinOp { dst, l, r, .. } | MirInst::Compare { dst, l, r, .. } => {
                *dst = f(*dst);
                map_op(l, &mut f);
                map_op(r, &mut f);
            }
            MirInst::Unary { dst, operand, .. } | MirInst::Truthy { dst, operand } => {
                *dst = f(*dst);
                map_op(operand, &mut f);
            }
            MirInst::Call { dst, args, .. }
            | MirInst::CallBuiltin { dst, args, .. }
            | MirInst::CallContainer { dst, args, .. }
            | MirInst::CallRuntime { dst, args, .. } => {
                if let Some(d) = dst {
                    *d = f(*d);
                }
                args.iter_mut().for_each(|a| map_op(a, &mut f));
            }
            MirInst::CallVirtual {
                dst, recv, args, ..
            } => {
                if let Some(d) = dst {
                    *d = f(*d);
                }
                map_op(recv, &mut f);
                args.iter_mut().for_each(|a| map_op(a, &mut f));
            }
            MirInst::CallIndirect {
                dst, callee, args, ..
            } => {
                if let Some(d) = dst {
                    *d = f(*d);
                }
                map_op(callee, &mut f);
                args.iter_mut().for_each(|a| map_op(a, &mut f));
            }
            MirInst::MakeInstance { dst, .. }
            | MirInst::GetClassAttr { dst, .. }
            | MirInst::GlobalGet { dst, .. }
            | MirInst::MakeGenerator { dst, .. } => *dst = f(*dst),
            MirInst::GetField { dst, base, .. } | MirInst::GetFieldNamed { dst, base, .. } => {
                *dst = f(*dst);
                map_op(base, &mut f);
            }
            MirInst::SetField { base, value, .. } | MirInst::SetFieldNamed { base, value, .. } => {
                map_op(base, &mut f);
                map_op(value, &mut f);
            }
            MirInst::IsInstance { dst, value, .. } => {
                *dst = f(*dst);
                map_op(value, &mut f);
            }
            MirInst::SetClassAttr { value, .. } | MirInst::GlobalSet { value, .. } => {
                map_op(value, &mut f);
            }
            MirInst::AssertFail => {}
            MirInst::Print { arg, .. } => {
                if let Some(a) = arg {
                    map_op(a, &mut f);
                }
            }
            MirInst::MakeClosure { dst, captures, .. } => {
                *dst = f(*dst);
                captures.iter_mut().for_each(|c| map_op(c, &mut f));
            }
            MirInst::MakeCell { dst, init } => {
                *dst = f(*dst);
                map_op(init, &mut f);
            }
            MirInst::CellGet { dst, cell } => {
                *dst = f(*dst);
                map_op(cell, &mut f);
            }
            MirInst::CellSet { cell, value } => {
                map_op(cell, &mut f);
                map_op(value, &mut f);
            }
            MirInst::GenOpInst {
                dst, gen, value, ..
            } => {
                if let Some(d) = dst {
                    *d = f(*d);
                }
                map_op(gen, &mut f);
                if let Some(v) = value {
                    map_op(v, &mut f);
                }
            }
            MirInst::ExcOp(_) => {}
            MirInst::ExcQuery { dst, .. } => *dst = f(*dst),
            MirInst::ExcInstanceStr { dst, value } => {
                *dst = f(*dst);
                map_op(value, &mut f);
            }
            MirInst::Raise(raise) => match raise {
                MirRaise::Builtin { msg, .. } | MirRaise::BuiltinFromNone { msg, .. } => {
                    if let Some(m) = msg {
                        map_op(m, &mut f);
                    }
                }
                MirRaise::BuiltinFrom { msg, cause_msg, .. } => {
                    if let Some(m) = msg {
                        map_op(m, &mut f);
                    }
                    if let Some(m) = cause_msg {
                        map_op(m, &mut f);
                    }
                }
                MirRaise::CustomWithInstance { msg, instance, .. } => {
                    if let Some(m) = msg {
                        map_op(m, &mut f);
                    }
                    map_op(instance, &mut f);
                }
                MirRaise::Stdlib { msg, .. } => {
                    if let Some(m) = msg {
                        map_op(m, &mut f);
                    }
                }
                MirRaise::Instance { value } => map_op(value, &mut f),
                MirRaise::Reraise => {}
            },
        }
    }

    /// Visit every operand this instruction reads (NOT its `dst`). The shared
    /// traversal for the optimizer's use-counting and rewriting.
    pub fn for_each_operand(&self, mut f: impl FnMut(&Operand)) {
        match self {
            MirInst::Const { .. } => {}
            MirInst::Coerce(c) => f(c.src()),
            MirInst::BinOp { l, r, .. } | MirInst::Compare { l, r, .. } => {
                f(l);
                f(r);
            }
            MirInst::Unary { operand, .. } | MirInst::Truthy { operand, .. } => f(operand),
            MirInst::Call { args, .. }
            | MirInst::CallBuiltin { args, .. }
            | MirInst::CallContainer { args, .. }
            | MirInst::CallRuntime { args, .. } => args.iter().for_each(f),
            MirInst::CallVirtual { recv, args, .. } => {
                f(recv);
                args.iter().for_each(f);
            }
            MirInst::CallIndirect { callee, args, .. } => {
                f(callee);
                args.iter().for_each(f);
            }
            MirInst::MakeInstance { .. } => {}
            MirInst::GetField { base, .. } | MirInst::GetFieldNamed { base, .. } => f(base),
            MirInst::SetField { base, value, .. } | MirInst::SetFieldNamed { base, value, .. } => {
                f(base);
                f(value);
            }
            MirInst::IsInstance { value, .. }
            | MirInst::SetClassAttr { value, .. }
            | MirInst::ExcInstanceStr { value, .. } => f(value),
            MirInst::GetClassAttr { .. } => {}
            MirInst::AssertFail => {}
            MirInst::Print { arg, .. } => {
                if let Some(a) = arg {
                    f(a);
                }
            }
            MirInst::MakeClosure { captures, .. } => captures.iter().for_each(f),
            MirInst::MakeCell { init, .. } => f(init),
            MirInst::CellGet { cell, .. } => f(cell),
            MirInst::CellSet { cell, value } => {
                f(cell);
                f(value);
            }
            MirInst::GlobalGet { .. } => {}
            MirInst::GlobalSet { value, .. } => f(value),
            MirInst::MakeGenerator { .. } => {}
            MirInst::GenOpInst { gen, value, .. } => {
                f(gen);
                if let Some(v) = value {
                    f(v);
                }
            }
            MirInst::ExcOp(_) => {}
            MirInst::ExcQuery { .. } => {}
            MirInst::Raise(raise) => match raise {
                MirRaise::Builtin { msg, .. } | MirRaise::BuiltinFromNone { msg, .. } => {
                    if let Some(m) = msg {
                        f(m);
                    }
                }
                MirRaise::BuiltinFrom { msg, cause_msg, .. } => {
                    if let Some(m) = msg {
                        f(m);
                    }
                    if let Some(m) = cause_msg {
                        f(m);
                    }
                }
                MirRaise::CustomWithInstance { msg, instance, .. } => {
                    if let Some(m) = msg {
                        f(m);
                    }
                    f(instance);
                }
                MirRaise::Stdlib { msg, .. } => {
                    if let Some(m) = msg {
                        f(m);
                    }
                }
                MirRaise::Instance { value } => f(value),
                MirRaise::Reraise => {}
            },
        }
    }
}

/// The resolved shape of a `raise` (Phase 7). Mirrors the `rt_exc_*` raise
/// entry points one-to-one.
#[derive(Debug, Clone)]
pub enum MirRaise {
    /// `rt_exc_raise(tag, msg, len)`.
    Builtin { tag: u8, msg: Option<Operand> },
    /// `rt_exc_raise_from(tag, msg, len, cause_tag, cause_msg, cause_len)`.
    BuiltinFrom {
        tag: u8,
        msg: Option<Operand>,
        cause_tag: u8,
        cause_msg: Option<Operand>,
    },
    /// `rt_exc_raise_from_none(tag, msg, len)`.
    BuiltinFromNone { tag: u8, msg: Option<Operand> },
    /// `rt_exc_raise_custom_with_instance(class_id, msg, len, instance)` — the
    /// instance was constructed (and `__init__` run) at the raise site.
    CustomWithInstance {
        class_id: ClassId,
        msg: Option<Operand>,
        instance: Operand,
    },
    /// `rt_exc_raise_stdlib(exc_type_tag, class_id, msg, len)` — a stdlib
    /// exception with a builtin parent tag plus its own reserved class id
    /// (Phase 8D).
    Stdlib {
        class_id: u8,
        exc_type_tag: u8,
        msg: Option<Operand>,
    },
    /// `rt_exc_raise_instance(value)` — re-raise a caught instance (`raise e`).
    Instance { value: Operand },
    /// `rt_exc_reraise()` — bare `raise`.
    Reraise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Pos,
    Invert,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtE,
    Gt,
    GtE,
}

/// The flavor of a print operation. Parameterized so the runtime print surface
/// does not explode into per-symbol instruction variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintKind {
    /// `str()`-semantics: print a string object's raw bytes, no quotes.
    StrObj,
    Int,
    Float,
    Bool,
    None_,
    /// Generic tagged-value print (tag-dispatched; bignum-safe for ints).
    Obj,
    /// The default `' '` separator between arguments.
    Sep,
    /// The trailing newline.
    Newline,
}

#[derive(Debug, Clone)]
pub enum Const {
    /// A string literal; the bytes live in [`MirProgram::str_pool`].
    Str(InternedString),
    /// A `bytes` literal `b"…"`; the raw bytes live in [`MirProgram::str_pool`].
    /// Codegen materializes it via `rt_make_bytes` into a `Heap(Bytes)`.
    Bytes(InternedString),
    /// A fixnum integer literal (tagged at codegen).
    Int(i64),
    /// A big integer literal; decimal text lives in [`MirProgram::str_pool`].
    BigIntStr(InternedString),
    Float(f64),
    Bool(bool),
    None,
    /// A null-pointer `Value` (raw bits 0 — the pointer tag with a null
    /// payload). The "absent optional object" sentinel for stdlib runtime
    /// calls (Phase 8B): descriptors whose optional object params carry no
    /// `ConstValue` default receive this, and the runtime checks `is_null()`.
    /// Distinct from [`Const::None`] (`Value::NONE` has a non-zero tag).
    NullPtr,
}

#[derive(Debug, Clone)]
pub enum MirTerminator {
    Return(Option<Operand>),
    Jump(BlockId),
    Branch {
        cond: Operand,
        then: BlockId,
        else_: BlockId,
    },
    Unreachable,
}

#[derive(Debug, Clone)]
pub enum Operand {
    Local(LocalId),
}

// ============================================================================
// String pool
// ============================================================================

/// Maps each interned string (literal bytes or big-int decimal text) to its raw
/// bytes. Lowering fills it; codegen reads it to emit one data object per id.
#[derive(Debug, Default)]
pub struct StrPool {
    bytes: HashMap<InternedString, Vec<u8>>,
}

impl StrPool {
    pub fn new() -> Self {
        Self {
            bytes: HashMap::new(),
        }
    }

    /// Record the bytes of a string literal (idempotent for a given id).
    pub fn insert(&mut self, id: InternedString, bytes: Vec<u8>) {
        self.bytes.entry(id).or_insert(bytes);
    }

    /// The bytes of a previously-recorded literal.
    pub fn bytes(&self, id: InternedString) -> Option<&[u8]> {
        self.bytes.get(&id).map(Vec::as_slice)
    }

    /// Iterate every (id, bytes) pair — codegen declares one data object each.
    pub fn iter(&self) -> impl Iterator<Item = (InternedString, &[u8])> {
        self.bytes.iter().map(|(id, b)| (*id, b.as_slice()))
    }
}

// ============================================================================
// Coercion legality (the shared predicate the verifier enforces)
// ============================================================================

/// The kind of bridging a [`MirInst::Coerce`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coercion {
    /// Zero machine instructions: the bits are already valid at `to`.
    Noop,
    /// A typed heap pointer reinterpreted as the universal tagged value — free
    /// in this runtime's ABI (heap pointers *are* tagged `Value`s).
    HeapToTagged,
    /// The sound reverse: a tagged value re-typed as a heap pointer. Bit-identical
    /// (a heap pointer *is* a tagged value), so it is a zero-instruction Noop in
    /// codegen — unlike `UnboxFloat`/`UntagBool`, it does **not** reinterpret bits
    /// by an assumed primitive type, so a value of the wrong dynamic type is not
    /// immediately mis-read. It is emitted only when `typeck` has typed the slot as
    /// that container/iterator (e.g. a uniform-tagged `rt_*_get` result feeding a
    /// `list[list[int]]` element local, or `iter_next` into a typed loop variable),
    /// so the narrowing is proven sound.
    TaggedToHeap,
    BoxFloat,
    UnboxFloat,
    TagInt,
    UntagInt,
    TagBool,
    UntagBool,
}

/// **The single coercion legality table.** Returns `Some(kind)` if a value at
/// representation `from` may be legally bridged to `to`, else `None`.
pub fn classify_coercion(from: &Repr, to: &Repr) -> Option<Coercion> {
    if from == to {
        return Some(Coercion::Noop);
    }
    match (from, to) {
        // Two heap shapes of the same container *family* (same constructor; same
        // arity for fixed tuples) are physically identical — element/key/value
        // representation is compile-time metadata only, since every slot is stored
        // as a tagged `Value`. So re-typing one as the other (a `list[Never]`
        // comprehension result into an annotated `list[int]`, a `list[int]` into a
        // `list[Dyn]`, …) is a zero-instruction Noop. Different families
        // (`list` → `dict`) stay illegal: that would mis-dispatch the runtime.
        (Repr::Heap(a), Repr::Heap(b)) if same_container_family(a, b) => Some(Coercion::Noop),
        // A typed heap pointer is bit-identical to a tagged `Value` (both ways).
        (Repr::Heap(_), Repr::Tagged) => Some(Coercion::Noop),
        (Repr::Tagged, Repr::Heap(_)) => Some(Coercion::TaggedToHeap),
        // A closure value IS a tagged heap pointer (its env tuple), so it
        // re-types to `Tagged` for free; the reverse is the same guarded
        // reinterpret as `Tagged → Heap` (proven by typeck's Callable typing /
        // the indirect-call boundary check). `Closure(a) → Closure(b)` with
        // `a == b` is the `from == to` Noop fast path above; with `a != b` it
        // stays ILLEGAL — two different signatures never silently bridge
        // (that would forge an indirect-call ABI).
        (Repr::Closure(_), Repr::Tagged) => Some(Coercion::Noop),
        (Repr::Tagged, Repr::Closure(_)) => Some(Coercion::TaggedToHeap),
        (Repr::Raw(RawKind::F64), Repr::Tagged) => Some(Coercion::BoxFloat),
        (Repr::Tagged, Repr::Raw(RawKind::F64)) => Some(Coercion::UnboxFloat),
        (Repr::Raw(RawKind::I8), Repr::Tagged) => Some(Coercion::TagBool),
        (Repr::Tagged, Repr::Raw(RawKind::I8)) => Some(Coercion::UntagBool),
        (Repr::Raw(RawKind::I64), Repr::Tagged) => Some(Coercion::TagInt),
        (Repr::Tagged, Repr::Raw(RawKind::I64)) => Some(Coercion::UntagInt),
        _ => None,
    }
}

/// True iff `to` is the universal heap-string representation.
pub(crate) fn is_heap_str(repr: &Repr) -> bool {
    matches!(repr, Repr::Heap(HeapShape::Str))
}

/// True iff two heap shapes are the same container *family* — the same physical
/// object kind, differing only in compile-time element/key/value metadata (which
/// is irrelevant because every container slot is a tagged `Value`). Fixed tuples
/// must additionally share arity. Non-container heap shapes match only themselves
/// (handled by the `from == to` fast path, so they are not listed here).
fn same_container_family(a: &HeapShape, b: &HeapShape) -> bool {
    use HeapShape::{Dict, Iterator, List, Set, Tuple, TupleVar};
    match (a, b) {
        (List(_), List(_)) => true,
        (Dict(..), Dict(..)) => true,
        (Set(_), Set(_)) => true,
        (TupleVar(_), TupleVar(_)) => true,
        (Tuple(x), Tuple(y)) => x.len() == y.len(),
        // A fixed-arity tuple and a variable-length tuple are the SAME physical
        // runtime object (`TupleObj`); the arity is compile-time metadata only.
        // Needed for `*args` packing (a call-site `Tuple([...])` literal into a
        // `tuple[Dyn, ...]` param) — Phase 6C.
        (Tuple(_), TupleVar(_)) | (TupleVar(_), Tuple(_)) => true,
        (Iterator(_), Iterator(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_coercion_arms_phase6() {
        use pyaot_types::SigRepr;
        let sig = SigRepr {
            params: vec![Repr::Tagged],
            ret: Box::new(Repr::Tagged),
        };
        let closure = Repr::Closure(Box::new(sig.clone()));
        // Closure <-> Tagged: free both ways (the env tuple IS a tagged value).
        assert_eq!(
            classify_coercion(&closure, &Repr::Tagged),
            Some(Coercion::Noop)
        );
        assert_eq!(
            classify_coercion(&Repr::Tagged, &closure),
            Some(Coercion::TaggedToHeap)
        );
        // Same-signature closure -> closure is the identity Noop.
        assert_eq!(classify_coercion(&closure, &closure), Some(Coercion::Noop));
        // A *different* signature is an illegal bridge (would forge an ABI).
        let other = Repr::Closure(Box::new(SigRepr {
            params: vec![Repr::Tagged, Repr::Tagged],
            ret: Box::new(Repr::Tagged),
        }));
        assert_eq!(classify_coercion(&closure, &other), None);
    }

    #[test]
    fn coercion_table_phase2() {
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Tagged),
            Some(Coercion::Noop)
        );
        assert_eq!(
            classify_coercion(&Repr::Heap(HeapShape::Str), &Repr::Tagged),
            Some(Coercion::Noop)
        );
        assert_eq!(
            classify_coercion(&Repr::Raw(RawKind::F64), &Repr::Tagged),
            Some(Coercion::BoxFloat)
        );
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Raw(RawKind::F64)),
            Some(Coercion::UnboxFloat)
        );
        assert_eq!(
            classify_coercion(&Repr::Raw(RawKind::I8), &Repr::Tagged),
            Some(Coercion::TagBool)
        );
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Raw(RawKind::I8)),
            Some(Coercion::UntagBool)
        );
        // Tagged → a typed heap pointer is the sound, bit-identical reverse Noop
        // (emitted only where typeck has proven the slot's container type).
        assert_eq!(
            classify_coercion(&Repr::Tagged, &Repr::Heap(HeapShape::Str)),
            Some(Coercion::TaggedToHeap)
        );
    }
}
