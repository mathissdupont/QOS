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
        Instruction::Crz(a, b, _) | Instruction::Cp(a, b, _) => (*a, Some(*b)),
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

/// Rotation axis key for merging: same-axis rotations on the same qubit compose by angle sum.
fn rot_key(inst: &Instruction) -> Option<(u8, usize, f64)> {
    match inst {
        Instruction::Rx(q, t) => Some((0, *q, *t)),
        Instruction::Ry(q, t) => Some((1, *q, *t)),
        Instruction::Rz(q, t) => Some((2, *q, *t)),
        Instruction::P(q, t) => Some((3, *q, *t)),
        _ => None,
    }
}

fn rot_make(axis: u8, q: usize, theta: f64) -> Instruction {
    match axis {
        0 => Instruction::Rx(q, theta),
        1 => Instruction::Ry(q, theta),
        2 => Instruction::Rz(q, theta),
        _ => Instruction::P(q, theta),
    }
}

/// Merge adjacent same-axis rotations on the same qubit (RZ(a)·RZ(b) → RZ(a+b)), dropping
/// rotations that sum to ~0 (mod 2π). Runs to a fixpoint. Returns `(optimized, merged_count)`.
pub fn merge_rotations(mut instrs: Vec<Instruction>) -> (Vec<Instruction>, usize) {
    const TAU: f64 = 2.0 * core::f64::consts::PI;
    let mut merged = 0;
    loop {
        let mut changed = false;
        let mut i = 0;
        'outer: while i < instrs.len() {
            if let Some((axis, q, t0)) = rot_key(&instrs[i]) {
                let mut j = i + 1;
                while j < instrs.len() {
                    if let Some((axis2, q2, t1)) = rot_key(&instrs[j]) {
                        if axis2 == axis && q2 == q {
                            // Compose the pair.
                            let mut sum = (t0 + t1) % TAU;
                            if sum > core::f64::consts::PI {
                                sum -= TAU;
                            }
                            instrs.remove(j);
                            if libm::fabs(sum) < 1e-12 {
                                instrs.remove(i); // net identity
                            } else {
                                instrs[i] = rot_make(axis, q, sum);
                            }
                            merged += 1;
                            changed = true;
                            continue 'outer;
                        }
                    }
                    if overlaps(&instrs[i], &instrs[j]) {
                        break;
                    }
                    j += 1;
                }
            }
            i += 1;
        }
        if !changed {
            break;
        }
    }
    (instrs, merged)
}

/// The standard optimization pipeline: pair cancellation + rotation merging, to a joint
/// fixpoint. Returns `(optimized, cancelled, merged)`.
pub fn optimize(instrs: Vec<Instruction>) -> (Vec<Instruction>, usize, usize) {
    let mut cur = instrs;
    let (mut cancelled, mut merged) = (0usize, 0usize);
    loop {
        let (a, c) = cancel_pairs(cur);
        let (b, m) = merge_rotations(a);
        cancelled += c;
        merged += m;
        cur = b;
        if c == 0 && m == 0 {
            break;
        }
    }
    (cur, cancelled, merged)
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
