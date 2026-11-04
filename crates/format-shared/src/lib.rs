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
    insert_grouping_n(digits, sep, 3)
}

/// [`insert_grouping`] with an explicit group size (CPython groups `_` by 4
/// for `b`/`o`/`x`/`X`).
pub fn insert_grouping_n(digits: &str, sep: char, group: usize) -> String {
    let chars: Vec<char> = digits.chars().collect();
    let mut out = String::with_capacity(chars.len() + chars.len() / group);
    for (i, c) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % group == 0 {
            out.push(sep);
        }
        out.push(*c);
    }
    out
}

/// Apply grouping separators to a formatted integer or float string.
pub fn apply_grouping_to_number(s: &str, sep: char) -> String {
    apply_grouping_to_number_n(s, sep, 3)
}

/// Split a formatted number into `(lead, int_digits, rest)`: sign and
/// `0x`-style prefix, the integer digits, and everything after them.
fn split_number(s: &str, hex: bool) -> (&str, &str, &str) {
    let chars: &[u8] = s.as_bytes();
    let mut i = 0;
    if !chars.is_empty() && matches!(chars[0], b'+' | b'-' | b' ') {
        i = 1;
    }
    if i + 1 < chars.len()
        && chars[i] == b'0'
        && matches!(chars[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
    {
        i += 2;
    }
    let start = i;
    while i < chars.len() && (chars[i].is_ascii_digit() || (hex && chars[i].is_ascii_hexdigit())) {
        i += 1;
    }
    (&s[..start], &s[start..i], &s[i..])
}

/// [`apply_grouping_to_number`] with an explicit group size.
pub fn apply_grouping_to_number_n(s: &str, sep: char, group: usize) -> String {
    let (lead, digits, tail) = split_number(s, group == 4);
    format!("{}{}{}", lead, insert_grouping_n(digits, sep, group), tail)
}

/// Group a number (`grouping`) and pad it to `spec.width`. With the `0` flag
/// (fill `0`, align `=`) the separators continue into the zero padding, as
/// CPython does (`format(1234, '010,')` → `00,001,234`).
fn group_and_pad(s: &str, spec: &FormatSpec, group: usize) -> String {
    let Some(sep) = spec.grouping else {
        return apply_padding(s, spec);
    };
    let zero_fill = spec.fill == '0' && spec.align == Some('=');
    match (zero_fill, spec.width) {
        (true, Some(width)) => {
            let (lead, digits, rest) = split_number(s, group == 4);
            let mut digits = digits.to_string();
            loop {
                let g = insert_grouping_n(&digits, sep, group);
                if lead.chars().count() + g.chars().count() + rest.chars().count() >= width {
                    return format!("{}{}{}", lead, g, rest);
                }
                digits.insert(0, '0');
            }
        }
        _ => apply_padding(&apply_grouping_to_number_n(s, sep, group), spec),
    }
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
        if sep == ',' && !matches!(type_spec, 'd' | 'n') {
            return Err(format!("Cannot specify ',' with '{}'.", type_spec));
        }
    }
    let group = if matches!(type_spec, 'b' | 'o' | 'x' | 'X') {
        4
    } else {
        3
    };
    Ok(group_and_pad(&result, spec, group))
}

/// Decimal digits of a finite, non-negative `v`: `(digits, decpt)` with
/// `v ≈ 0.d1d2d3… × 10**decpt`. `sig = None` gives the shortest round-trip
/// digits (repr); `Some(n)` rounds correctly to `n >= 1` significant digits.
fn decimal_digits(v: f64, sig: Option<usize>) -> (String, i32) {
    if v == 0.0 {
        return ("0".to_string(), 1);
    }
    let s = match sig {
        Some(n) => format!("{:.*e}", n - 1, v),
        None => format!("{:e}", v),
    };
    let (mant, exp) = s.split_once('e').expect("LowerExp output has an exponent");
    let exp: i32 = exp.parse().expect("integer exponent");
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    (digits, exp + 1)
}

/// Place a decimal point into `digits` at `decpt` (fixed notation).
fn fixed_from_digits(digits: &str, decpt: i32) -> String {
    let n = digits.len() as i32;
    if decpt <= 0 {
        format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
    } else if decpt >= n {
        format!("{}{}", digits, "0".repeat((decpt - n) as usize))
    } else {
        format!(
            "{}.{}",
            &digits[..decpt as usize],
            &digits[decpt as usize..]
        )
    }
}

/// Exponent notation from `digits`: `d[.ddd]e±XX` (at least two exponent
/// digits, as CPython).
fn exp_from_digits(digits: &str, decpt: i32, upper: bool) -> String {
    let mut s = String::new();
    s.push_str(&digits[..1]);
    if digits.len() > 1 {
        s.push('.');
        s.push_str(&digits[1..]);
    }
    let e = decpt - 1;
    s.push(if upper { 'E' } else { 'e' });
    s.push(if e < 0 { '-' } else { '+' });
    s.push_str(&format!("{:02}", e.unsigned_abs()));
    s
}

/// CPython's `PyOS_double_to_string` for a finite, non-negative `v`, without
/// sign. `mode`: `'f'`/`'e'`/`'g'` (with `precision`) or `'r'` (repr,
/// shortest digits). `add_dot_0` is the "no type code" flag: like `g`, but a
/// fixed-notation result always keeps a fractional digit, and the switch to
/// exponent notation happens one decimal position earlier.
pub fn float_body(
    v: f64,
    mode: char,
    precision: usize,
    alternate: bool,
    add_dot_0: bool,
) -> String {
    let upper = mode.is_ascii_uppercase();
    match mode.to_ascii_lowercase() {
        'f' => {
            let mut s = format!("{:.*}", precision, v);
            if alternate && precision == 0 {
                s.push('.');
            }
            s
        }
        'e' => {
            let (mut digits, decpt) = decimal_digits(v, Some(precision + 1));
            if v == 0.0 {
                digits = "0".repeat(precision + 1);
            }
            let mut s = exp_from_digits(&digits, decpt, upper);
            if alternate && precision == 0 {
                let e_pos = s.find(['e', 'E']).unwrap_or(s.len());
                s.insert(e_pos, '.');
            }
            s
        }
        mode @ ('g' | 'r') => {
            let (mut digits, decpt) = if mode == 'r' {
                decimal_digits(v, None)
            } else {
                decimal_digits(v, Some(precision.max(1)))
            };
            let p = if mode == 'r' {
                17
            } else {
                precision.max(1) as i32
            };
            let use_exp = if mode == 'r' {
                decpt <= -4 || decpt > 16
            } else {
                decpt <= -4 || decpt > if add_dot_0 { p - 1 } else { p }
            };
            if alternate && mode == 'g' {
                // `#`: keep trailing zeros up to the precision.
                while (digits.len() as i32) < p {
                    digits.push('0');
                }
            } else {
                let trimmed = digits.trim_end_matches('0');
                digits = if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                };
            }
            if use_exp {
                exp_from_digits(&digits, decpt, upper)
            } else {
                let mut s = fixed_from_digits(&digits, decpt);
                if !s.contains('.') && (add_dot_0 || alternate) {
                    s.push_str(if add_dot_0 { ".0" } else { "." });
                }
                s
            }
        }
        _ => unreachable!("float_body mode"),
    }
}

/// `repr(x)` / `str(x)` of a float, exactly as CPython.
pub fn float_repr(v: f64) -> String {
    if v.is_nan() {
        return "nan".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let body = float_body(v.abs(), 'r', 0, false, true);
    if v.is_sign_negative() {
        format!("-{}", body)
    } else {
        body
    }
}

/// Format a float value according to a parsed `FormatSpec` (CPython's
/// `format_float_internal`): no type code is repr-like (`add_dot_0`), `g`
/// decides fixed vs exponent AFTER rounding, `-0.0` keeps its sign, `nan` /
/// `inf` are lower-case (upper-case for `F`/`E`/`G`), and grouping continues
/// into `0`-flag padding.
pub fn format_float(value: f64, spec: &FormatSpec) -> Result<String, String> {
    let (mode, precision, add_dot_0) = match spec.type_spec {
        None => match spec.precision {
            None => ('r', 0, true),
            Some(p) => ('g', p, true),
        },
        Some(t @ ('f' | 'F' | 'e' | 'E' | 'g' | 'G')) => (t, spec.precision.unwrap_or(6), false),
        Some('n') => ('g', spec.precision.unwrap_or(6), false),
        Some('%') => ('%', spec.precision.unwrap_or(6), false),
        Some(t) => {
            return Err(format!(
                "Unknown format code '{}' for object of type 'float'",
                t
            ))
        }
    };
    if let Some(sep) = spec.grouping {
        if spec.type_spec == Some('n') {
            return Err(format!("Cannot specify '{}' with 'n'.", sep));
        }
    }
    let upper = matches!(mode, 'F' | 'E' | 'G');
    let negative = value.is_sign_negative() && !value.is_nan();
    let magnitude = value.abs();
    let mut body = if value.is_nan() {
        "nan".to_string()
    } else if value.is_infinite() {
        "inf".to_string()
    } else if mode == '%' {
        format!(
            "{}%",
            float_body(magnitude * 100.0, 'f', precision, spec.alternate, false)
        )
    } else {
        float_body(magnitude, mode, precision, spec.alternate, add_dot_0)
    };
    if !value.is_finite() && mode == '%' {
        body.push('%');
    }
    if upper {
        body = body.to_uppercase();
    }
    let sign = if negative {
        "-"
    } else {
        match spec.sign {
            Some('+') => "+",
            Some(' ') => " ",
            _ => "",
        }
    };
    let result = format!("{}{}", sign, body);
    if !value.is_finite() {
        // No grouping / zero-fill digits for nan/inf: plain padding.
        return Ok(apply_padding(&result, spec));
    }
    Ok(group_and_pad(&result, spec, 3))
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
    fn test_float_cpython_edges() {
        let cases: &[(f64, &str, &str)] = &[
            (1.0, ".3", "1.0"),
            (1234.5, ".3", "1.23e+03"),
            (12.0, ".2", "1.2e+01"),
            (12.0, "", "12.0"),
            (1e16, "", "1e+16"),
            (1e-5, "", "1e-05"),
            (0.0001, "", "0.0001"),
            (123.456, "10", "   123.456"),
            (f64::NAN, "f", "nan"),
            (f64::INFINITY, "F", "INF"),
            (-0.0, "+.1f", "-0.0"),
            (-0.0, "", "-0.0"),
            (9.9999, ".3g", "10"),
            (0.00009999, ".3g", "0.0001"),
            (99999.5, ".5g", "1e+05"),
            (1234567.0, ",.2f", "1,234,567.00"),
            (1234.5, "010,.1f", "0,001,234.5"),
            (1e100, ".3", "1e+100"),
            (5.0, ".0", "5e+00"),
            (2.5, ".0f", "2"),
            (-1.5, "08.2f", "-0001.50"),
            (1.0, "#g", "1.00000"),
            (1.0, "g", "1"),
            (100.0, ".2", "1e+02"),
            (1.0, "%", "100.000000%"),
            (f64::NAN, "+", "+nan"),
            (f64::NAN, "010", "0000000nan"),
            (123456789.0, ",g", "1.23457e+08"),
            (0.1, ".17", "0.10000000000000001"),
        ];
        for (v, spec, want) in cases {
            assert_eq!(
                &format_float_spec(*v, spec).unwrap(),
                want,
                "{} {:?}",
                v,
                spec
            );
        }
        assert_eq!(format_int_spec(1234, "010,").unwrap(), "00,001,234");
        assert_eq!(format_int_spec(1234, "010_x").unwrap(), "0_0000_04d2");
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(1e22), "1e+22");
        assert_eq!(float_repr(-2.5), "-2.5");
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
