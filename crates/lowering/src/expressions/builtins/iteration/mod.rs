//! Iteration functions lowering: iter(), next(), reversed(), sorted(), zip(), chain(), islice()
//!
//! This module handles lowering of all iteration-related built-in and stdlib function calls.
//! It is organized into submodules by functionality:
//! - `core`: lower_iter, parse_range_args, lower_iter_range, lower_next, make_iter_from_expr
//! - `transform`: lower_reversed, lower_reversed_range, lower_sorted, lower_sorted_range
//! - `composite`: lower_zip, lower_map, lower_filter, lower_reduce, lower_captures_to_tuple
//! - `chains`: lower_chain, lower_islice, lower_enumerate

mod chains;
mod composite;
mod core;
mod transform;
