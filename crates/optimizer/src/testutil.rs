//! Test-only MIR builders, mirroring `mir/src/verify.rs`'s test fixtures.

use pyaot_mir::{verify, LocalDecl, MirBlock, MirFunction, MirInst, MirTerminator, Operand};
use pyaot_types::Repr;
use pyaot_utils::{BlockId, InternedString, LocalId, StringInterner};

pub fn interned(s: &str) -> InternedString {
    StringInterner::new().intern(s)
}

pub fn l(i: u32) -> LocalId {
    LocalId::new(i)
}

pub fn op(i: u32) -> Operand {
    Operand::Local(l(i))
}

/// A zero-param function from `(insts, term)` block descriptions; entry = 0.
pub fn function(locals: Vec<Repr>, blocks: Vec<(Vec<MirInst>, MirTerminator)>) -> MirFunction {
    MirFunction {
        name: interned("test_fn"),
        file: interned_file(),
        params: Vec::new(),
        ret: Repr::Tagged,
        locals: locals.into_iter().map(|repr| LocalDecl { repr }).collect(),
        blocks: blocks
            .into_iter()
            .map(|(insts, term)| MirBlock {
                insts,
                term,
                handler: None,
            })
            .collect(),
        entry: BlockId::new(0),
    }
}

pub fn single_block(locals: Vec<Repr>, insts: Vec<MirInst>, term: MirTerminator) -> MirFunction {
    function(locals, vec![(insts, term)])
}

/// Every pass test ends here: the rewritten function must still verify.
pub fn verify_ok(f: &MirFunction) {
    verify(f, std::slice::from_ref(f)).expect("pass output must verify");
}

pub(crate) fn interned_file() -> pyaot_utils::InternedString {
    pyaot_utils::StringInterner::new().intern("test.py")
}
