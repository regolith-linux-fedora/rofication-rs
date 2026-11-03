//! Unbounded integers for notification ids.
//!
//! Ids continue after the highest id ever handed out, and the counter file
//! may hold any integer (`int()` accepts arbitrary sizes), so ids are not
//! bounded by `u32`. Only the operations the queue needs exist: parsing,
//! `+ 1`, ordering, comparison with a float and decimal formatting.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BigInt {
    negative: bool,
    /// Decimal digits, most significant first, without leading zeros; empty
    /// for zero (which is never negative).
    digits: Vec<u8>,
}

impl BigInt {
    pub fn zero() -> BigInt {
        BigInt {
            negative: false,
            digits: Vec::new(),
        }
    }

    pub fn from_u64(value: u64) -> BigInt {
        Self::from_digits(false, value.to_string().as_bytes())
    }

    pub fn from_i64(value: i64) -> BigInt {
        Self::from_digits(value < 0, value.unsigned_abs().to_string().as_bytes())
    }

    /// Build from ASCII decimal digits (leading zeros allowed).
    pub fn from_digits(negative: bool, ascii: &[u8]) -> BigInt {
        let digits: Vec<u8> = ascii
            .iter()
            .map(|b| b - b'0')
            .skip_while(|d| *d == 0)
            .collect();
        BigInt {
            negative: negative && !digits.is_empty(),
            digits,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.digits.is_empty()
    }

    /// `self + 1`.
    pub fn succ(&self) -> BigInt {
        if self.negative {
            // -(m - 1)
            let mut digits = self.digits.clone();
            let mut i = digits.len();
            loop {
                i -= 1;
                if digits[i] == 0 {
                    digits[i] = 9;
                } else {
                    digits[i] -= 1;
                    break;
                }
            }
            let digits: Vec<u8> = digits.into_iter().skip_while(|d| *d == 0).collect();
            BigInt {
                negative: !digits.is_empty(),
                digits,
            }
        } else {
            let mut digits = self.digits.clone();
            let mut i = digits.len();
            loop {
                if i == 0 {
                    digits.insert(0, 1);
                    break;
                }
                i -= 1;
                if digits[i] == 9 {
                    digits[i] = 0;
                } else {
                    digits[i] += 1;
                    break;
                }
            }
            BigInt {
                negative: false,
                digits,
            }
        }
    }

    pub fn to_u32(&self) -> Option<u32> {
        if self.negative || self.digits.len() > 10 {
            return None;
        }
        let mut value: u64 = 0;
        for d in &self.digits {
            value = value * 10 + u64::from(*d);
        }
        u32::try_from(value).ok()
    }

    pub fn to_i64(&self) -> Option<i64> {
        if self.digits.len() > 19 {
            return None;
        }
        let mut value: i128 = 0;
        for d in &self.digits {
            value = value * 10 + i128::from(*d);
        }
        if self.negative {
            value = -value;
        }
        i64::try_from(value).ok()
    }

    /// Nearest float (Python compares int and float exactly; the nearest
    /// float is exact for every value a timestamp comparison meets).
    pub fn to_f64(&self) -> f64 {
        self.to_string().parse().unwrap_or(f64::NAN)
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (negative, _) => {
                let magnitude = self
                    .digits
                    .len()
                    .cmp(&other.digits.len())
                    .then_with(|| self.digits.cmp(&other.digits));
                if negative {
                    magnitude.reverse()
                } else {
                    magnitude
                }
            }
        }
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for BigInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.digits.is_empty() {
            return f.write_str("0");
        }
        let mut text = String::with_capacity(self.digits.len() + 1);
        if self.negative {
            text.push('-');
        }
        text.extend(self.digits.iter().map(|d| char::from(b'0' + d)));
        f.write_str(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::BigInt;

    fn n(text: &str) -> BigInt {
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text),
        };
        BigInt::from_digits(negative, digits.as_bytes())
    }

    #[test]
    fn succ_and_display() {
        assert_eq!(n("0").succ().to_string(), "1");
        assert_eq!(n("-1").succ().to_string(), "0");
        assert_eq!(n("-10").succ().to_string(), "-9");
        assert_eq!(n("999").succ().to_string(), "1000");
        assert_eq!(n("-0").to_string(), "0");
        assert_eq!(n("007").to_string(), "7");
        assert_eq!(
            n("1180591620717411303424").succ().to_string(),
            "1180591620717411303425"
        );
    }

    #[test]
    fn ordering_and_ranges() {
        assert!(n("-2") < n("-1"));
        assert!(n("-1") < n("0"));
        assert!(n("9") < n("10"));
        assert_eq!(n("4294967295").to_u32(), Some(u32::MAX));
        assert_eq!(n("4294967296").to_u32(), None);
        assert_eq!(n("-1").to_u32(), None);
    }
}
