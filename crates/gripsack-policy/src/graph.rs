//! Workspace reference roles and catalog-name binding, used by the IR's
//! workspace admission (gripsack-ir/src/sema/workspace_model/graph.rs).

/// What a declared reference means for the output that declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRole {
    Production,
    BuildInput,
    Runtime,
    Ordering,
    TaskPrereq,
    Validation,
    Retention,
}

impl GraphRole {
    /// Whether the reference orders its owner after its target: these edges
    /// form the dependency graph that admission refuses to see cyclic.
    pub fn is_dependency(self) -> bool {
        match self {
            Self::Production | Self::BuildInput | Self::Runtime | Self::TaskPrereq => true,
            Self::Ordering | Self::Validation | Self::Retention => false,
        }
    }
}

/// A catalog position checked to name exactly the referenced output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundOutputIndex {
    position: usize,
}

impl BoundOutputIndex {
    pub fn position(self) -> usize {
        self.position
    }
}

/// Bind a candidate position (from a name map) to the catalog only if it is
/// in range and names exactly `declared`. Missing, out-of-range and
/// wrong-name candidates are refused.
pub fn bind_output_index(
    names: &[&str],
    declared: &str,
    candidate: Option<usize>,
) -> Option<BoundOutputIndex> {
    let position = candidate?;
    (names.get(position) == Some(&declared)).then_some(BoundOutputIndex { position })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_indices_cannot_bind_another_name_or_escape_the_catalog() {
        let names = ["build", "hello"];
        let build = bind_output_index(&names, "build", Some(0)).expect("exact catalog entry");
        let hello = bind_output_index(&names, "hello", Some(1)).expect("exact catalog entry");
        assert_ne!(build.position(), hello.position());
        for candidate in [Some(1), Some(2), None] {
            assert!(bind_output_index(&names, "build", candidate).is_none());
        }
        assert!(bind_output_index(&names, "hello", Some(0)).is_none());
        assert!(bind_output_index(&names, "missing", Some(0)).is_none());
    }

    #[test]
    fn only_ordering_validation_and_retention_are_not_dependencies() {
        use GraphRole::*;
        for (role, dependency) in [
            (Production, true),
            (BuildInput, true),
            (Runtime, true),
            (TaskPrereq, true),
            (Ordering, false),
            (Validation, false),
            (Retention, false),
        ] {
            assert_eq!(role.is_dependency(), dependency, "{role:?}");
        }
    }
}
