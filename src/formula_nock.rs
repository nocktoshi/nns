//! Y3 recursive STARK helpers: inspect hand-built Nock formulas for opcode
//! patterns that Vesl's `prove-computation:vp` cannot trace yet (`%9`–`%11`).

use nockvm::mem::NockStack;
use nockvm::noun::{Noun, NounHandle, NounSpace};

/// Depth-first search: `true` if any subtree is a cell whose head is the atom
/// `9`, `10`, or `11` (Nock `%slam`, `%edit`, `%hint`).
///
/// Constants like `[1 9]` are fine: the opcode scan only treats a **cell head**
/// as an opcode. Data nouns whose head happens to be a small atom are
/// extremely unlikely in our 0–8 hand-built trees.
pub fn formula_contains_banned_nock_opcodes(stack: &mut NockStack, formula: Noun) -> bool {
    let space = stack.noun_space();
    walk(&space, formula)
}

fn walk(space: &NounSpace, n: Noun) -> bool {
    if n.is_atom() {
        return false;
    }
    let root = NounHandle::new(n, space);
    let Ok(cell) = root.as_cell() else {
        return false;
    };
    let head_n = cell.head().noun();
    if let Ok(ah) = NounHandle::new(head_n, space).as_atom() {
        if let Ok(v) = ah.as_u64() {
            if (9..=11).contains(&v) {
                return true;
            }
        }
    }
    walk(space, cell.head().noun()) || walk(space, cell.tail().noun())
}
