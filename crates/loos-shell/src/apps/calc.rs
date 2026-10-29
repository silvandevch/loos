//! Calculator: decimal fixed-point expressions (+ - * / % parentheses, unary minus).
//! No floats (the kernel target is soft-float): values are i128 with 12 fractional digits.

use alloc::string::String;

const SCALE: i128 = 1_000_000_000_000;

/// Evaluation error.
#[derive(Debug, PartialEq, Eq)]
pub enum CalcError {
    /// Malformed expression.
    Syntax,
    /// Division by zero.
    DivZero,
    /// Value out of range.
    Overflow,
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i] == b' ' {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn expr(&mut self) -> Result<i128, CalcError> {
        let mut v = self.term()?;
        while let Some(op @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if op == b'+' { v.checked_add(r) } else { v.checked_sub(r) }.ok_or(CalcError::Overflow)?;
        }
        Ok(v)
    }
    fn term(&mut self) -> Result<i128, CalcError> {
        let mut v = self.unary()?;
        while let Some(op @ (b'*' | b'/' | b'%')) = self.peek() {
            self.i += 1;
            let r = self.unary()?;
            v = match op {
                b'*' => v.checked_mul(r).map(|x| x / SCALE),
                b'/' if r == 0 => return Err(CalcError::DivZero),
                b'/' => v.checked_mul(SCALE).map(|x| x / r),
                _ if r == 0 => return Err(CalcError::DivZero),
                _ => Some(v % r),
            }
            .ok_or(CalcError::Overflow)?;
        }
        Ok(v)
    }
    fn unary(&mut self) -> Result<i128, CalcError> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Ok(-self.unary()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.unary()
            }
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err(CalcError::Syntax);
                }
                self.i += 1;
                Ok(v)
            }
            _ => self.number(),
        }
    }
    fn number(&mut self) -> Result<i128, CalcError> {
        self.ws();
        let (mut int, mut frac, mut fdig, mut any, mut dot) = (0i128, 0i128, 0u32, false, false);
        while let Some(&c) = self.s.get(self.i) {
            match c {
                b'0'..=b'9' if !dot => int = int.checked_mul(10).and_then(|x| x.checked_add((c - b'0') as i128)).ok_or(CalcError::Overflow)?,
                b'0'..=b'9' if fdig < 12 => { frac = frac * 10 + (c - b'0') as i128; fdig += 1; }
                b'0'..=b'9' => {}
                b'.' if !dot => dot = true,
                _ => break,
            }
            any |= c != b'.';
            self.i += 1;
        }
        if !any {
            return Err(CalcError::Syntax);
        }
        let frac = frac * 10i128.pow(12 - fdig);
        int.checked_mul(SCALE).and_then(|x| x.checked_add(frac)).ok_or(CalcError::Overflow)
    }
}

/// Evaluates `expr` (`x` and `×`/`÷` accepted for * and /).
pub fn eval(expr: &str) -> Result<i128, CalcError> {
    let norm: String = expr.chars().map(|c| match c { 'x' | 'X' | '×' => '*', '÷' => '/', ',' => '.', c => c }).collect();
    let mut p = P { s: norm.as_bytes(), i: 0 };
    let v = p.expr()?;
    if p.peek().is_some() {
        return Err(CalcError::Syntax);
    }
    Ok(v)
}

/// Formats a fixed-point value, trimming trailing zeros ("2.5", "-3", "0.333333333333").
pub fn format(v: i128) -> String {
    let neg = v < 0;
    let a = v.unsigned_abs();
    let (int, frac) = (a / SCALE as u128, a % SCALE as u128);
    let mut s = alloc::format!("{}{}", if neg { "-" } else { "" }, int);
    if frac != 0 {
        let f = alloc::format!("{:012}", frac);
        s.push('.');
        s.push_str(f.trim_end_matches('0'));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates() {
        let f = |e| format(eval(e).unwrap());
        assert_eq!(f("1+2*3"), "7");
        assert_eq!(f("(1+2)*3"), "9");
        assert_eq!(f("10/4"), "2.5");
        assert_eq!(f("-2.5 x 2"), "-5");
        assert_eq!(f("1/3"), "0.333333333333");
        assert_eq!(f("7 % 3"), "1");
        assert_eq!(eval("1/0"), Err(CalcError::DivZero));
        assert_eq!(eval("2+"), Err(CalcError::Syntax));
        assert_eq!(eval("(1"), Err(CalcError::Syntax));
    }
}
