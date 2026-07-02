//! OpenQASM 2.0 Parser for QOS
//!
//! Parses a subset of OpenQASM 2.0 sufficient for real quantum circuits.
//! Supported gates: h, x, y, z, s, t, cx, cz, swap, measure, reset, and the **parametric**
//! rotations rx(θ), ry(θ), rz(θ), p(θ) with angle expressions like `pi/2`, `-pi/4`, `3*pi/2`,
//! or plain decimals.

use alloc::string::String;
use alloc::vec::Vec;

/// A parsed quantum instruction
#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    /// Hadamard gate on qubit
    H(usize),
    /// Pauli-X gate on qubit
    X(usize),
    /// Pauli-Y gate on qubit
    Y(usize),
    /// Pauli-Z gate on qubit
    Z(usize),
    /// S gate (phase) on qubit
    S(usize),
    /// T gate on qubit
    T(usize),
    /// RX(θ): parametric rotation about X
    Rx(usize, f64),
    /// RY(θ): parametric rotation about Y
    Ry(usize, f64),
    /// RZ(θ): parametric rotation about Z
    Rz(usize, f64),
    /// P(θ): parametric phase gate diag(1, e^{iθ})
    P(usize, f64),
    /// CNOT: control, target
    Cx(usize, usize),
    /// CZ gate: control, target
    Cz(usize, usize),
    /// SWAP: qubit1, qubit2
    Swap(usize, usize),
    /// Measure qubit to classical bit
    Measure(usize, usize),
    /// Barrier (ignored in simulation, just parsed)
    Barrier(Vec<usize>),
    /// Reset qubit to |0⟩
    Reset(usize),
}

/// Parse error kinds
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    MissingHeader,
    InvalidSyntax(String),
    UndefinedRegister(String),
    QubitOutOfRange(usize, usize),
    UnsupportedGate(String),
}

/// A parse error carrying its **1-based source line** (0 = whole-program error, e.g. a global
/// qubit-count violation) — so editors/CLIs can point at the offending line (WP-07 slice 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub kind: ParseErrorKind,
}

impl ParseError {
    /// An error not attributable to a single line.
    pub fn program(kind: ParseErrorKind) -> Self {
        ParseError { line: 0, kind }
    }

    /// Short human-readable message (used by the IDE problems row and the shell).
    pub fn message(&self) -> alloc::string::String {
        use alloc::format;
        let what = match &self.kind {
            ParseErrorKind::MissingHeader => format!("missing OPENQASM header"),
            ParseErrorKind::InvalidSyntax(s) => format!("syntax: {}", s),
            ParseErrorKind::UndefinedRegister(s) => format!("undefined register '{}'", s),
            ParseErrorKind::QubitOutOfRange(q, max) => format!("qubit {} out of range (max {})", q, max),
            ParseErrorKind::UnsupportedGate(s) => format!("unsupported gate '{}'", s),
        };
        if self.line > 0 {
            format!("line {}: {}", self.line, what)
        } else {
            what
        }
    }
}

/// Build a [`ParseError`] at the lexer's current source line.
fn perr(lexer: &Lexer, kind: ParseErrorKind) -> ParseError {
    ParseError { line: lexer.line(), kind }
}

/// Parsed QASM program
#[derive(Clone, Debug)]
pub struct QasmProgram {
    pub n_qubits: usize,
    pub n_cbits: usize,
    pub instructions: Vec<Instruction>,
}

/// Tokenizer helper
struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    /// Current 1-based source line (computed on demand — only the error path pays for it).
    fn line(&self) -> usize {
        1 + self.input[..self.pos.min(self.input.len())]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
    }

    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n' {
                self.pos += 1;
            } else if ch == b'/' && self.pos + 1 < self.input.len() && self.input[self.pos + 1] == b'/' {
                // Line comment
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }

    fn advance(&mut self) {
        if self.pos < self.input.len() {
            self.pos += 1;
        }
    }

    fn expect(&mut self, ch: u8) -> bool {
        self.skip_whitespace_and_comments();
        if self.peek() == Some(ch) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn read_identifier(&mut self) -> Option<String> {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.input[self.pos];
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos > start {
            let s = &self.input[start..self.pos];
            Some(String::from_utf8_lossy(s).into_owned())
        } else {
            None
        }
    }

    fn read_number(&mut self) -> Option<usize> {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos > start {
            let s = &self.input[start..self.pos];
            let num_str = core::str::from_utf8(s).ok()?;
            num_str.parse().ok()
        } else {
            None
        }
    }

    /// Read an unsigned decimal number (integer or fraction) as f64.
    fn read_float(&mut self) -> Option<f64> {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if self.pos > start {
            core::str::from_utf8(&self.input[start..self.pos]).ok()?.parse().ok()
        } else {
            None
        }
    }

    fn skip_until_semicolon(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos] != b';' {
            self.pos += 1;
        }
        if self.pos < self.input.len() {
            self.pos += 1; // skip the semicolon
        }
    }

    fn is_eof(&mut self) -> bool {
        self.skip_whitespace_and_comments();
        self.pos >= self.input.len()
    }
}

/// One factor of an angle expression: `pi` or a decimal literal.
fn parse_angle_factor(lexer: &mut Lexer) -> Result<f64, ParseError> {
    lexer.skip_whitespace_and_comments();
    match lexer.peek() {
        Some(c) if c.is_ascii_alphabetic() => {
            let ident = match lexer.read_identifier() {
                Some(i) => i,
                None => return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected angle")))),
            };
            if ident == "pi" {
                Ok(core::f64::consts::PI)
            } else {
                Err(perr(lexer, ParseErrorKind::InvalidSyntax(ident)))
            }
        }
        _ => match lexer.read_float() {
            Some(v) => Ok(v),
            None => Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected angle")))),
        },
    }
}

/// Parse an angle expression: `[-] factor (('*' | '/') factor)*` — covers the common QASM forms
/// `pi/2`, `-pi/4`, `3*pi/2`, `0.785`, `2*pi`.
fn parse_angle(lexer: &mut Lexer) -> Result<f64, ParseError> {
    lexer.skip_whitespace_and_comments();
    let neg = if lexer.peek() == Some(b'-') {
        lexer.advance();
        true
    } else {
        false
    };
    let mut value = parse_angle_factor(lexer)?;
    loop {
        lexer.skip_whitespace_and_comments();
        match lexer.peek() {
            Some(b'*') => {
                lexer.advance();
                value *= parse_angle_factor(lexer)?;
            }
            Some(b'/') => {
                lexer.advance();
                let d = parse_angle_factor(lexer)?;
                if d == 0.0 {
                    return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("division by zero"))));
                }
                value /= d;
            }
            _ => break,
        }
    }
    Ok(if neg { -value } else { value })
}

/// Parse a qubit reference like "q[0]" or just "q" (for single-qubit regs)
fn parse_qubit_ref(lexer: &mut Lexer, qreg_name: &str, n_qubits: usize) -> Result<usize, ParseError> {
    let name = match lexer.read_identifier() {
        Some(n) => n,
        None => return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected qubit identifier")))),
    };

    if name != qreg_name {
        return Err(perr(lexer, ParseErrorKind::UndefinedRegister(name)));
    }

    lexer.skip_whitespace_and_comments();
    if lexer.peek() == Some(b'[') {
        lexer.advance();
        let idx = match lexer.read_number() {
            Some(n) => n,
            None => return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected qubit index")))),
        };
        if !lexer.expect(b']') {
            return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected ']'"))));
        }
        if idx >= n_qubits {
            return Err(perr(lexer, ParseErrorKind::QubitOutOfRange(idx, n_qubits)));
        }
        Ok(idx)
    } else {
        // Assume index 0 for single-qubit reg
        if n_qubits == 0 {
            return Err(perr(lexer, ParseErrorKind::QubitOutOfRange(0, n_qubits)));
        }
        Ok(0)
    }
}

/// Parse a classical bit reference like "c[0]"
fn parse_cbit_ref(lexer: &mut Lexer, creg_name: &str, n_cbits: usize) -> Result<usize, ParseError> {
    let name = match lexer.read_identifier() {
        Some(n) => n,
        None => return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected classical bit identifier")))),
    };

    if name != creg_name {
        return Err(perr(lexer, ParseErrorKind::UndefinedRegister(name)));
    }

    lexer.skip_whitespace_and_comments();
    if lexer.peek() == Some(b'[') {
        lexer.advance();
        let idx = match lexer.read_number() {
            Some(n) => n,
            None => return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected cbit index")))),
        };
        if !lexer.expect(b']') {
            return Err(perr(lexer, ParseErrorKind::InvalidSyntax(String::from("expected ']'"))));
        }
        if idx >= n_cbits {
            return Err(perr(lexer, ParseErrorKind::QubitOutOfRange(idx, n_cbits)));
        }
        Ok(idx)
    } else {
        if n_cbits == 0 {
            return Err(perr(lexer, ParseErrorKind::QubitOutOfRange(0, n_cbits)));
        }
        Ok(0)
    }
}

/// Main parse function for OpenQASM 2.0
pub fn parse_qasm2(input: &[u8]) -> Result<QasmProgram, ParseError> {
    let mut lexer = Lexer::new(input);
    
    // Parse header: OPENQASM 2.0;
    lexer.skip_whitespace_and_comments();
    let header = match lexer.read_identifier() {
        Some(h) => h,
        None => return Err(perr(&lexer, ParseErrorKind::MissingHeader)),
    };
    if header != "OPENQASM" {
        return Err(perr(&lexer, ParseErrorKind::MissingHeader));
    }
    
    // Skip version number
    lexer.skip_whitespace_and_comments();
    while lexer.pos < lexer.input.len() && lexer.input[lexer.pos] != b';' {
        lexer.advance();
    }
    lexer.expect(b';');

    let mut n_qubits = 0usize;
    let mut n_cbits = 0usize;
    let mut qreg_name = String::from("q");
    let mut creg_name = String::from("c");
    let mut instructions = Vec::new();

    // Parse declarations and instructions
    while !lexer.is_eof() {
        let Some(token) = lexer.read_identifier() else {
            lexer.skip_until_semicolon();
            continue;
        };

        match token.as_str() {
            "include" => {
                // Skip include statements
                lexer.skip_until_semicolon();
            }
            "qreg" => {
                // qreg q[n];
                let name = match lexer.read_identifier() {
                    Some(n) => n,
                    None => return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected qreg name")))),
                };
                qreg_name = name;
                if !lexer.expect(b'[') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected '['"))));
                }
                n_qubits = match lexer.read_number() {
                    Some(n) => n,
                    None => return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected qubit count")))),
                };
                if !lexer.expect(b']') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ']'"))));
                }
                lexer.expect(b';');
            }
            "creg" => {
                // creg c[n];
                let name = match lexer.read_identifier() {
                    Some(n) => n,
                    None => return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected creg name")))),
                };
                creg_name = name;
                if !lexer.expect(b'[') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected '['"))));
                }
                n_cbits = match lexer.read_number() {
                    Some(n) => n,
                    None => return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected cbit count")))),
                };
                if !lexer.expect(b']') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ']'"))));
                }
                lexer.expect(b';');
            }
            "h" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::H(q));
                lexer.expect(b';');
            }
            "x" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::X(q));
                lexer.expect(b';');
            }
            "y" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Y(q));
                lexer.expect(b';');
            }
            "z" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Z(q));
                lexer.expect(b';');
            }
            "s" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::S(q));
                lexer.expect(b';');
            }
            "t" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::T(q));
                lexer.expect(b';');
            }
            "rx" | "ry" | "rz" | "p" | "u1" => {
                // Parametric gate: rx(pi/2) q[0];
                if !lexer.expect(b'(') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected '(' after parametric gate"))));
                }
                let theta = parse_angle(&mut lexer)?;
                if !lexer.expect(b')') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ')'"))));
                }
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(match token.as_str() {
                    "rx" => Instruction::Rx(q, theta),
                    "ry" => Instruction::Ry(q, theta),
                    "rz" => Instruction::Rz(q, theta),
                    _ => Instruction::P(q, theta), // p / u1
                });
                lexer.expect(b';');
            }
            "cx" | "CX" | "cnot" | "CNOT" => {
                let ctrl = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                if !lexer.expect(b',') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ','"))));
                }
                let targ = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Cx(ctrl, targ));
                lexer.expect(b';');
            }
            "cz" | "CZ" => {
                let ctrl = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                if !lexer.expect(b',') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ','"))));
                }
                let targ = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Cz(ctrl, targ));
                lexer.expect(b';');
            }
            "swap" | "SWAP" => {
                let q1 = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                if !lexer.expect(b',') {
                    return Err(perr(&lexer, ParseErrorKind::InvalidSyntax(String::from("expected ','"))));
                }
                let q2 = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Swap(q1, q2));
                lexer.expect(b';');
            }
            "measure" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                // expect ->
                lexer.skip_whitespace_and_comments();
                if lexer.peek() == Some(b'-') {
                    lexer.advance();
                    if lexer.peek() == Some(b'>') {
                        lexer.advance();
                    }
                }
                let c = parse_cbit_ref(&mut lexer, &creg_name, n_cbits)?;
                instructions.push(Instruction::Measure(q, c));
                lexer.expect(b';');
            }
            "reset" => {
                let q = parse_qubit_ref(&mut lexer, &qreg_name, n_qubits)?;
                instructions.push(Instruction::Reset(q));
                lexer.expect(b';');
            }
            "barrier" => {
                // Skip barrier for now
                lexer.skip_until_semicolon();
            }
            _ => {
                // Unknown instruction, skip
                lexer.skip_until_semicolon();
            }
        }
    }

    Ok(QasmProgram {
        n_qubits,
        n_cbits,
        instructions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bell() {
        let qasm = b"OPENQASM 2.0;\nqreg q[2];\ncreg c[2];\nh q[0];\ncx q[0],q[1];\nmeasure q[0] -> c[0];\nmeasure q[1] -> c[1];\n";
        let prog = parse_qasm2(qasm).unwrap();
        assert_eq!(prog.n_qubits, 2);
        assert_eq!(prog.n_cbits, 2);
        assert_eq!(prog.instructions.len(), 4);
        assert_eq!(prog.instructions[0], Instruction::H(0));
        assert_eq!(prog.instructions[1], Instruction::Cx(0, 1));
    }
}
