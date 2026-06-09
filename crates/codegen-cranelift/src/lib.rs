//! # codegen-cranelift — typed MIR → native code
//!
//! **Scaffold.** Lowers typed MIR to Cranelift IR and emits an object file.
//!
//! Responsibilities:
//! * map [`pyaot_types::Repr`] to Cranelift register classes via ONE function —
//!   there is no second logical-type mapper and no per-function ABI flags; the
//!   ABI is read straight off the parameter `Repr`s;
//! * the GC shadow-stack prologue/epilogue with the `nroots == 0` leaf
//!   optimization;
//! * exception support — initially setjmp/longjmp (with setjmp called directly
//!   from generated code, see PITFALLS B3), with table-based zero-cost unwinding
//!   as a follow-up so real tracebacks are possible;
//! * DWARF debug info.

#![forbid(unsafe_code)]
