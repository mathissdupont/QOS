//! Transpile passes for QOS's quantum toolchain (WP-06 / MASTERPLAN E-80).
//!
//! First real pass: **self-inverse gate cancellation** — adjacent identical H/X/Y/Z/CX/CZ/SWAP
//! pairs acting on the same qubit(s) with nothing touching those qubits in between compose to the
//! identity and are removed. Plus **circuit-depth** analysis (the length of the longest
//! qubit-timeline), the metric hardware queues care about.

use alloc::vec::Vec;

use super::parser::Instruction;

/// Which qubits an instruction touches (measure/reset/barrier block optimization across them).
fn touched(inst: &Instruction) -> (usize, Option<usize>) {
    match inst {
        Instruction::H(q)
        | Instruction::X(q)
        | Instruction::Y(q)
        | Instruction::Z(q)
        | Instruction::S(q)
        | Instruction::T(q)
        | Instruction::Rx(q, _)
        | Instruction::Ry(q, _)
        | Instruction::Rz(q, _)
        | Instruction::P(q, _)
        | Instruction::Reset(q)
        | Instruction::Measure(q, _) => (*q, None),
        Instruction::Cx(a, b) | Instruction::Cz(a, b) | Instruction::Swap(a, b) => (*a, Some(*b)),
        Instruction::Barrier(_) => (usize::MAX, None), // treated as touching everything
    }
}

/// True for gates that are their own inverse (U·U = I) — candidates for pair cancellation.
fn self_inverse(inst: &Instruction) -> bool {
    matches!(
        inst,
        Instruction::H(_)
            | Instruction::X(_)
            | Instruction::Y(_)
            | Instruction::Z(_)
            | Instruction::Cx(_, _)
            | Instruction::Cz(_, _)
            | Instruction::Swap(_, _)
    )
}

/// Do two instructions share any qubit?
fn overlaps(a: &Instruction, b: &Instruction) -> bool {
    let (a1, a2) = touched(a);
    let (b1, b2) = touched(b);
    if a1 == usize::MAX || b1 == usize::MAX {
        return true; // barrier overlaps everything
    }
    a1 == b1 || Some(a1) == b2 || a2 == Some(b1) || (a2.is_some() && a2 == b2)
}

/// Cancel adjacent self-inverse pairs (identical instruction with no overlapping op in between).
/// Runs to a fixpoint. Returns `(optimized, removed_count)`.
pub fn cancel_pairs(mut instrs: Vec<Instruction>) -> (Vec<Instruction>, usize) {
    let mut removed = 0;
    loop {
        let mut cancelled_this_round = false;
        let mut i = 0;
        'outer: while i < instrs.len() {
            if self_inverse(&instrs[i]) {
                // Look ahead for the identical gate, allowing non-overlapping ops in between.
                let mut j = i + 1;
                while j < instrs.len() {
                    if instrs[j] == instrs[i] {
                        instrs.remove(j);
                        instrs.remove(i);
                        removed += 2;
                        cancelled_this_round = true;
                        continue 'outer; // re-check from the same position
                    }
                    if overlaps(&instrs[i], &instrs[j]) {
                        break; // something touches our qubits first — no cancellation
                    }
                    j += 1;
                }
            }
            i += 1;
        }
        if !cancelled_this_round {
            break;
        }
    }
    (instrs, removed)
}

/// Circuit depth: the longest per-qubit timeline, counting each gate as one time step on every
/// qubit it touches (barriers synchronize all qubits).
pub fn depth(instrs: &[Instruction], n_qubits: usize) -> usize {
    let mut level = alloc::vec![0usize; n_qubits.max(1)];
    for inst in instrs {
        let (a, b) = touched(inst);
        if a == usize::MAX {
            // Barrier: align every wire to the current max.
            let m = *level.iter().max().unwrap_or(&0);
            for l in level.iter_mut() {
                *l = m;
            }
            continue;
        }
        let mut t = level.get(a).copied().unwrap_or(0);
        if let Some(b) = b {
            t = t.max(level.get(b).copied().unwrap_or(0));
        }
        t += 1;
        if let Some(l) = level.get_mut(a) {
            *l = t;
        }
        if let Some(b) = b {
            if let Some(l) = level.get_mut(b) {
                *l = t;
            }
        }
    }
    *level.iter().max().unwrap_or(&0)
}
