//! Runtime support for Python's format() builtin
//!
//! Implements format specification mini-language:
//! [[fill]align][sign][#][0][width][grouping_option][.precision][type]

use crate::object::{BoolObj, FloatObj, IntObj, Obj, StrObj};
use pyaot_core_defs::{BuiltinExceptionKind, TypeTagKind};

/// Parsed format specification
#[derive(Debug, Clone)]
struct FormatSpec {
    fill: char,
    align: Option<char>, // '<', '>', '^', '='
    sign: Option<char>,  // '+', '-', ' '
    alternate: bool,     // '#'
    zero_pad: bool,      // '0'
    width: Option<usize>,
    grouping: Option<char>, // '_', ','
    precision: Option<usize>,
    type_spec: Option<char>, // 'd', 'b', 'o', 'x', 'X', 'f', 'e', 'g', 's', 'c', '%', 'n'
}

impl Default for FormatSpec {
    fn default() -> Self {
        Self {
            fill: ' ',
            align: None,
            sign: None,
            alternate: false,
            zero_pad: false,
            width: None,
            grouping: None,
            precision: None,
            type_spec: None,
        }
    }
}

/// Parse format specification string
fn parse_format_spec(spec_str: &str) -> Result<FormatSpec, String> {
    let mut spec = FormatSpec::default();
    let chars: Vec<char> = spec_str.chars().collect();
    let mut i = 0;

    if chars.is_empty() {
        return Ok(spec);
    }

    // Check for [fill]align - align is one of <>=^
    // Fill is any character before align
    if chars.len() >= 2 {
        if matches!(chars[1], '<' | '>' | '^' | '=') {
            spec.fill = chars[0];
            spec.align = Some(chars[1]);
            i = 2;
        } else if matches!(chars[0], '<' | '>' | '^' | '=') {
            spec.align = Some(chars[0]);
            i = 1;
        }
    } else if chars.len() == 1 && matches!(chars[0], '<' | '>' | '^' | '=') {
        spec.align = Some(chars[0]);
        i = 1;
    }

    // Sign: '+', '-', ' '
    if i < chars.len() && matches!(chars[i], '+' | '-' | ' ') {
        spec.sign = Some(chars[i]);
        i += 1;
    }

    // Alternate form: '#'
    if i < chars.len() && chars[i] == '#' {
        spec.alternate = true;
        i += 1;
    }

    // Zero padding: '0'
    if i < chars.len() && chars[i] == '0' {
        spec.zero_pad = true;
        i += 1;
    }

    // Width
    let width_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i > width_start {
        let width_str: String = chars[width_start..i].iter().collect();
        spec.width = Some(width_str.parse().map_err(|_| "Invalid width")?);
    }

    // Grouping: '_' or ','
    if i < chars.len() && matches!(chars[i], '_' | ',') {
        spec.grouping = Some(chars[i]);
        i += 1;
    }

    // Precision: '.' followed by digits
    if i < chars.len() && chars[i] == '.' {
        i += 1;
        let prec_start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i > prec_start {
            let prec_str: String = chars[prec_start..i].iter().collect();
            spec.precision = Some(prec_str.parse().map_err(|_| "Invalid precision")?);
        } else {
            return Err("Missing precision after '.'".to_string());
        }
    }

    // Type specifier
    if i < chars.len() {
        let type_char = chars[i];
        if matches!(
            type_char,
            'd' | 'b' | 'o' | 'x' | 'X' | 'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 's' | 'c' | '%' | 'n'
        ) {
            spec.type_spec = Some(type_char);
            i += 1;
        } else {
            return Err(format!("Invalid format specifier '{}'", type_char));
        }
    }

    // Check for trailing characters
    if i < chars.len() {
        return Err("Invalid format specification".to_string());
    }

    // Apply zero_pad logic: if zero_pad is set and no align, set align to '=' and fill to '0'
    if spec.zero_pad && spec.align.is_none() {
        spec.align = Some('=');
        spec.fill = '0';
    }

    Ok(spec)
}

/// Apply padding to a string
fn apply_padding(s: &str, spec: &FormatSpec) -> String {
    let width = match spec.width {
        Some(w) => w,
        None => return s.to_string(),
    };

    let len = s.chars().count();
    if len >= width {
        return s.to_string();
    }

    let pad = width - len;
    let fill = spec.fill;
    let align = spec.align.unwrap_or('>'); // Default right-align

    match align {
        '<' => {
            // Left align
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", s, fill_str)
        }
        '>' => {
            // Right align
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", fill_str, s)
        }
        '^' => {
            // Center
            let left = pad / 2;
            let right = pad - left;
            let l: String = std::iter::repeat_n(fill, left).collect();
            let r: String = std::iter::repeat_n(fill, right).collect();
            format!("{}{}{}", l, s, r)
        }
        '=' => {
            // Pad after sign
            if let Some(first_char) = s.chars().next() {
                if matches!(first_char, '+' | '-' | ' ') {
                    let fill_str: String = std::iter::repeat_n(fill, pad).collect();
                    let rest: String = s.chars().skip(1).collect();
                    format!("{}{}{}", first_char, fill_str, rest)
                } else {
                    // No sign, treat as right-align
                    let fill_str: String = std::iter::repeat_n(fill, pad).collect();
                    format!("{}{}", fill_str, s)
                }
            } else {
                s.to_string()
            }
        }
        _ => {
            // Default to right-align
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", fill_str, s)
        }
    }
}

/// Format an integer value
fn format_int(value: i64, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('d');

    // Determine the base representation
    let mut result = match type_spec {
        'd' | 'n' => {
            // Decimal
            value.to_string()
        }
        'b' => {
            // Binary
            let binary = format!("{:b}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", binary)
            } else {
                binary
            }
        }
        'o' => {
            // Octal
            let octal = format!("{:o}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", octal)
            } else {
                octal
            }
        }
        'x' => {
            // Lowercase hex
            let hex = format!("{:x}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", hex)
            } else {
                hex
            }
        }
        'X' => {
            // Uppercase hex
            let hex = format!("{:X}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", hex)
            } else {
                hex
            }
        }
        'c' => {
            // Character
            if !(0..=0x10FFFF).contains(&value) {
                return Err("%c requires int in range(0x110000)".to_string());
            }
            if let Some(ch) = char::from_u32(value as u32) {
                return Ok(ch.to_string()); // No padding for character
            } else {
                return Err(format!("Invalid character code: {}", value));
            }
        }
        _ => {
            return Err(format!(
                "Unknown format code '{}' for object of type 'int'",
                type_spec
            ));
        }
    };

    // Add alternate form prefix
    if spec.alternate && value != 0 {
        match type_spec {
            'b' => {
                if value < 0 {
                    result = format!("-0b{}", &result[1..]);
                } else {
                    result = format!("0b{}", result);
                }
            }
            'o' => {
                if value < 0 {
                    result = format!("-0o{}", &result[1..]);
                } else {
                    result = format!("0o{}", result);
                }
            }
            'x' => {
                if value < 0 {
                    result = format!("-0x{}", &result[1..]);
                } else {
                    result = format!("0x{}", result);
                }
            }
            'X' => {
                if value < 0 {
                    result = format!("-0X{}", &result[1..]);
                } else {
                    result = format!("0X{}", result);
                }
            }
            _ => {}
        }
    }

    // Apply sign
    if value >= 0 {
        if let Some(sign) = spec.sign {
            match sign {
                '+' => result = format!("+{}", result),
                ' ' => result = format!(" {}", result),
                _ => {}
            }
        }
    }

    // Apply padding
    Ok(apply_padding(&result, spec))
}

/// Format a float value
fn format_float(value: f64, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('f');
    let precision = spec.precision.unwrap_or(6);

    let mut result = match type_spec {
        'f' | 'F' => {
            // Fixed-point
            format!("{:.prec$}", value, prec = precision)
        }
        'e' => {
            // Exponential lowercase
            format!("{:.prec$e}", value, prec = precision)
        }
        'E' => {
            // Exponential uppercase
            format!("{:.prec$E}", value, prec = precision)
        }
        'g' => {
            // General format lowercase
            if precision == 0 {
                format!("{:.1}", value)
            } else {
                format!("{:.prec$}", value, prec = precision)
            }
        }
        'G' => {
            // General format uppercase
            let s = if precision == 0 {
                format!("{:.1}", value)
            } else {
                format!("{:.prec$}", value, prec = precision)
            };
            s.replace('e', "E")
        }
        '%' => {
            // Percentage
            let percent_value = value * 100.0;
            format!("{:.prec$}%", percent_value, prec = precision)
        }
        'n' => {
            // Number (same as 'g' for now)
            if precision == 0 {
                format!("{:.1}", value)
            } else {
                format!("{:.prec$}", value, prec = precision)
            }
        }
        _ => {
            return Err(format!(
                "Unknown format code '{}' for object of type 'float'",
                type_spec
            ));
        }
    };

    // Apply sign for non-negative values
    if value >= 0.0 && !value.is_nan() {
        if let Some(sign) = spec.sign {
            match sign {
                '+' => result = format!("+{}", result),
                ' ' => result = format!(" {}", result),
                _ => {}
            }
        }
    }

    // Apply padding
    Ok(apply_padding(&result, spec))
}

/// Format a string value
fn format_str(s: &str, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('s');

    if type_spec != 's' {
        return Err(format!(
            "Unknown format code '{}' for object of type 'str'",
            type_spec
        ));
    }

    // Apply precision (truncation)
    let result = if let Some(prec) = spec.precision {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() > prec {
            chars[..prec].iter().collect()
        } else {
            s.to_string()
        }
    } else {
        s.to_string()
    };

    // Apply padding
    Ok(apply_padding(&result, spec))
}

/// Format a boolean value
fn format_bool(value: bool, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('s');

    match type_spec {
        's' => {
            // String representation
            let s = if value { "True" } else { "False" };
            format_str(s, spec)
        }
        'd' | 'b' | 'o' | 'x' | 'X' | 'c' | 'n' => {
            // Numeric representation (True=1, False=0)
            let int_val = if value { 1 } else { 0 };
            format_int(int_val, spec)
        }
        _ => Err(format!(
            "Unknown format code '{}' for object of type 'bool'",
            type_spec
        )),
    }
}

/// Raise a ValueError with the given message
unsafe fn raise_value_error(msg: &str) -> ! {
    crate::exceptions::rt_exc_raise(
        BuiltinExceptionKind::ValueError.tag(),
        msg.as_ptr(),
        msg.len(),
    )
}

/// Format a value according to the format specification
///
/// # Safety
/// - `value` must be a valid object pointer
/// - `spec` must be null or a valid StrObj pointer
#[no_mangle]
pub unsafe extern "C" fn rt_format_value(value: *mut Obj, spec: *mut Obj) -> *mut Obj {
    // Get the format specification string
    let spec_str = if spec.is_null() {
        ""
    } else {
        let spec_obj = &*(spec as *const StrObj);
        if spec_obj.header.type_tag != TypeTagKind::Str {
            raise_value_error("format spec must be a string");
        }
        if spec_obj.len == 0 {
            ""
        } else {
            let bytes = std::slice::from_raw_parts(spec_obj.data.as_ptr(), spec_obj.len);
            std::str::from_utf8(bytes)
                .unwrap_or_else(|_| raise_value_error("Invalid UTF-8 in format spec"))
        }
    };

    // If spec is empty, fall back to str() conversion
    if spec_str.is_empty() {
        return crate::conversions::rt_obj_to_str(value);
    }

    // Parse the format specification
    let format_spec = match parse_format_spec(spec_str) {
        Ok(spec) => spec,
        Err(e) => raise_value_error(&format!("Invalid format specifier: {}", e)),
    };

    // Get the value's type
    let header = &(*value).header;
    let type_tag = header.type_tag;

    // Format based on type
    let formatted = match type_tag {
        TypeTagKind::Int => {
            let int_obj = &*(value as *const IntObj);
            match format_int(int_obj.value, &format_spec) {
                Ok(s) => s,
                Err(e) => raise_value_error(&e),
            }
        }
        TypeTagKind::Float => {
            let float_obj = &*(value as *const FloatObj);
            match format_float(float_obj.value, &format_spec) {
                Ok(s) => s,
                Err(e) => raise_value_error(&e),
            }
        }
        TypeTagKind::Bool => {
            let bool_obj = &*(value as *const BoolObj);
            let bool_val = bool_obj.value;
            match format_bool(bool_val, &format_spec) {
                Ok(s) => s,
                Err(e) => raise_value_error(&e),
            }
        }
        TypeTagKind::Str => {
            let str_obj = &*(value as *const StrObj);
            let bytes = std::slice::from_raw_parts(str_obj.data.as_ptr(), str_obj.len);
            let s = std::str::from_utf8(bytes)
                .unwrap_or_else(|_| raise_value_error("Invalid UTF-8 in string"));
            match format_str(s, &format_spec) {
                Ok(s) => s,
                Err(e) => raise_value_error(&e),
            }
        }
        _ => {
            let type_name = type_tag.type_name();
            raise_value_error(&format!(
                "unsupported format string passed to {}.__format__",
                type_name
            ));
        }
    };

    // Create and return the formatted string
    crate::string::rt_make_str(formatted.as_ptr(), formatted.len())
}
