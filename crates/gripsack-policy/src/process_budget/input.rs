//! Admitted request sizes and checked serializer growth.
//! Vec length/capacity and allocator behavior are caller contracts.
use vstd::prelude::*;

verus! {

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct InputByteLimit { bytes: usize }

impl InputByteLimit {
    pub closed spec fn view(self) -> usize { self.bytes }
    pub fn new(bytes: usize) -> (limit: Self)
        ensures limit.view() == bytes,
    { Self { bytes } }
    pub fn bytes(self) -> (bytes: usize)
        ensures bytes == self.view(),
    { self.bytes }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct InputAppend {
    ghost new_length: usize,
    reserve_additional: usize,
}

impl InputAppend {
    pub closed spec fn length(&self) -> usize { self.new_length }
    pub closed spec fn reservation(&self) -> usize { self.reserve_additional }
    pub fn reserve_additional(&self) -> (bytes: usize)
        ensures bytes == self.reservation(),
    { self.reserve_additional }
}

pub fn admit_input_append(limit: InputByteLimit, length: usize, capacity: usize, additional: usize) -> (append: Option<InputAppend>)
    requires length <= capacity, length <= limit.view(),
    ensures
        append.is_some() <==> (length as nat) + (additional as nat) <= limit.view(),
        append.is_some() ==> append.unwrap().length() == (length as nat) + (additional as nat),
        append.is_some() ==> append.unwrap().length() <= limit.view(),
        append.is_some() ==> (append.unwrap().reservation() == 0 <==> append.unwrap().length() <= capacity),
        append.is_some() && append.unwrap().reservation() > 0 ==>
            append.unwrap().length() <= (length as nat) + (append.unwrap().reservation() as nat) <= limit.view(),
{
    if additional > limit.bytes - length { return None; }
    let needed = length + additional;
    let reserve_additional = if needed > capacity {
        let doubled = if capacity > usize::MAX / 2 { usize::MAX } else { capacity * 2 };
        let desired = if doubled > needed { doubled } else { needed };
        let capped = if desired < limit.bytes { desired } else { limit.bytes };
        capped - length
    } else { 0 };
    Some(InputAppend { new_length: needed, reserve_additional })
}

}
