//! Graph closures as a verified kernel (0047): the build-dependency
//! closure every consumer pins rides on, computed over indices. The
//! IR adapter (gripsack-ir/src/dependencies.rs) hands in a
//! name-indexed view; ordering-only edges are NOT in the input, so
//! they can never leak into a build closure — unrepresentable, not
//! just unused.
//!
//! Proved: the closure contains exactly the build-reachable non-root
//! nodes (soundness via ghost witness paths, completeness via the
//! worklist edge-closure invariant), and the walk terminates on cyclic
//! input. Unknown dep names are sink nodes by adapter construction —
//! they join the closure but have no out-edges, the pre-0047
//! semantics.

use vstd::prelude::*;
// plain-cargo shim builds see no use of the seq/map lemmas; the
// `broadcast use` below (erased outside verification) needs them
#[allow(unused_imports)]
use vstd::{map_lib::*, seq_lib::*, set_lib::*};

verus! {

broadcast use {group_seq_properties, group_map_properties, group_set_properties};

/// The spec-level edge view: target indices as ints.
pub open spec fn edge_view(edges: Seq<Vec<usize>>) -> Seq<Seq<int>> {
    edges.map(|_i, targets: Vec<usize>| targets@.map(|_j, t: usize| t as int))
}

/// A path: nonempty, endpoints as given, every step an edge.
pub open spec fn is_path(edges: Seq<Seq<int>>, path: Seq<int>, from: int, to: int) -> bool {
    &&& path.len() >= 1
    &&& path[0] == from
    &&& path[path.len() - 1] == to
    &&& (forall|i: int| 0 <= i < path.len() ==> 0 <= #[trigger] path[i] < edges.len())
    &&& (forall|i: int| 0 <= i < path.len() - 1 ==>
            (#[trigger] edges[path[i]]).contains(path[i + 1]))
}

/// Reachability through build edges (existence of a path — any
/// length; no fuel, no bound).
pub open spec fn reachable(edges: Seq<Seq<int>>, from: int, to: int) -> bool {
    exists|path: Seq<int>| is_path(edges, path, from, to)
}

/// The build-dependency closure of `root`: every node reachable
/// through build edges, root excluded (a cycle back to the root is
/// not a dependency of itself — the pre-0047 `closure.remove(name)`
/// semantics, kept).
pub fn build_closure(n_nodes: usize, edges: &[Vec<usize>], root: usize) -> (result: Vec<usize>)
    requires
        edges@.len() == n_nodes,
        root < n_nodes,
        // edges stay in range (the adapter builds them from the name
        // table; this is the admission contract)
        forall|i: int, idx: int| #![trigger edges@[i]@[idx]]
            0 <= i < n_nodes && 0 <= idx < edges@[i]@.len()
                ==> (edges@[i]@[idx] as int) < n_nodes,
    ensures
        // soundness: every member is a reachable non-root node
        forall|j: usize| result@.contains(j) ==>
            j != root && reachable(edge_view(edges@), root as int, j as int),
        // completeness: every reachable non-root node is a member
        forall|j: usize| 0 <= j < n_nodes && j != root
            && reachable(edge_view(edges@), root as int, j as int)
                ==> result@.contains(j),
        // uniqueness (the shipped BTreeSet semantics)
        forall|a: int, b: int| 0 <= a < result@.len() && 0 <= b < result@.len()
            && result@[a] == result@[b] ==> a == b,
{
    let mut visited: Vec<bool> = Vec::new();
    let mut processed: Vec<bool> = Vec::new();
    let mut init = 0;
    while init < n_nodes
        invariant
            init <= n_nodes,
            visited@.len() == init as int,
            processed@.len() == init as int,
            forall|j: int| 0 <= j < init as int ==> !visited@[j] && !processed@[j],
        decreases n_nodes - init,
    {
        visited.push(false);
        processed.push(false);
        init += 1;
    }
    visited[root] = true;
    proof {
        // after marking the root: visited ⟺ "is the root"
        assert forall|j: int| 0 <= j < n_nodes implies visited@[j] == (j == root as int) by {
        };
    }
    let mut result: Vec<usize> = Vec::new();
    let mut pending: Vec<usize> = vec![root];
    // the processed set as a ghost Set (insert-length lemmas are
    // standard; the filter-count plumbing was not)
    let ghost mut pset: Set<int> = Set::empty();
    // witness paths for soundness (ghost: no runtime cost)
    let ghost mut paths: Map<int, Seq<int>> = Map::empty();
    proof {
        paths = paths.insert(root as int, seq![root as int]);
        assert(is_path(edge_view(edges@), seq![root as int], root as int, root as int));
    }
    // the worklist loop: while-let would hide the exit fact
    // (pending empty) from the solver — the pending@ measure needs it
    #[allow(clippy::manual_while_let_some)]
    while !pending.is_empty()
        invariant
            n_nodes == edges@.len(),
            root < n_nodes,
            forall|i: int, idx: int| #![trigger edges@[i]@[idx]]
                0 <= i < n_nodes && 0 <= idx < edges@[i]@.len()
                    ==> (edges@[i]@[idx] as int) < n_nodes,
            visited@.len() == n_nodes,
            processed@.len() == n_nodes,
            // pending entries are visited, in range
            forall|k: int| 0 <= k < pending@.len() ==>
                (#[trigger] pending@[k]) < n_nodes && visited@[pending@[k] as int],
            // the witness map's domain is exactly the visited set, and
            // every key maps to a valid path from the root (soundness)
            forall|j: int| #![trigger paths.dom().contains(j)]
                paths.dom().contains(j) ==>
                (0 <= j < n_nodes && visited@[j]
                && is_path(edge_view(edges@), paths[j], root as int, j)),
            forall|j: int| 0 <= j < n_nodes && visited@[j] ==>
                paths.dom().contains(j),
            // processed nodes' edges all land in visited (completeness source)
            forall|a: int, b: int| #![trigger edge_view(edges@)[a].contains(b)]
                0 <= a < n_nodes && processed@[a]
                && edge_view(edges@)[a].contains(b) ==> visited@[b],
            // visited ⊆ processed ∪ pending
            forall|j: int| 0 <= j < n_nodes && visited@[j] ==>
                processed@[j] || pending@.contains(j as usize),
            // result == visited minus root, unique
            forall|j: usize| result@.contains(j) <==>
                (j as int) < n_nodes && visited@[j as int] && j != root,
            forall|a: int, b: int| 0 <= a < result@.len() && 0 <= b < result@.len()
                && result@[a] == result@[b] ==> a == b,
            forall|j: int| #![trigger pset.contains(j)]
                pset.contains(j) <==> (0 <= j < n_nodes && processed@[j]),
            pset.subset_of(set_int_range(0, n_nodes as int)),
            pset.len() <= n_nodes as int,
            visited@[root as int],
            forall|j: int| 0 <= j < n_nodes ==> (processed@[j] ==> visited@[j]),
        decreases n_nodes - pset.len(), pending@.len(),
    {
        let node = pending.pop().unwrap();
        proof {
            // node was a pending entry — the loop-head invariant made
            // every pending entry visited
            assert(visited@[node as int]);
        }
        if processed[node] {
            continue;
        }
        proof {
            // the flip grows the processed set by one; a strict subset
            // of the node range has strictly smaller length
            vstd::set_lib::lemma_len_subset(pset, set_int_range(0, n_nodes as int));
            pset.lemma_subset_not_in_lt(set_int_range(0, n_nodes as int), node as int);
            pset = pset.insert(node as int);
        }
        processed[node] = true;
        let mut k = 0;
        while k < edges[node].len()
            invariant
                n_nodes == edges@.len(),
                root < n_nodes,
                forall|i: int, idx: int| #![trigger edges@[i]@[idx]]
                    0 <= i < n_nodes && 0 <= idx < edges@[i]@.len()
                        ==> (edges@[i]@[idx] as int) < n_nodes,
                visited@.len() == n_nodes,
                processed@.len() == n_nodes,
                k <= edges@[node as int].len(),
                node < n_nodes,
                processed@[node as int],
                visited@[node as int],
                visited@[root as int],
                forall|k2: int| 0 <= k2 < pending@.len() ==>
                    (#[trigger] pending@[k2]) < n_nodes && visited@[pending@[k2] as int],
                forall|j: int| #![trigger paths.dom().contains(j)]
                    paths.dom().contains(j) ==>
                    (0 <= j < n_nodes && visited@[j]
                    && is_path(edge_view(edges@), paths[j], root as int, j)),
                forall|j: int| 0 <= j < n_nodes && visited@[j] ==>
                    paths.dom().contains(j),
                forall|a: int, b: int| #![trigger edge_view(edges@)[a].contains(b)]
                    0 <= a < n_nodes && processed@[a]
                    && a != node
                    && edge_view(edges@)[a].contains(b) ==> visited@[b],
                forall|j: int| 0 <= j < n_nodes && visited@[j] ==>
                    processed@[j] || pending@.contains(j as usize) || j == node,
                // this node's first k edges land in visited (pure
                // index arithmetic — no contains/take reasoning)
                forall|idx: int| 0 <= idx < k as int ==>
                    visited@[edges@[node as int]@[idx] as int],
                forall|j: usize| result@.contains(j) <==>
                    (j as int) < n_nodes && visited@[j as int] && j != root,
                forall|a: int, b: int| 0 <= a < result@.len() && 0 <= b < result@.len()
                    && result@[a] == result@[b] ==> a == b,
                pset.len() <= n_nodes as int,
                forall|j: int| 0 <= j < n_nodes && processed@[j] ==> visited@[j],
            decreases edges@[node as int].len() - k,
        {
            let target = edges[node][k];
            proof {
                // the exec read IS the spec edge, at index k
                assert(edges@[node as int]@[k as int] == target);
                assert((edge_view(edges@)[node as int])[k as int] == target as int);
                assert((edge_view(edges@)[node as int]).contains(target as int)) by {
                    assert(0 <= k && (k as int) < (edge_view(edges@)[node as int]).len());
                };
            }
            if !visited[target] {
                visited[target] = true;
                pending.push(target);
                proof {
                    // extend the witness path: node's path + this edge
                    let extended = paths[node as int].push(target as int);
                    assert(node != target);
                    paths = paths.insert(target as int, extended);
                    // every key's witness: unchanged ones by the
                    // insert's pointwise rule, the new one by
                    // construction
                    assert forall|j: int| #![trigger paths.dom().contains(j)]
                        paths.dom().contains(j) implies
                        (0 <= j < n_nodes && visited@[j]
                        && is_path(edge_view(edges@), paths[j], root as int, j))
                    by {
                        if j == target as int {
                            let witness = paths[node as int].push(target as int);
                            assert(paths[j] == witness);
                            assert(witness[0] == root as int);
                            assert(witness[witness.len() - 1] == j);
                            assert forall|i: int| 0 <= i < witness.len() - 1 implies
                                (#[trigger] edge_view(edges@)[witness[i]]).contains(
                                    witness[i + 1],
                                )
                            by {
                                if i < witness.len() - 2 {
                                    assert(witness[i] == paths[node as int][i]);
                                    assert(witness[i + 1] == paths[node as int][i + 1]);
                                } else {
                                    assert(witness[i] == node as int);
                                    assert(witness[i + 1] == target as int);
                                }
                            };
                        }
                    };
                }
                if target != root {
                    result.push(target);
                }
            }
            proof {
                // every edge up to and including k lands in visited:
                // earlier ones by the head invariant, this one by the body
                assert forall|idx: int| 0 <= idx <= k as int implies
                    visited@[edges@[node as int]@[idx] as int]
                by {
                    if idx == k as int {
                        assert(edges@[node as int]@[k as int] == target);
                    }
                };
            }
            k += 1;
        }
        proof {
            // every target of this (now processed) node is visited —
            // the outer invariant, from the inner loop's index form
            assert forall|b: int| (edge_view(edges@)[node as int]).contains(b)
                implies visited@[b]
            by {
                let idx = choose|idx: int| 0 <= idx < (edge_view(edges@)[node as int]).len()
                    && (edge_view(edges@)[node as int])[idx] == b;
                assert((edge_view(edges@)[node as int])[idx] == edges@[node as int]@[idx] as int);
                assert(visited@[edges@[node as int]@[idx] as int]);
            };
        }
    }
    proof {
        // soundness: a result member is visited (invariant) and every
        // visited node has a witness path (invariant) — present it
        assert forall|j: usize| result@.contains(j) implies
            j != root && reachable(edge_view(edges@), root as int, j as int)
        by {
            assert(visited@[j as int]);
            assert(paths.dom().contains(j as int));
            assert(is_path(edge_view(edges@), paths[j as int], root as int, j as int));
        };
        // completeness: a reachable node's path is a visited node by
        // induction over the path (lemma), and visited == processed at
        // exit (pending is empty), which lands it in result
        assert(pending@.len() == 0);
        assert forall|j: int| 0 <= j < n_nodes && visited@[j] implies processed@[j] by {
            assert(!pending@.contains(j as usize));
        };
        assert forall|j: usize| 0 <= j < n_nodes && j != root
            && reachable(edge_view(edges@), root as int, j as int) implies result@.contains(j)
        by {
            let path = choose|path: Seq<int>|
                is_path(edge_view(edges@), path, root as int, j as int);
            lemma_reachable_visited(edge_view(edges@), path, root as int, visited@, processed@);
        };
    }
    result
}


/// Reachable nodes are visited once the worklist drains: induction
/// over the path, using edge-closure (processed ⟹ targets visited)
/// and visited ⊆ processed at exit.
pub proof fn lemma_reachable_visited(
    edges: Seq<Seq<int>>,
    path: Seq<int>,
    from: int,
    visited: Seq<bool>,
    processed: Seq<bool>,
)
    requires
        is_path(edges, path, from, path[path.len() - 1]),
        0 <= from < visited.len(),
        visited[from],
        visited.len() == processed.len() == edges.len(),
        // edge closure: processed nodes' targets are visited
        forall|a: int, b: int| #![trigger edges[a].contains(b)]
            0 <= a < edges.len() && processed[a]
            && edges[a].contains(b) ==> 0 <= b < visited.len() && visited[b],
        // at exit, visited == processed
        forall|j: int| 0 <= j < visited.len() && visited[j] ==> processed[j],
    ensures
        visited[path[path.len() - 1]],
    decreases path.len(),
{
    let last = path[path.len() - 1];
    if path.len() > 1 {
        let prev = path[path.len() - 2];
        lemma_reachable_visited(
            edges,
            path.subrange(0, path.len() - 1),
            from,
            visited,
            processed,
        );
        // prev is visited, hence processed, hence last is visited
        assert(edges[prev].contains(last));
    }
}


/// build-only membership: incoming build edge and no incoming runtime
/// edge. (A trivial loop — the contract is the point.)
pub fn build_only_members(build_target: &[bool], runtime_target: &[bool]) -> (result: Vec<usize>)
    requires
        build_target@.len() == runtime_target@.len(),
    ensures
        forall|i: usize| result@.contains(i) <==>
            (i as int) < build_target@.len() && build_target@[i as int] && !runtime_target@[i as int],
        forall|a: int, b: int| 0 <= a < result@.len() && 0 <= b < result@.len()
            && result@[a] == result@[b] ==> a == b,
{
    let mut result: Vec<usize> = Vec::new();
    let mut i = 0;
    while i < build_target.len()
        invariant
            i <= build_target@.len(),
            build_target@.len() == runtime_target@.len(),
            forall|j: usize| result@.contains(j) <==>
                (j as int) < i && build_target@[j as int] && !runtime_target@[j as int],
            forall|a: int, b: int| 0 <= a < result@.len() && 0 <= b < result@.len()
                && result@[a] == result@[b] ==> a == b,
        decreases build_target.len() - i,
    {
        if build_target[i] && !runtime_target[i] {
            result.push(i);
        }
        i += 1;
    }
    result
}

}
