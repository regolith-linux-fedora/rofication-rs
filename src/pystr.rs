//! Python string and number semantics the original implementation relies on.

use std::fmt::Write as _;

use crate::bigint::BigInt;

/// `str.isspace()` for one character (Python's whitespace set, which unlike
/// Rust's `char::is_whitespace` includes the separators U+001C..U+001F).
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// `str.strip()`.
pub fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// `str.split()` without arguments.
pub fn split_whitespace(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_space).filter(|part| !part.is_empty())
}

/// Python's default limit on the digits `int()` converts.
const MAX_STR_DIGITS: usize = 4300;

/// `int(text)` in base 10: surrounding whitespace, one optional sign, ASCII
/// digits with single underscores between them. `None` is the `ValueError`.
pub fn parse_int(text: &str) -> Option<BigInt> {
    let text = strip(text);
    let (negative, rest) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut digits = Vec::with_capacity(rest.len());
    let mut after_digit = false;
    for b in rest.bytes() {
        match b {
            b'0'..=b'9' => {
                digits.push(b);
                after_digit = true;
            }
            b'_' if after_digit => after_digit = false,
            _ => return None,
        }
    }
    if digits.is_empty() || !after_digit || digits.len() > MAX_STR_DIGITS {
        return None;
    }
    Some(BigInt::from_digits(negative, &digits))
}

/// `repr(float)`: the shortest round-tripping digits, positional notation
/// for decimal exponents in [-4, 16), scientific notation otherwise.
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    // `{:e}` yields the shortest round-tripping digits, e.g. "-1.2345e-5".
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific
        .split_once('e')
        .expect("LowerExp always has an exponent");
    let exponent: i32 = exponent.parse().expect("LowerExp exponent is decimal");
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    // The decimal point sits after `decpt` digits.
    let decpt = exponent + 1;
    let mut out = String::with_capacity(digits.len() + 8);
    if negative {
        out.push('-');
    }
    if !(-4 < decpt && decpt <= 16) {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let shown = decpt - 1;
        let _ = write!(
            out,
            "e{}{:02}",
            if shown < 0 { '-' } else { '+' },
            shown.unsigned_abs()
        );
    } else if decpt <= 0 {
        out.push_str("0.");
        for _ in 0..(-decpt) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let decpt = decpt as usize;
        if decpt >= digits.len() {
            out.push_str(&digits);
            for _ in digits.len()..decpt {
                out.push('0');
            }
            out.push_str(".0");
        } else {
            out.push_str(&digits[..decpt]);
            out.push('.');
            out.push_str(&digits[decpt..]);
        }
    }
    out
}

/// Universal-newlines text decoding: `\r\n` and lone `\r` become `\n`.
pub fn translate_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_match_python_repr() {
        let cases = [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.5, "1.5"),
            (42.0, "42.0"),
            (3.14159, "3.14159"),
            (1e300, "1e+300"),
            (5e-324, "5e-324"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-0.025, "-0.025"),
            (1578364000.0, "1578364000.0"),
            (0.30000000000000004, "0.30000000000000004"),
            (1577836861.123456, "1577836861.123456"),
            (1e-20, "1e-20"),
            (1e20, "1e+20"),
            (123456789012345680.0, "1.2345678901234568e+17"),
        ];
        for (value, expected) in cases {
            assert_eq!(float_repr(value), expected, "{value:e}");
        }
    }

    #[test]
    fn int_parsing_matches_python() {
        let ok = [
            ("01", "1"),
            ("+2", "2"),
            ("-1", "-1"),
            (" 3", "3"),
            ("3 ", "3"),
            ("1_0", "10"),
            ("\t1", "1"),
            ("-0", "0"),
        ];
        for (text, expected) in ok {
            assert_eq!(parse_int(text).unwrap().to_string(), expected, "{text:?}");
        }
        for text in [
            "", " ", "0x10", "1.0", "_1", "1_", "1__0", "+-1", "abc", "- 1",
        ] {
            assert!(parse_int(text).is_none(), "{text:?}");
        }
    }

    #[test]
    fn whitespace_set() {
        assert_eq!(strip("\u{1c} x \u{a0}"), "x");
        assert_eq!(
            split_whitespace(" a\t\nb\u{85}c ").collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }
}
