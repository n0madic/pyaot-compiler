//! Shared format specification logic — PEP 3101 format spec mini-language.
//!
//! Used by the runtime (`pyaot-runtime`) for runtime dispatch and by the
//! optimizer (`pyaot-optimizer`) for compile-time constant folding.
//!
//! All functions return `Result<String, String>` — callers adapt errors to
//! their environment (runtime raises ValueError; optimizer skips the fold).
//!
//! Format spec grammar: `[[fill]align][sign][#][0][width][grouping][.precision][type]`

/// Parsed format specification.
#[derive(Debug, Clone)]
pub struct FormatSpec {
    pub fill: char,
    pub align: Option<char>, // '<', '>', '^', '='
    pub sign: Option<char>,  // '+', '-', ' '
    pub alternate: bool,     // '#'
    pub zero_pad: bool,      // '0'
    pub width: Option<usize>,
    pub grouping: Option<char>, // '_', ','
    pub precision: Option<usize>,
    pub type_spec: Option<char>, // 'd', 'b', 'o', 'x', 'X', 'f', 'e', 'g', 's', 'c', '%', 'n'
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

/// Parse a format specification string.
pub fn parse_format_spec(spec_str: &str) -> Result<FormatSpec, String> {
    let mut spec = FormatSpec::default();
    let chars: Vec<char> = spec_str.chars().collect();
    let mut i = 0;

    if chars.is_empty() {
        return Ok(spec);
    }

    // Check for [fill]align - align is one of <>=^
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
        spec.width = Some(width_str.parse().map_err(|_| "Invalid width".to_string())?);
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
            spec.precision = Some(
                prec_str
                    .parse()
                    .map_err(|_| "Invalid precision".to_string())?,
            );
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

/// Insert a grouping separator every 3 digits from the right.
/// `digits` must contain only ASCII digit characters (no sign, no prefix).
pub fn insert_grouping(digits: &str, sep: char) -> String {
    let len = digits.len();
    if len <= 3 {
        return digits.to_string();
    }
    let mut result = String::with_capacity(len + len / 3);
    let first_group = len % 3;
    if first_group > 0 {
        result.push_str(&digits[..first_group]);
    }
    let mut i = first_group;
    while i < len {
        if !result.is_empty() {
            result.push(sep);
        }
        result.push_str(&digits[i..i + 3]);
        i += 3;
    }
    result
}

/// Apply grouping separators to a formatted integer or float string.
pub fn apply_grouping_to_number(s: &str, sep: char) -> String {
    let chars: &[u8] = s.as_bytes();
    let mut i = 0;

    let sign_len = if !chars.is_empty() && matches!(chars[0], b'+' | b'-' | b' ') {
        1
    } else {
        0
    };
    i += sign_len;

    let prefix_len = if i + 1 < chars.len()
        && chars[i] == b'0'
        && matches!(chars[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        2
    } else {
        0
    };
    i += prefix_len;

    let digit_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    let digit_end = i;
    let tail = &s[digit_end..];

    let sign = &s[..sign_len];
    let prefix = &s[sign_len..sign_len + prefix_len];
    let digits = &s[digit_start..digit_end];

    let grouped = insert_grouping(digits, sep);
    format!("{}{}{}{}", sign, prefix, grouped, tail)
}

/// Apply padding to a string according to the format spec.
pub fn apply_padding(s: &str, spec: &FormatSpec) -> String {
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
    let align = spec.align.unwrap_or('>');

    match align {
        '<' => {
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", s, fill_str)
        }
        '>' => {
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", fill_str, s)
        }
        '^' => {
            let left = pad / 2;
            let right = pad - left;
            let l: String = std::iter::repeat_n(fill, left).collect();
            let r: String = std::iter::repeat_n(fill, right).collect();
            format!("{}{}{}", l, s, r)
        }
        '=' => {
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            let bytes = s.as_bytes();
            let mut prefix_len = 0;
            if !bytes.is_empty() && matches!(bytes[0], b'+' | b'-' | b' ') {
                prefix_len = 1;
            }
            if bytes.len() >= prefix_len + 2
                && bytes[prefix_len] == b'0'
                && matches!(bytes[prefix_len + 1], b'x' | b'X' | b'b' | b'o')
            {
                prefix_len += 2;
            }
            if prefix_len > 0 {
                let prefix = &s[..prefix_len];
                let rest = &s[prefix_len..];
                format!("{}{}{}", prefix, fill_str, rest)
            } else {
                format!("{}{}", fill_str, s)
            }
        }
        _ => {
            let fill_str: String = std::iter::repeat_n(fill, pad).collect();
            format!("{}{}", fill_str, s)
        }
    }
}

/// Fix Rust's exponent format to match Python's: `e1` → `e+01`, `e-1` → `e-01`.
pub fn fix_exponent_format(s: &str) -> String {
    if let Some(e_pos) = s.find('e') {
        let (mantissa, exp_part) = s.split_at(e_pos);
        let exp_str = &exp_part[1..];
        let (sign, digits) = if let Some(d) = exp_str.strip_prefix('-') {
            ("-", d)
        } else if let Some(d) = exp_str.strip_prefix('+') {
            ("+", d)
        } else {
            ("+", exp_str)
        };
        if digits.len() < 2 {
            format!("{}e{}{:0>2}", mantissa, sign, digits)
        } else {
            format!("{}e{}{}", mantissa, sign, digits)
        }
    } else {
        s.to_string()
    }
}

/// Format an integer value according to a parsed `FormatSpec`.
pub fn format_int(value: i64, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('d');

    let mut result = match type_spec {
        'd' | 'n' => value.to_string(),
        'b' => {
            let binary = format!("{:b}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", binary)
            } else {
                binary
            }
        }
        'o' => {
            let octal = format!("{:o}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", octal)
            } else {
                octal
            }
        }
        'x' => {
            let hex = format!("{:x}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", hex)
            } else {
                hex
            }
        }
        'X' => {
            let hex = format!("{:X}", value.unsigned_abs());
            if value < 0 {
                format!("-{}", hex)
            } else {
                hex
            }
        }
        'c' => {
            if !(0..=0x10FFFF).contains(&value) {
                return Err("%c requires int in range(0x110000)".to_string());
            }
            char::from_u32(value as u32)
                .ok_or_else(|| format!("Invalid character code: {}", value))?
                .to_string()
        }
        _ => {
            return Err(format!(
                "Unknown format code '{}' for object of type 'int'",
                type_spec
            ))
        }
    };

    if spec.alternate {
        match type_spec {
            'b' => {
                result = if value < 0 {
                    format!("-0b{}", &result[1..])
                } else {
                    format!("0b{}", result)
                }
            }
            'o' => {
                result = if value < 0 {
                    format!("-0o{}", &result[1..])
                } else {
                    format!("0o{}", result)
                }
            }
            'x' => {
                result = if value < 0 {
                    format!("-0x{}", &result[1..])
                } else {
                    format!("0x{}", result)
                }
            }
            'X' => {
                result = if value < 0 {
                    format!("-0X{}", &result[1..])
                } else {
                    format!("0X{}", result)
                }
            }
            _ => {}
        }
    }

    if value >= 0 {
        if let Some(sign) = spec.sign {
            match sign {
                '+' => result = format!("+{}", result),
                ' ' => result = format!(" {}", result),
                _ => {}
            }
        }
    }

    if let Some(sep) = spec.grouping {
        if !matches!(type_spec, 'd' | 'n' | 'b' | 'o' | 'x' | 'X') {
            return Err(format!("Cannot specify '{}' with '{}'", sep, type_spec));
        }
        result = apply_grouping_to_number(&result, sep);
    }

    Ok(apply_padding(&result, spec))
}

/// Format a float value according to a parsed `FormatSpec`.
pub fn format_float(value: f64, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('g');
    let precision = spec.precision.unwrap_or(6);

    let mut result = match type_spec {
        'f' | 'F' => format!("{:.prec$}", value, prec = precision),
        'e' => fix_exponent_format(&format!("{:.prec$e}", value, prec = precision)),
        'E' => {
            fix_exponent_format(&format!("{:.prec$e}", value, prec = precision)).replace('e', "E")
        }
        'g' | 'G' => {
            let prec = if precision == 0 { 1 } else { precision };
            let abs_val = value.abs();
            let formatted = if abs_val == 0.0 {
                let mut s = format!("{:.prec$}", value, prec = prec.saturating_sub(1));
                if !spec.alternate && s.contains('.') {
                    s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                }
                s
            } else {
                let exp = abs_val.log10().floor() as i32;
                if exp >= -4 && exp < prec as i32 {
                    let fixed_prec = (prec as i32 - 1 - exp).max(0) as usize;
                    let mut s = format!("{:.prec$}", value, prec = fixed_prec);
                    if !spec.alternate && s.contains('.') {
                        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                    }
                    s
                } else {
                    let exp_prec = prec.saturating_sub(1);
                    let mut s = format!("{:.prec$e}", value, prec = exp_prec);
                    if !spec.alternate && s.contains('.') {
                        if let Some(e_pos) = s.find('e') {
                            let (mantissa, exp_part) = s.split_at(e_pos);
                            let trimmed = mantissa.trim_end_matches('0').trim_end_matches('.');
                            s = format!("{}{}", trimmed, exp_part);
                        }
                    }
                    fix_exponent_format(&s)
                }
            };
            if type_spec == 'G' {
                formatted.replace('e', "E")
            } else {
                formatted
            }
        }
        '%' => format!("{:.prec$}%", value * 100.0, prec = precision),
        'n' => {
            let prec = if precision == 0 { 1 } else { precision };
            let abs_val = value.abs();
            if abs_val == 0.0 {
                let mut s = format!("{:.prec$}", value, prec = prec.saturating_sub(1));
                if s.contains('.') {
                    s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                }
                s
            } else {
                let exp = abs_val.log10().floor() as i32;
                if exp >= -4 && exp < prec as i32 {
                    let fixed_prec = (prec as i32 - 1 - exp).max(0) as usize;
                    let mut s = format!("{:.prec$}", value, prec = fixed_prec);
                    if s.contains('.') {
                        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
                    }
                    s
                } else {
                    fix_exponent_format(&format!(
                        "{:.prec$e}",
                        value,
                        prec = prec.saturating_sub(1)
                    ))
                }
            }
        }
        _ => {
            return Err(format!(
                "Unknown format code '{}' for object of type 'float'",
                type_spec
            ))
        }
    };

    if type_spec == 'F' {
        result = result.replace("inf", "INF").replace("nan", "NAN");
    }

    if value >= 0.0 || value.is_nan() {
        if let Some(sign) = spec.sign {
            match sign {
                '+' => result = format!("+{}", result),
                ' ' => result = format!(" {}", result),
                _ => {}
            }
        }
    }

    if let Some(sep) = spec.grouping {
        if !matches!(type_spec, 'f' | 'F' | 'e' | 'E' | 'g' | 'G' | '%' | 'n') {
            return Err(format!("Cannot specify '{}' with '{}'", sep, type_spec));
        }
        result = apply_grouping_to_number(&result, sep);
    }

    Ok(apply_padding(&result, spec))
}

/// Format a string value according to a parsed `FormatSpec`.
pub fn format_str(s: &str, spec: &FormatSpec) -> Result<String, String> {
    let type_spec = spec.type_spec.unwrap_or('s');
    if type_spec != 's' {
        return Err(format!(
            "Unknown format code '{}' for object of type 'str'",
            type_spec
        ));
    }

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

    // Strings default to left-align in Python
    let mut str_spec = spec.clone();
    if str_spec.align.is_none() {
        str_spec.align = Some('<');
    }
    Ok(apply_padding(&result, &str_spec))
}

/// Format a boolean value according to a parsed `FormatSpec`.
///
/// CPython: `bool` inherits `int.__format__`, so a non-empty spec formats the
/// integer 1/0 — `f"{True:5}"` is `"    1"` (NOT `" True"`). Only an empty spec
/// renders `"True"`/`"False"` (via `str(self)`), and that is handled by the
/// caller before this function is reached.
pub fn format_bool(value: bool, spec: &FormatSpec) -> Result<String, String> {
    // CPython: `bool` inherits `int.__format__`. An EMPTY spec yields
    // "True"/"False" (`str(self)`) — but that case never reaches here, the
    // caller (`rt_format`/constfold) short-circuits an empty spec to `str()`.
    // So any spec seen here is non-empty and formats the integer 1/0
    // (`f"{True:5}"` == "    1", `f"{False:08b}"` == "00000000"). Only an
    // unknown type char (e.g. the string-only `s`) is an error.
    let int_value = if value { 1 } else { 0 };
    match spec.type_spec {
        None | Some('d' | 'b' | 'o' | 'x' | 'X' | 'c' | 'n') => format_int(int_value, spec),
        Some(c) => Err(format!(
            "Unknown format code '{}' for object of type 'bool'",
            c
        )),
    }
}

// --- Convenience entry points for constfold and testing ---

/// Format an integer value with a raw spec string.
pub fn format_int_spec(value: i64, spec: &str) -> Result<String, String> {
    let fs = parse_format_spec(spec)?;
    format_int(value, &fs)
}

/// Format a float value with a raw spec string.
pub fn format_float_spec(value: f64, spec: &str) -> Result<String, String> {
    let fs = parse_format_spec(spec)?;
    format_float(value, &fs)
}

/// Format a string value with a raw spec string.
pub fn format_str_spec(s: &str, spec: &str) -> Result<String, String> {
    let fs = parse_format_spec(spec)?;
    format_str(s, &fs)
}

/// Format a boolean value with a raw spec string.
pub fn format_bool_spec(value: bool, spec: &str) -> Result<String, String> {
    let fs = parse_format_spec(spec)?;
    format_bool(value, &fs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_int_basic() {
        assert_eq!(format_int_spec(42, "d").unwrap(), "42");
        assert_eq!(format_int_spec(42, "5d").unwrap(), "   42");
        assert_eq!(format_int_spec(42, "<5d").unwrap(), "42   ");
        assert_eq!(format_int_spec(42, "^5d").unwrap(), " 42  ");
        assert_eq!(format_int_spec(42, "05d").unwrap(), "00042");
        assert_eq!(format_int_spec(-42, "05d").unwrap(), "-0042");
    }

    #[test]
    fn test_int_bases() {
        assert_eq!(format_int_spec(255, "x").unwrap(), "ff");
        assert_eq!(format_int_spec(255, "X").unwrap(), "FF");
        assert_eq!(format_int_spec(255, "#x").unwrap(), "0xff");
        assert_eq!(format_int_spec(255, "b").unwrap(), "11111111");
        assert_eq!(format_int_spec(255, "#b").unwrap(), "0b11111111");
        assert_eq!(format_int_spec(255, "o").unwrap(), "377");
        assert_eq!(format_int_spec(255, "#o").unwrap(), "0o377");
    }

    #[test]
    fn test_int_sign() {
        assert_eq!(format_int_spec(42, "+d").unwrap(), "+42");
        assert_eq!(format_int_spec(-42, "+d").unwrap(), "-42");
        assert_eq!(format_int_spec(42, " d").unwrap(), " 42");
    }

    #[test]
    fn test_int_grouping() {
        assert_eq!(format_int_spec(1_000_000, ",d").unwrap(), "1,000,000");
        assert_eq!(format_int_spec(1_000_000, "_d").unwrap(), "1_000_000");
    }

    #[test]
    // `3.14159` is a formatter test input, not the math constant pi.
    #[allow(clippy::approx_constant)]
    fn test_float_basic() {
        assert_eq!(format_float_spec(3.14159, ".2f").unwrap(), "3.14");
        assert_eq!(format_float_spec(3.14159, "8.2f").unwrap(), "    3.14");
        assert_eq!(format_float_spec(0.25, ".1%").unwrap(), "25.0%");
        assert_eq!(format_float_spec(0.0001234, ".2e").unwrap(), "1.23e-04");
        assert_eq!(format_float_spec(1234.5, ",.2f").unwrap(), "1,234.50");
    }

    #[test]
    fn test_str_basic() {
        assert_eq!(format_str_spec("abcdef", ".3").unwrap(), "abc");
        assert_eq!(format_str_spec("hi", "5").unwrap(), "hi   ");
        assert_eq!(format_str_spec("hi", ">5").unwrap(), "   hi");
        assert_eq!(format_str_spec("hi", "*^9").unwrap(), "***hi****");
    }

    #[test]
    fn test_bool_basic() {
        assert_eq!(format_bool_spec(true, "d").unwrap(), "1");
        assert_eq!(format_bool_spec(false, "d").unwrap(), "0");
        // CPython: a non-empty spec formats the integer 1/0 (bool → int.__format__).
        assert_eq!(format_bool_spec(true, "5").unwrap(), "    1");
        assert_eq!(format_bool_spec(false, "5").unwrap(), "    0");
        // 's' is a string-only type code — invalid for bool/int.
        assert!(format_bool_spec(true, "5s").is_err());
        assert_eq!(format_bool_spec(true, "5d").unwrap(), "    1");
        assert_eq!(format_bool_spec(false, "08b").unwrap(), "00000000");
    }
}
