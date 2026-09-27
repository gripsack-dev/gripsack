//! Generation identities and strictly ordered inventories. Serialization and
//! filesystem enumeration stay in the store; policy receives admitted values.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct GenerationId {
    number: u64,
}

impl GenerationId {
    pub closed spec fn view(self) -> u64 { self.number }

    /// Every wire u64 is an identity, including retained historical zero IDs.
    pub fn new(number: u64) -> (id: Self)
        ensures id.view() == number,
    {
        Self { number }
    }

    /// Numeric representation for wire encoding and inventory ordering only.
    pub fn value(self) -> (number: u64)
        ensures number == self.view(),
    {
        self.number
    }

    pub fn checked_next(self) -> (next: Option<Self>)
        ensures
            next.is_some() <==> self.view() < u64::MAX,
            next.is_some() ==> next.unwrap().view() == self.view() + 1,
    {
        if self.number == u64::MAX {
            None
        } else {
            Some(Self { number: self.number + 1 })
        }
    }
    pub fn checked_previous(self) -> (previous: Option<Self>)
        ensures
            previous.is_some() <==> self.view() > 0,
            previous.is_some() ==> previous.unwrap().view() + 1 == self.view(),
    {
        if self.number == 0 { None } else { Some(Self { number: self.number - 1 }) }
    }
}

pub open spec fn strictly_ascending(ids: Seq<GenerationId>) -> bool {
    forall|i: int| 0 < i < ids.len() ==> ids[i - 1].view() < #[trigger] ids[i].view()
}

/// Borrowed, strictly ascending and duplicate-free generation identities.
/// Raw slices cannot enter retention policy without admission.
///
/// ```compile_fail
/// use gripsack_policy::retention::plan_prune;
/// plan_prune(&[1, 2, 3], None, Some(1));
/// ```
#[derive(Debug, Clone, Copy)]
pub struct GenerationInventory<'a> {
    ids: &'a [GenerationId],
}

impl<'a> GenerationInventory<'a> {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool { strictly_ascending(self.ids@) }

    pub closed spec fn view(self) -> Seq<GenerationId> { self.ids@ }

    pub fn new(ids: &'a [GenerationId]) -> (result: Option<Self>)
        ensures
            result.is_some() <==> strictly_ascending(ids@),
            result.is_some() ==> result.unwrap().view() == ids@,
    {
        let mut i = 1;
        while i < ids.len()
            invariant
                1 <= i,
                forall|j: int| 0 < j < i && j < ids.len() ==>
                    ids@[j - 1].view() < #[trigger] ids@[j].view(),
            decreases ids.len() - i,
        {
            if ids[i - 1].number >= ids[i].number {
                proof {
                    assert(ids@[i as int - 1].view() >= ids@[i as int].view());
                    assert(!strictly_ascending(ids@));
                }
                return None;
            }
            i += 1;
        }
        Some(Self { ids })
    }

    pub fn as_slice(&self) -> (ids: &'a [GenerationId])
        ensures ids@ == self.view(), strictly_ascending(ids@),
    {
        proof { use_type_invariant(self); }
        self.ids
    }
}

/// Owned inventory admission happens once; subsequent policy calls borrow it.
#[derive(Debug)]
pub struct GenerationList {
    ids: Vec<GenerationId>,
}

impl GenerationList {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool { strictly_ascending(self.ids@) }

    pub closed spec fn view(&self) -> Seq<GenerationId> { self.ids@ }

    pub fn new(ids: Vec<GenerationId>) -> (result: Option<Self>)
        ensures
            result.is_some() <==> strictly_ascending(ids@),
            result.is_some() ==> result.unwrap().view() == ids@,
    {
        if GenerationInventory::new(&ids).is_some() {
            Some(Self { ids })
        } else {
            None
        }
    }

    pub fn inventory(&self) -> (inventory: GenerationInventory<'_>)
        ensures inventory.view() == self.view(), strictly_ascending(inventory.view()),
    {
        proof { use_type_invariant(self); }
        GenerationInventory { ids: &self.ids }
    }
}

}

impl std::fmt::Display for GenerationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.number.fmt(f)
    }
}

impl std::str::FromStr for GenerationId {
    type Err = std::num::ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self::new)
    }
}

impl Ord for GenerationId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.number.cmp(&other.number)
    }
}

impl PartialOrd for GenerationId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::ops::Deref for GenerationList {
    type Target = [GenerationId];

    fn deref(&self) -> &Self::Target {
        &self.ids
    }
}

impl IntoIterator for GenerationList {
    type Item = GenerationId;
    type IntoIter = std::vec::IntoIter<GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.ids.into_iter()
    }
}

impl<'a> IntoIterator for &'a GenerationList {
    type Item = &'a GenerationId;
    type IntoIter = std::slice::Iter<'a, GenerationId>;

    fn into_iter(self) -> Self::IntoIter {
        self.ids.iter()
    }
}

impl std::hash::Hash for GenerationId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.number.hash(state);
    }
}
