//! Decode the same bounded v2 messages used by production. This target is
//! retained for the separately authorized fuzz lane; integration does not run
//! fuzzing or corpus replay under the owner's current instruction.
use gripsack_buildkit::protocol::{
    ExecuteRequest, Frame, FromBridge, LowerRequest, decode_frame, encode_frame,
};
use serde::{Deserialize, Serialize};

pub(crate) fn exercise(input: &[u8]) {
    let (target, wire) = input.split_first().unwrap_or((&0, &[]));
    match target % 3 {
        0 => {
            if let Ok(Frame { body }) = decode_frame::<FromBridge>(wire) {
                round_trip(&body);
            }
        }
        1 => {
            if let Ok(Frame { body }) = decode_frame::<LowerRequest>(wire) {
                round_trip(&body);
            }
        }
        _ => {
            if let Ok(Frame { body }) = decode_frame::<ExecuteRequest>(wire) {
                round_trip(&body);
            }
        }
    }
}

fn round_trip<M>(body: &M)
where
    M: Serialize + for<'de> Deserialize<'de> + std::fmt::Debug + PartialEq,
{
    // Canonical encoding can introduce omitted nullable fields. The production
    // encoder must still refuse a resulting request exceeding the frame budget.
    let Ok(encoded) = encode_frame(body) else {
        return;
    };
    let Frame { body: again } =
        decode_frame::<M>(&encoded).expect("encoded admitted message decodes");
    assert_eq!(body, &again, "bridge canonicalization changed semantics");
}
