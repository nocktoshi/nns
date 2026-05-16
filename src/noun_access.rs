//! Space-scoped noun accessors for nockvm's `NounHandle` / `AtomHandle` API.

use nockapp::noun::slab::NounSlab;
use nockvm::mem::NockStack;
use nockvm::noun::{Noun, NounAllocator, NounSpace};

#[derive(Clone, Copy)]
enum AllocRef<'a> {
    Slab(&'a NounSlab),
    Stack(&'a NockStack),
}

/// A noun paired with the allocator it was allocated in (for [`NounSpace::noun_space`]).
#[derive(Clone, Copy)]
pub struct ScopedNoun<'a> {
    pub noun: Noun,
    alloc: AllocRef<'a>,
}

impl<'a> ScopedNoun<'a> {
    pub fn in_slab(slab: &'a NounSlab, noun: Noun) -> Self {
        Self {
            noun,
            alloc: AllocRef::Slab(slab),
        }
    }

    pub fn from_slab(slab: &'a NounSlab) -> Self {
        Self::in_slab(slab, unsafe { *slab.root() })
    }

    pub fn from_stack(stack: &'a NockStack, noun: Noun) -> Self {
        Self {
            noun,
            alloc: AllocRef::Stack(stack),
        }
    }

    #[inline]
    fn noun_space(&self) -> NounSpace {
        match self.alloc {
            AllocRef::Slab(s) => s.noun_space(),
            AllocRef::Stack(st) => st.noun_space(),
        }
    }

    pub fn rebind(&self, noun: Noun) -> Self {
        Self {
            noun,
            alloc: self.alloc,
        }
    }

    pub fn is_atom(&self) -> bool {
        self.noun.is_atom()
    }

    pub fn head(&self) -> Result<Self, String> {
        let space = self.noun_space();
        Ok(self.rebind(
            self.noun
                .in_space(&space)
                .as_cell()
                .map_err(|_| "expected cell".to_string())?
                .head()
                .noun(),
        ))
    }

    pub fn tail(&self) -> Result<Self, String> {
        let space = self.noun_space();
        Ok(self.rebind(
            self.noun
                .in_space(&space)
                .as_cell()
                .map_err(|_| "expected cell".to_string())?
                .tail()
                .noun(),
        ))
    }

    pub fn uncons(&self) -> Result<(Self, Self), String> {
        let space = self.noun_space();
        let c = self
            .noun
            .in_space(&space)
            .as_cell()
            .map_err(|_| "expected cell".to_string())?;
        Ok((
            self.rebind(c.head().noun()),
            self.rebind(c.tail().noun()),
        ))
    }

    pub fn as_u64(&self) -> Result<u64, String> {
        let space = self.noun_space();
        self.noun
            .in_space(&space)
            .as_atom()
            .map_err(|_| "expected atom".to_string())?
            .as_u64()
            .map_err(|_| "atom overflows u64".to_string())
    }

    pub fn as_u64_opt(&self) -> Option<u64> {
        self.as_u64().ok()
    }

    pub fn as_ne_bytes(&self) -> Result<Vec<u8>, String> {
        let space = self.noun_space();
        Ok(self
            .noun
            .in_space(&space)
            .as_atom()
            .map_err(|_| "expected atom".to_string())?
            .as_ne_bytes()
            .to_vec())
    }

    pub fn as_cord(&self) -> Result<String, String> {
        let space = self.noun_space();
        let atom = self
            .noun
            .in_space(&space)
            .as_atom()
            .map_err(|_| "expected cord atom".to_string())?;
        let bytes = atom.as_ne_bytes();
        std::str::from_utf8(bytes)
            .map_err(|_| "cord not utf-8".to_string())
            .map(|s| s.trim_end_matches('\0').to_string())
    }

    pub fn list_elements(&self) -> Result<Vec<ScopedNoun<'a>>, String> {
        let mut out = Vec::new();
        let mut cur = self.noun;
        loop {
            if cur.is_atom() {
                break;
            }
            let space = self.noun_space();
            let cell = cur
                .in_space(&space)
                .as_cell()
                .map_err(|_| "malformed list".to_string())?;
            out.push(self.rebind(cell.head().noun()));
            cur = cell.tail().noun();
        }
        Ok(out)
    }
}

/// Deep-copy `noun` from `src` into a fresh [`NounSlab`].
///
/// Required before `in_space` / jam on nouns returned from kernel peeks or
/// effects when the source slab is PMA-backed and child pointers may not
/// resolve under `src.noun_space()`.
pub fn copy_noun_to_slab(src: &NounSlab, noun: Noun) -> NounSlab {
    let mut dst = NounSlab::new();
    let space = src.noun_space();
    let copied = dst.copy_into(noun, &space);
    dst.set_root(copied);
    dst
}

/// LE bytes of an atom noun copied out of `src`.
pub fn atom_bytes_from_slab(src: &NounSlab, noun: Noun) -> Result<Vec<u8>, String> {
    let tmp = copy_noun_to_slab(src, noun);
    ScopedNoun::from_slab(&tmp).as_ne_bytes()
}

/// JAM bytes of any noun copied out of `src`.
pub fn jam_bytes_from_slab(src: &NounSlab, noun: Noun) -> Vec<u8> {
    copy_noun_to_slab(src, noun).jam().to_vec()
}
