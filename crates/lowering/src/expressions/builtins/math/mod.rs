//! Math functions lowering: abs(), pow(), round(), min(), max(), sum(), divmod(), bin(), hex(), oct()
//!
//! This module handles lowering of all math-related built-in function calls.
//! It is organized into submodules by functionality:
//! - `arithmetic`: abs(), pow(), round(), sum(), divmod()
//! - `formatting`: bin(), hex(), oct(), fmt_int(), fmt_int_grouped(), fmt_float_grouped()
//! - `minmax`: min(), max(), min/max on containers and ranges

mod arithmetic;
mod formatting;
mod minmax;
