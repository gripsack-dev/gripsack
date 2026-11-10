//! Admitted request sizes and checked serializer growth.
//! Vec length/capacity and allocator behavior are caller contracts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputByteLimit {
    bytes: usize,
}

impl InputByteLimit {
    pub fn new(bytes: usize) -> Self {
        Self { bytes }
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputAppend {
    reserve_additional: usize,
}

impl InputAppend {
    pub fn reserve_additional(&self) -> usize {
        self.reserve_additional
    }
}

/// Admit `additional` bytes onto a buffer of `length` (≤ `capacity`, ≤ the
/// limit) only if the result stays within the limit; the reservation grows
/// the buffer geometrically but never past the limit.
pub fn admit_input_append(
    limit: InputByteLimit,
    length: usize,
    capacity: usize,
    additional: usize,
) -> Option<InputAppend> {
    debug_assert!(length <= capacity && length <= limit.bytes);
    if additional > limit.bytes - length {
        return None;
    }
    let needed = length + additional;
    let reserve_additional = if needed > capacity {
        let doubled = if capacity > usize::MAX / 2 {
            usize::MAX
        } else {
            capacity * 2
        };
        let desired = if doubled > needed { doubled } else { needed };
        let capped = if desired < limit.bytes {
            desired
        } else {
            limit.bytes
        };
        capped - length
    } else {
        0
    };
    Some(InputAppend { reserve_additional })
}
