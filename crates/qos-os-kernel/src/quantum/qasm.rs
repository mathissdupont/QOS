//! QASM Parser for QOS
//!
//! Parses a subset of OpenQASM 2.0 and produces a Circuit.
//! Handles: qreg, creg, h, x, y, z, s, t, cx, cz, swap, measure, reset, barrier

use alloc::string::String;
use alloc::vec::Vec;

use super::circuit::{Circuit, Gate};

/// Parse error types
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    MissingHeader,
    InvalidSyntax(String),
    UndefinedRegister(String),
    QubitOutOfRange(usize, usize),
    UnsupportedGate(String),
    InvalidQubitIndex,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ParseError::MissingHeader => write!(f, "Missing OPENQASM header"),
            ParseError::InvalidSyntax(s) => write!(f, "Invalid syntax: {}", s),
            ParseError::UndefinedRegister(s) => write!(f, "Undefined register: {}", s),
            ParseError::QubitOutOfRange(q, max) => write!(f, "Qubit {} out of range (max {})", q, max),
            ParseError::UnsupportedGate(s) => write!(f, "Unsupported gate: {}", s),
            ParseError::InvalidQubitIndex => write!(f, "Invalid qubit index"),
        }
    }
}

/// Parse a QASM string into a Circuit
pub fn parse_qasm(source: &str) -> Result<Circuit, ParseError> {
    let bytes = source.as_bytes();
    let mut lexer = Lexer::new(bytes);
    
    // Parse header (optional but recommended)
    lexer.skip_whitespace_and_comments();
    
    // Look for OPENQASM header
    let mut has_header = false;
    if let Some(ident) = lexer.peek_identifier() {
        if ident == "OPENQASM" {
            lexer.read_identifier();
            lexer.skip_until_semicolon();
            has_header = true;
        }
    }
    
    // Skip include statements
    loop {
        lexer.skip_whitespace_and_comments();
        if let Some(ident) = lexer.peek_identifier() {
            if ident == "include" {
                lexer.read_identifier();
                lexer.skip_until_semicolon();
                continue;
            }
        }
        break;
    }
    
    // Parse register declarations and find qubit/cbit counts
    let mut n_qubits = 0usize;
    let mut n_cbits = 0usize;
    let mut gates: Vec<Gate> = Vec::new();
    let mut qreg_name: Option<String> = None;
    let mut creg_name: Option<String> = None;
    
    loop {
        lexer.skip_whitespace_and_comments();
        if lexer.is_eof() {
            break;
        }
        
        let Some(ident) = lexer.read_identifier() else {
            // Try to skip unknown token
            if lexer.peek() == Some(b';') {
                lexer.advance();
                continue;
            }
            lexer.advance();
            continue;
        };
        
        match ident.as_str() {
            "qreg" => {
                // qreg name[size];
                lexer.skip_whitespace_and_comments();
                let name = lexer.read_identifier().unwrap_or_default();
                qreg_name = Some(name);
                lexer.expect(b'[');
                let size = lexer.read_number().unwrap_or(1);
                lexer.expect(b']');
                lexer.expect(b';');
                n_qubits = size;
            }
            "creg" => {
                // creg name[size];
                lexer.skip_whitespace_and_comments();
                let name = lexer.read_identifier().unwrap_or_default();
                creg_name = Some(name);
                lexer.expect(b'[');
                let size = lexer.read_number().unwrap_or(1);
                lexer.expect(b']');
                lexer.expect(b';');
                n_cbits = size;
            }
            "h" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::H(q));
            }
            "x" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::X(q));
            }
            "y" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Y(q));
            }
            "z" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Z(q));
            }
            "s" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::S(q));
            }
            "t" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::T(q));
            }
            "cx" | "CX" | "cnot" | "CNOT" => {
                let (ctrl, targ) = parse_two_qubit_args(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Cx(ctrl, targ));
            }
            "cz" | "CZ" => {
                let (ctrl, targ) = parse_two_qubit_args(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Cz(ctrl, targ));
            }
            "swap" | "SWAP" => {
                let (q1, q2) = parse_two_qubit_args(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Swap(q1, q2));
            }
            "measure" => {
                // measure q[i] -> c[j];
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_whitespace_and_comments();
                // Skip "->"
                if lexer.peek() == Some(b'-') {
                    lexer.advance();
                    if lexer.peek() == Some(b'>') {
                        lexer.advance();
                    }
                }
                let c = parse_qubit_arg(&mut lexer, n_cbits.max(n_qubits)).unwrap_or(q);
                lexer.skip_until_semicolon();
                gates.push(Gate::Measure(q, c));
            }
            "reset" => {
                let q = parse_qubit_arg(&mut lexer, n_qubits)?;
                lexer.skip_until_semicolon();
                gates.push(Gate::Reset(q));
            }
            "barrier" => {
                // barrier q[0], q[1], ...;
                let mut qubits = Vec::new();
                loop {
                    if let Ok(q) = parse_qubit_arg(&mut lexer, n_qubits) {
                        qubits.push(q);
                    }
                    lexer.skip_whitespace_and_comments();
                    if lexer.peek() != Some(b',') {
                        break;
                    }
                    lexer.advance(); // skip comma
                }
                lexer.skip_until_semicolon();
                gates.push(Gate::Barrier(qubits));
            }
            "OPENQASM" => {
                // Skip any duplicate headers
                lexer.skip_until_semicolon();
            }
            "include" => {
                lexer.skip_until_semicolon();
            }
            _ => {
                // Unknown gate - skip
                lexer.skip_until_semicolon();
            }
        }
    }
    
    // Ensure we have at least 1 qubit
    if n_qubits == 0 {
        n_qubits = 1;
    }
    if n_cbits == 0 {
        n_cbits = n_qubits;
    }
    
    let mut circuit = Circuit::new(n_qubits, n_cbits);
    circuit.gates = gates;
    Ok(circuit)
}

/// Parse a single qubit argument like "q[0]" or just "q"
fn parse_qubit_arg(lexer: &mut Lexer, max_qubits: usize) -> Result<usize, ParseError> {
    lexer.skip_whitespace_and_comments();
    
    // Read register name (or skip if not present)
    let _ = lexer.read_identifier();
    
    lexer.skip_whitespace_and_comments();
    
    // Check for index
    if lexer.peek() == Some(b'[') {
        lexer.advance();
        let idx = lexer.read_number().ok_or(ParseError::InvalidQubitIndex)?;
        lexer.expect(b']');
        if idx >= max_qubits && max_qubits > 0 {
            return Err(ParseError::QubitOutOfRange(idx, max_qubits));
        }
        Ok(idx)
    } else {
        // No index, assume qubit 0
        Ok(0)
    }
}

/// Parse two qubit arguments like "q[0], q[1]"
fn parse_two_qubit_args(lexer: &mut Lexer, max_qubits: usize) -> Result<(usize, usize), ParseError> {
    let q1 = parse_qubit_arg(lexer, max_qubits)?;
    lexer.skip_whitespace_and_comments();
    lexer.expect(b',');
    let q2 = parse_qubit_arg(lexer, max_qubits)?;
    Ok((q1, q2))
}

/// Simple lexer for QASM parsing
struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
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

    fn peek_identifier(&mut self) -> Option<String> {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        let mut end = start;
        while end < self.input.len() {
            let ch = self.input[end];
            if ch.is_ascii_alphanumeric() || ch == b'_' {
                end += 1;
            } else {
                break;
            }
        }
        if end > start {
            let s = &self.input[start..end];
            Some(String::from_utf8_lossy(s).into_owned())
        } else {
            None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bell() {
        let qasm = r#"
            OPENQASM 2.0;
            include "qelib1.inc";
            qreg q[2];
            creg c[2];
            h q[0];
            cx q[0],q[1];
            measure q[0] -> c[0];
            measure q[1] -> c[1];
        "#;
        
        let circuit = parse_qasm(qasm).unwrap();
        assert_eq!(circuit.n_qubits, 2);
        assert_eq!(circuit.n_cbits, 2);
        assert_eq!(circuit.gates.len(), 4);
    }
}
