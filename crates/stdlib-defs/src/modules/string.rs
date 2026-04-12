//! string module definition
//!
//! Provides string constants: ascii_letters, ascii_lowercase, ascii_uppercase,
//! digits, hexdigits, octdigits, punctuation, whitespace, printable.

use crate::types::{ConstValue, StdlibConstDef, StdlibModuleDef, TypeSpec};

/// string module definition
pub static STRING_MODULE: StdlibModuleDef = StdlibModuleDef {
    name: "string",
    functions: &[],
    attrs: &[],
    constants: &[
        StdlibConstDef {
            name: "ascii_letters",
            value: ConstValue::Str(
                "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ",
            ),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "ascii_lowercase",
            value: ConstValue::Str("abcdefghijklmnopqrstuvwxyz"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "ascii_uppercase",
            value: ConstValue::Str("ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "digits",
            value: ConstValue::Str("0123456789"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "hexdigits",
            value: ConstValue::Str("0123456789abcdefABCDEF"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "octdigits",
            value: ConstValue::Str("01234567"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "punctuation",
            value: ConstValue::Str("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "whitespace",
            value: ConstValue::Str(" \t\n\r\x0b\x0c"),
            ty: TypeSpec::Str,
        },
        StdlibConstDef {
            name: "printable",
            value: ConstValue::Str("0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~ \t\n\r\x0b\x0c"),
            ty: TypeSpec::Str,
        },
    ],
    classes: &[],
    exceptions: &[],
    submodules: &[],
};
