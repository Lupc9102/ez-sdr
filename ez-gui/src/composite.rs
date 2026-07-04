//! Channel composite expression parser and Meteor MSU-MR presets.
//!
//! Supports arithmetic (`+`, `-`, `*`, `/`, `^`), parentheses, variable
//! references (`ch1`..`ch6`), ternary operators (`cond?t:f`), and
//! comparisons (`<`, `>`, `<=`, `>=`, `==`) in ternary conditions.

// ── Presets ──────────────────────────────────────────────────────────────────

/// A predefined channel composite for Meteor MSU-MR.
pub struct CompositePreset {
    pub name: &'static str,
    pub description: &'static str,
    pub r_expr: &'static str,
    pub g_expr: &'static str,
    pub b_expr: &'static str,
}

/// Standard composites for Meteor LRPT MSU-MR (6 channels).
pub const METEOR_COMPOSITES: &[CompositePreset] = &[
    CompositePreset {
        name: "221",
        description: "Standard visible color",
        r_expr: "ch2",
        g_expr: "ch2",
        b_expr: "ch1",
    },
    CompositePreset {
        name: "421",
        description: "VIS/IR blend",
        r_expr: "ch4",
        g_expr: "ch2",
        b_expr: "ch1",
    },
    CompositePreset {
        name: "321",
        description: "False color IR",
        r_expr: "ch3",
        g_expr: "ch2",
        b_expr: "ch1",
    },
    CompositePreset {
        name: "654",
        description: "Thermal IR",
        r_expr: "ch6",
        g_expr: "ch5",
        b_expr: "ch4",
    },
    CompositePreset {
        name: "543",
        description: "IR composite",
        r_expr: "ch5",
        g_expr: "ch4",
        b_expr: "ch3",
    },
    CompositePreset {
        name: "Natural Color",
        description: "True-ish visible color",
        r_expr: "ch2",
        g_expr: "ch2",
        b_expr: "ch1",
    },
    CompositePreset {
        name: "MCIR",
        description: "Cloud IR (single channel)",
        r_expr: "ch4",
        g_expr: "ch4",
        b_expr: "ch4",
    },
    CompositePreset {
        name: "NDVI",
        description: "Vegetation index approximation",
        r_expr: "ch2-ch1",
        g_expr: "ch2+ch1",
        b_expr: "ch1",
    },
    CompositePreset {
        name: "Day/Night",
        description: "VIS day, IR night blend",
        r_expr: "ch1>0.1?ch2:ch4",
        g_expr: "ch1>0.1?ch2:ch4",
        b_expr: "ch1>0.1?ch1:ch5",
    },
    CompositePreset {
        name: "Thermal Enhanced",
        description: "Gamma-stretched thermal",
        r_expr: "ch6^0.7",
        g_expr: "ch5^0.7",
        b_expr: "ch4^0.7",
    },
];

// ── Expression AST ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CmpOp {
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f32),
    Var(u8), // ch1..ch6 -> index 0..5
    BinOp {
        op: char,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        cmp_op: CmpOp,
        cmp_val: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
}

impl Expr {
    /// Evaluate expression for one pixel. `channels` is [ch1, ch2, ch3, ch4, ch5, ch6]
    /// with values in 0.0..1.0.
    pub fn eval(&self, channels: &[f32; 6]) -> f32 {
        match self {
            Expr::Num(v) => *v,
            Expr::Var(idx) => {
                if (*idx as usize) < channels.len() {
                    channels[*idx as usize]
                } else {
                    0.0
                }
            }
            Expr::BinOp { op, left, right } => {
                let l = left.eval(channels);
                let r = right.eval(channels);
                match op {
                    '+' => l + r,
                    '-' => l - r,
                    '*' => l * r,
                    '/' => {
                        if r.abs() < 1e-10 {
                            0.0
                        } else {
                            l / r
                        }
                    }
                    '^' => l.powf(r),
                    _ => 0.0,
                }
            }
            Expr::Ternary {
                cond,
                cmp_op,
                cmp_val,
                then_branch,
                else_branch,
            } => {
                let c = cond.eval(channels);
                let cv = cmp_val.eval(channels);
                let cond_met = match cmp_op {
                    CmpOp::Lt => c < cv,
                    CmpOp::Gt => c > cv,
                    CmpOp::Le => c <= cv,
                    CmpOp::Ge => c >= cv,
                    CmpOp::Eq => (c - cv).abs() < 1e-6,
                };
                if cond_met {
                    then_branch.eval(channels)
                } else {
                    else_branch.eval(channels)
                }
            }
        }
    }
}

// ── Tokenizer ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f32),
    Var(u8), // 0..5 for ch1..ch6
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Question,
    Colon,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
            }
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '?' => {
                tokens.push(Token::Question);
                i += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            '<' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::Le);
                    i += 2;
                } else {
                    tokens.push(Token::Lt);
                    i += 1;
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    tokens.push(Token::Gt);
                    i += 1;
                }
            }
            '=' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token::Eq);
                    i += 2;
                } else {
                    return Err(format!(
                        "Unexpected '=' at position {i}, did you mean '=='?"
                    ));
                }
            }
            'c' => {
                // Parse ch1..ch6
                if i + 2 < chars.len() && chars[i + 1] == 'h' {
                    if let Some(digit) = chars[i + 2].to_digit(10) {
                        if (1..=6).contains(&digit) {
                            tokens.push(Token::Var((digit - 1) as u8));
                            i += 3;
                            continue;
                        }
                    }
                }
                return Err(format!("Unexpected 'c' at position {i}"));
            }
            '0'..='9' | '.' => {
                // Parse number
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let num_str: String = chars[start..i].iter().collect();
                let num: f32 = num_str
                    .parse()
                    .map_err(|_| format!("Invalid number '{num_str}'"))?;
                tokens.push(Token::Num(num));
            }
            other => {
                return Err(format!("Unexpected character '{other}' at position {i}"));
            }
        }
    }

    Ok(tokens)
}

// ── Parser (recursive descent) ──────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let t = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    /// Parse a full expression (entry point).
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_comparison()
    }

    /// Parse comparison (for ternary condition): expr < expr, expr > expr, etc.
    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let left = self.parse_add_sub()?;

        if let Some(Token::Lt | Token::Gt | Token::Le | Token::Ge | Token::Eq) = self.peek() {
            let cmp_op = match self.next().unwrap() {
                Token::Lt => CmpOp::Lt,
                Token::Gt => CmpOp::Gt,
                Token::Le => CmpOp::Le,
                Token::Ge => CmpOp::Ge,
                Token::Eq => CmpOp::Eq,
                _ => unreachable!(),
            };
            let cmp_val = self.parse_add_sub()?;

            // Expect '?'
            if self.next() != Some(Token::Question) {
                return Err("Expected '?' after comparison in ternary".to_string());
            }
            let then_branch = self.parse_expr()?;

            if self.next() != Some(Token::Colon) {
                return Err("Expected ':' in ternary expression".to_string());
            }
            let else_branch = self.parse_expr()?;

            return Ok(Expr::Ternary {
                cond: Box::new(left),
                cmp_op,
                cmp_val: Box::new(cmp_val),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            });
        }

        Ok(left)
    }

    /// Parse addition/subtraction.
    fn parse_add_sub(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_mul_div()?;

        while let Some(Token::Plus | Token::Minus) = self.peek() {
            let op = match self.next().unwrap() {
                Token::Plus => '+',
                Token::Minus => '-',
                _ => unreachable!(),
            };
            let right = self.parse_mul_div()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse multiplication/division.
    fn parse_mul_div(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_power()?;

        while let Some(Token::Star | Token::Slash) = self.peek() {
            let op = match self.next().unwrap() {
                Token::Star => '*',
                Token::Slash => '/',
                _ => unreachable!(),
            };
            let right = self.parse_power()?;
            left = Expr::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// Parse power (right-associative).
    fn parse_power(&mut self) -> Result<Expr, String> {
        let base = self.parse_unary()?;

        if let Some(Token::Caret) = self.peek() {
            self.next();
            let exp = self.parse_power()?; // right-associative
            return Ok(Expr::BinOp {
                op: '^',
                left: Box::new(base),
                right: Box::new(exp),
            });
        }

        Ok(base)
    }

    /// Parse unary minus.
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if let Some(Token::Minus) = self.peek() {
            self.next();
            let expr = self.parse_atom()?;
            return Ok(Expr::BinOp {
                op: '*',
                left: Box::new(Expr::Num(-1.0)),
                right: Box::new(expr),
            });
        }
        self.parse_atom()
    }

    /// Parse atomic expression: number, variable, or parenthesized expression.
    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Token::Num(v)) => Ok(Expr::Num(v)),
            Some(Token::Var(idx)) => Ok(Expr::Var(idx)),
            Some(Token::LParen) => {
                let expr = self.parse_expr()?;
                if self.next() != Some(Token::RParen) {
                    return Err("Expected ')'".to_string());
                }
                Ok(expr)
            }
            Some(t) => Err(format!("Unexpected token: {t:?}")),
            None => Err("Unexpected end of expression".to_string()),
        }
    }
}

/// Parse a composite expression string into an evaluatable AST.
pub fn parse_expression(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("Empty expression".to_string());
    }
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expr()?;
    if parser.pos < parser.tokens.len() {
        return Err(format!(
            "Unexpected tokens after expression: {:?}",
            &parser.tokens[parser.pos..]
        ));
    }
    Ok(expr)
}

/// Evaluate a composite expression for one pixel. Returns raw value (may exceed 0..1).
pub fn evaluate(expr: &Expr, channels: &[f32; 6]) -> f32 {
    expr.eval(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_variable() {
        let expr = parse_expression("ch1").unwrap();
        let val = evaluate(&expr, &[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 0.5).abs() < 0.001);
    }

    #[test]
    fn parse_number() {
        let expr = parse_expression("0.75").unwrap();
        let val = evaluate(&expr, &[0.0; 6]);
        assert!((val - 0.75).abs() < 0.001);
    }

    #[test]
    fn parse_addition() {
        let expr = parse_expression("ch1+ch2").unwrap();
        let val = evaluate(&expr, &[0.3, 0.4, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 0.7).abs() < 0.001);
    }

    #[test]
    fn parse_subtraction() {
        let expr = parse_expression("ch2-ch1").unwrap();
        let val = evaluate(&expr, &[0.3, 0.8, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 0.5).abs() < 0.001);
    }

    #[test]
    fn parse_multiplication() {
        let expr = parse_expression("ch1*ch2").unwrap();
        let val = evaluate(&expr, &[0.5, 0.6, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 0.3).abs() < 0.001);
    }

    #[test]
    fn parse_division() {
        let expr = parse_expression("ch1/ch2").unwrap();
        let val = evaluate(&expr, &[0.6, 0.3, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 2.0).abs() < 0.001);
    }

    #[test]
    fn parse_division_by_zero() {
        let expr = parse_expression("ch1/ch2").unwrap();
        let val = evaluate(&expr, &[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(val, 0.0);
    }

    #[test]
    fn parse_power() {
        let expr = parse_expression("ch1^2.0").unwrap();
        let val = evaluate(&expr, &[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 0.25).abs() < 0.001);
    }

    #[test]
    fn parse_power_right_associative() {
        // 2^3^2 = 2^(3^2) = 2^9 = 512
        let expr = parse_expression("ch1^3^2").unwrap();
        let val = evaluate(&expr, &[2.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 512.0).abs() < 0.01);
    }

    #[test]
    fn parse_parentheses() {
        let expr = parse_expression("(ch1+ch2)*ch3").unwrap();
        let val = evaluate(&expr, &[0.3, 0.4, 0.5, 0.0, 0.0, 0.0]);
        assert!((val - 0.35).abs() < 0.001);
    }

    #[test]
    fn parse_unary_minus() {
        let expr = parse_expression("-ch1").unwrap();
        let val = evaluate(&expr, &[0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn parse_ternary_gt() {
        let expr = parse_expression("ch1>0.5?ch2:ch3").unwrap();
        let val_high = evaluate(&expr, &[0.8, 0.3, 0.7, 0.0, 0.0, 0.0]);
        let val_low = evaluate(&expr, &[0.2, 0.3, 0.7, 0.0, 0.0, 0.0]);
        assert!((val_high - 0.3).abs() < 0.001);
        assert!((val_low - 0.7).abs() < 0.001);
    }

    #[test]
    fn parse_ternary_lt() {
        let expr = parse_expression("ch1<0.5?ch2:ch3").unwrap();
        let val = evaluate(&expr, &[0.2, 0.9, 0.1, 0.0, 0.0, 0.0]);
        assert!((val - 0.9).abs() < 0.001);
    }

    #[test]
    fn parse_ternary_le() {
        let expr = parse_expression("ch1<=0.5?ch2:ch3").unwrap();
        let val_eq = evaluate(&expr, &[0.5, 0.9, 0.1, 0.0, 0.0, 0.0]);
        assert!((val_eq - 0.9).abs() < 0.001);
    }

    #[test]
    fn parse_ternary_eq() {
        let expr = parse_expression("ch1==0.5?ch2:ch3").unwrap();
        let val = evaluate(&expr, &[0.5, 0.9, 0.1, 0.0, 0.0, 0.0]);
        assert!((val - 0.9).abs() < 0.001);
    }

    #[test]
    fn parse_complex_expression() {
        // ch1>0.1?ch2:ch4 (from Day/Night preset)
        let expr = parse_expression("ch1>0.1?ch2:ch4").unwrap();
        let val_day = evaluate(&expr, &[0.5, 0.7, 0.0, 0.3, 0.0, 0.0]);
        let val_night = evaluate(&expr, &[0.01, 0.7, 0.0, 0.3, 0.0, 0.0]);
        assert!((val_day - 0.7).abs() < 0.001);
        assert!((val_night - 0.3).abs() < 0.001);
    }

    #[test]
    fn parse_all_channel_references() {
        let expr = parse_expression("ch1+ch2+ch3+ch4+ch5+ch6").unwrap();
        let val = evaluate(&expr, &[0.1, 0.2, 0.3, 0.4, 0.5, 0.6]);
        assert!((val - 2.1).abs() < 0.001);
    }

    #[test]
    fn parse_error_empty() {
        assert!(parse_expression("").is_err());
    }

    #[test]
    fn parse_error_invalid_char() {
        assert!(parse_expression("ch1 & ch2").is_err());
    }

    #[test]
    fn parse_error_missing_paren() {
        assert!(parse_expression("(ch1+ch2").is_err());
    }

    #[test]
    fn preset_221_combines_correctly() {
        let preset = &METEOR_COMPOSITES[0];
        assert_eq!(preset.name, "221");
        assert_eq!(preset.r_expr, "ch2");
        assert_eq!(preset.g_expr, "ch2");
        assert_eq!(preset.b_expr, "ch1");
    }

    #[test]
    fn preset_count() {
        assert_eq!(METEOR_COMPOSITES.len(), 10);
    }

    #[test]
    fn output_raw_not_clamped() {
        // ch1+ch2 with values > 1.0 total — evaluate returns raw, clamping happens at output
        let expr = parse_expression("ch1+ch2").unwrap();
        let val = evaluate(&expr, &[0.8, 0.9, 0.0, 0.0, 0.0, 0.0]);
        assert!((val - 1.7).abs() < 0.001);
    }
}
