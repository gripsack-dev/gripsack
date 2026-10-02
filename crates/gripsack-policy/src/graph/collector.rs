//! Structural collection laws used by the actual v6 typed collector.
//! These do not assume the source-to-edge correspondence: its callers prove
//! that correspondence over the transparent production declaration types.
use vstd::prelude::*;
verus! {
pub open spec fn expand<T, E>(values: Set<T>, f: spec_fn(T) -> ISet<E>) -> ISet<E> {
    ISet::new(|edge: E| exists|value: T| values.contains(value) && f(value).contains(edge))
}
pub open spec fn expand_seq<T, E>(values: Seq<T>, f: spec_fn(T) -> ISet<E>) -> ISet<E> {
    expand(values.to_set(), f)
}
pub proof fn expand_union<T, E>(a: Set<T>, b: Set<T>, f: spec_fn(T) -> ISet<E>)
    ensures expand(a.union(b), f) =~= expand(a, f).union(expand(b, f)),
{
    assert forall|e: E| expand(a.union(b), f).contains(e) <==> expand(a, f).union(expand(b, f)).contains(e) by {
        if expand(a.union(b), f).contains(e) {
            let v = choose|v: T| a.union(b).contains(v) && f(v).contains(e);
            if a.contains(v) { assert(expand(a, f).contains(e)); }
            else { assert(expand(b, f).contains(e)); }
        } else if expand(a, f).contains(e) {
            let v = choose|v: T| a.contains(v) && f(v).contains(e);
            assert(a.union(b).contains(v));
        } else if expand(b, f).contains(e) {
            let v = choose|v: T| b.contains(v) && f(v).contains(e);
            assert(a.union(b).contains(v));
        }
    }
}
pub proof fn expand_insert<T, E>(a: Set<T>, value: T, f: spec_fn(T) -> ISet<E>)
    ensures expand(a.insert(value), f) =~= expand(a, f).union(f(value)),
{
    assert forall|e: E| expand(a.insert(value), f).contains(e) <==> expand(a, f).union(f(value)).contains(e) by {
        if expand(a.insert(value), f).contains(e) {
            let v = choose|v: T| a.insert(value).contains(v) && f(v).contains(e);
            if v != value { assert(a.contains(v)); assert(expand(a, f).contains(e)); }
        } else if expand(a, f).contains(e) {
            let v = choose|v: T| a.contains(v) && f(v).contains(e);
            assert(a.insert(value).contains(v));
        } else if f(value).contains(e) { assert(a.insert(value).contains(value)); }
    }
}
pub proof fn expand_push<T, E>(a: Seq<T>, value: T, f: spec_fn(T) -> ISet<E>)
    ensures expand_seq(a.push(value), f) =~= expand_seq(a, f).union(f(value)),
{
    broadcast use vstd::seq_lib::group_seq_properties;
    assert(a.push(value).to_set() =~= a.to_set().insert(value));
    expand_insert(a.to_set(), value, f);
}
pub proof fn expand_take<T, E>(a: Seq<T>, i: int, f: spec_fn(T) -> ISet<E>)
    requires 0 <= i < a.len(),
    ensures expand_seq(a.take(i + 1), f) =~= expand_seq(a.take(i), f).union(f(a[i])),
{
    assert(a.take(i + 1) =~= a.take(i).push(a[i]));
    expand_push(a.take(i), a[i], f);
}
}
