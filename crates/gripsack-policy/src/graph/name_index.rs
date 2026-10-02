//! Bind a catalog name to an index before that index enters a verified
//! graph kernel. The caller may obtain a candidate from an unverified map;
//! an in-range position with a different name never becomes authority.
//! Schema decoding and catalog construction are separate obligations.

use vstd::prelude::*;

verus! {

/// A position known to name exactly the referenced output in this
/// catalog. Callers may pass the position to the existing index-based
/// closure kernel only after binding it against the same name slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundOutputIndex {
    position: usize,
}

impl BoundOutputIndex {
    /// Expose only the bound value in contracts, not a forgeable
    /// constructor or public field.
    pub closed spec fn verified_position(self) -> usize {
        self.position
    }

    /// The index may be handed to index-based graph kernels only after
    /// `bind_output_index` has checked its source name.
    pub fn position(self) -> (index: usize)
        ensures index == self.verified_position(),
    {
        self.position
    }
}

pub open spec fn exact_name_at(
    names: Seq<Seq<char>>,
    declared: Seq<char>,
    candidate: Option<usize>,
) -> bool {
    match candidate {
        Some(position) => position < names.len() && names[position as int] == declared,
        None => false,
    }
}

/// Total over missing, out-of-range and wrong-name candidates. This
/// checks the actual borrowed strings; it does not hash, copy or scan
/// the catalog, and adds no successful-path heap allocation.
pub fn bind_output_index(
    names: &[&str],
    declared: &str,
    candidate: Option<usize>,
) -> (result: Option<BoundOutputIndex>)
    ensures
        result.is_some() <==> exact_name_at(
            names@.map(|_i, name: &str| name@), declared@, candidate,
        ),
        match result {
            Some(bound) => candidate == Some(bound.verified_position())
                && (bound.verified_position() as int) < names@.len()
                && names@[bound.verified_position() as int]@ == declared@,
            None => true,
        },
{
    let position = candidate?;
    if position >= names.len() {
        return None;
    }
    let same_name = names[position] == declared;
    proof {
        assert(same_name == (names@[position as int]@ == declared@));
    }
    if same_name {
        Some(BoundOutputIndex { position })
    } else {
        None
    }
}

/// Pointwise bindings cannot alias two distinct output names. The
/// policy adapter must still show that it bound *every* decoded edge.
pub proof fn distinct_names_have_distinct_indices(
    names: Seq<Seq<char>>,
    first: Seq<char>,
    first_index: usize,
    second: Seq<char>,
    second_index: usize,
)
    requires
        first != second,
        first_index < names.len(),
        second_index < names.len(),
        names[first_index as int] == first,
        names[second_index as int] == second,
    ensures first_index != second_index,
{
}

}
