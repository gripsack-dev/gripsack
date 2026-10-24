use super::*;

fn declaration(name: &str, deps: Vec<String>) -> WorkspaceOutput {
    serde_json::from_value(serde_json::json!({
        "kind":"task", "name":name, "span":{"file":"graph.ts","line":1},
        "context":{"kind":"host","mutable_paths":[]}, "deps":deps,
        "steps":[{"command":{"kind":"exec","span":{"file":"graph.ts","line":2},
            "argv":[{"kind":"literal","value":"/bin/true"}]}}]
    }))
    .unwrap()
}

#[test]
fn every_four_node_forward_graph_matches_independent_reachability_oracle() {
    // Every edge-deletion subset is enumerated, including all smaller failing
    // shapes a shrinking campaign would reach; no random corpus is needed.
    let possible = [(1, 0), (2, 0), (2, 1), (3, 0), (3, 1), (3, 2)];
    for mask in 0usize..(1 << possible.len()) {
        let mut adjacency = [[false; 4]; 4];
        for (index, &(owner, prerequisite)) in possible.iter().enumerate() {
            adjacency[owner][prerequisite] = mask & (1 << index) != 0;
        }
        let declarations: Vec<_> = (0..4)
            .map(|owner| {
                declaration(
                    &format!("n{owner}"),
                    (0..4)
                        .filter(|prerequisite| adjacency[owner][*prerequisite])
                        .map(|prerequisite| format!("n{prerequisite}"))
                        .collect(),
                )
            })
            .collect();
        let outputs: BTreeMap<_, _> = declarations
            .iter()
            .map(|output| (output.name(), output))
            .collect();
        let mut reachable = [false, false, false, true];
        for _ in 0..4 {
            for owner in 0..4 {
                for prerequisite in 0..4 {
                    reachable[prerequisite] |= reachable[owner] && adjacency[owner][prerequisite];
                }
            }
        }
        for _occasion in 0..2 {
            let mut nodes = Vec::new();
            visit(
                "n3",
                &outputs,
                &mut Vec::new(),
                &mut BTreeSet::new(),
                &mut nodes,
            )
            .unwrap();
            let names: BTreeSet<_> = nodes.iter().map(|node| node.name.as_str()).collect();
            assert_eq!(
                names.len(),
                nodes.len(),
                "shared prerequisite duplicated for graph {mask}"
            );
            assert_eq!(
                nodes.len(),
                reachable.iter().filter(|present| **present).count()
            );
            let positions: BTreeMap<_, _> = nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.name.as_str(), index))
                .collect();
            for (owner, present) in reachable.into_iter().enumerate() {
                assert_eq!(names.contains(format!("n{owner}").as_str()), present);
                if present {
                    for prerequisite in 0..4 {
                        if adjacency[owner][prerequisite] {
                            assert!(
                                positions[format!("n{prerequisite}").as_str()]
                                    < positions[format!("n{owner}").as_str()]
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn cycles_missing_edges_and_wrong_kind_are_not_executable_graphs() {
    let outputs = [
        declaration("a", vec!["b".into()]),
        declaration("b", vec!["a".into()]),
    ];
    let graph = outputs
        .iter()
        .map(|output| (output.name(), output))
        .collect();
    let error = visit(
        "a",
        &graph,
        &mut Vec::new(),
        &mut BTreeSet::new(),
        &mut Vec::new(),
    )
    .unwrap_err();
    let ExecError::Gate(diagnostic) = error else {
        panic!("cycle must be a source diagnostic")
    };
    assert_eq!(diagnostic.labels.len(), 2);
    let outputs = [declaration("a", vec!["absent".into()])];
    let graph = outputs
        .iter()
        .map(|output| (output.name(), output))
        .collect();
    assert!(
        visit(
            "a",
            &graph,
            &mut Vec::new(),
            &mut BTreeSet::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    let environment: WorkspaceOutput = serde_json::from_value(serde_json::json!({
        "kind":"environment","name":"not-task","span":{"file":"graph.ts","line":3},
        "packages":[],"target":{"os":"linux","arch":"x86_64"}
    }))
    .unwrap();
    let outputs = [declaration("a", vec!["not-task".into()]), environment];
    let graph = outputs
        .iter()
        .map(|output| (output.name(), output))
        .collect();
    assert!(
        visit(
            "a",
            &graph,
            &mut Vec::new(),
            &mut BTreeSet::new(),
            &mut Vec::new()
        )
        .is_err()
    );
}
