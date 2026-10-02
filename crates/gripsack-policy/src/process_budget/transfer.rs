//! One outstanding bounded pipe write, with monotone acknowledged input.
use super::InputByteLimit;
use vstd::prelude::*;

verus! {

pub const IO_CHUNK_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
enum TransferPhase { Ready, Offered(usize), Closed }

pub struct InputChunk { start: usize, end: usize }
impl InputChunk {
    pub closed spec fn start(&self) -> usize { self.start }
    pub closed spec fn end(&self) -> usize { self.end }
    pub fn range(self) -> (range: std::ops::Range<usize>)
        ensures range.start == self.start(), range.end == self.end(),
    { self.start..self.end }
}

pub struct InputTransfer { total: usize, sent: usize, phase: TransferPhase }
impl InputTransfer {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self.sent <= self.total
            && (match self.phase {
                TransferPhase::Ready => self.sent < self.total,
                TransferPhase::Offered(bytes) => 0 < bytes <= IO_CHUNK_BYTES
                    && self.sent + bytes <= self.total,
                TransferPhase::Closed => true,
            })
    }

    pub closed spec fn total(&self) -> usize { self.total }
    pub closed spec fn sent(&self) -> usize { self.sent }
    pub closed spec fn ready(&self) -> bool { self.phase == TransferPhase::Ready }
    pub closed spec fn closed(&self) -> bool { self.phase == TransferPhase::Closed }
    pub closed spec fn offered(&self) -> Option<usize> {
        match self.phase { TransferPhase::Offered(bytes) => Some(bytes), _ => None }
    }

    pub fn admit(length: usize, limit: InputByteLimit) -> (transfer: Option<Self>)
        ensures
            transfer.is_some() <==> length <= limit.view(),
            transfer.is_some() ==> transfer.unwrap().total() == length
                && transfer.unwrap().sent() == 0
                && (transfer.unwrap().closed() <==> length == 0),
            transfer.is_some() && length > 0 ==> transfer.unwrap().ready(),
    {
        if length > limit.bytes() { return None; }
        Some(Self { total: length, sent: 0,
            phase: if length == 0 { TransferPhase::Closed } else { TransferPhase::Ready } })
    }

    pub fn begin(&mut self) -> (chunk: Option<InputChunk>)
        ensures
            final(self).total() == old(self).total(), final(self).sent() == old(self).sent(),
            chunk.is_some() <==> old(self).ready(),
            !chunk.is_some() ==> final(self).offered() == old(self).offered()
                && final(self).closed() == old(self).closed(),
            chunk.is_some() ==> chunk.unwrap().start() == old(self).sent()
                && chunk.unwrap().start() < chunk.unwrap().end() <= old(self).total()
                && chunk.unwrap().end() - chunk.unwrap().start() <= IO_CHUNK_BYTES
                && final(self).offered() == Some((chunk.unwrap().end() - chunk.unwrap().start()) as usize),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if !matches!(self.phase, TransferPhase::Ready) { return None; }
        let remaining = self.total - self.sent;
        let bytes = if remaining < IO_CHUNK_BYTES { remaining } else { IO_CHUNK_BYTES };
        let end = self.sent + bytes;
        self.phase = TransferPhase::Offered(bytes);
        Some(InputChunk { start: self.sent, end })
    }

    pub fn complete(&mut self, written: usize) -> (accepted: bool)
        ensures
            final(self).total() == old(self).total(),
            accepted <==> old(self).offered().is_some()
                && 0 < written <= old(self).offered().unwrap(),
            accepted ==> final(self).sent() == old(self).sent() + written,
            !accepted ==> final(self).sent() == old(self).sent() && final(self).closed(),
            final(self).offered().is_none(),
            accepted ==> (final(self).closed() <==> final(self).sent() == final(self).total()),
            accepted && final(self).sent() < final(self).total() ==> final(self).ready(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        let bytes = match self.phase {
            TransferPhase::Offered(bytes) => bytes,
            _ => { self.phase = TransferPhase::Closed; return false; }
        };
        if written == 0 || written > bytes {
            self.phase = TransferPhase::Closed;
            return false;
        }
        self.phase = TransferPhase::Closed;
        self.sent += written;
        if self.sent < self.total { self.phase = TransferPhase::Ready; }
        true
    }

    pub fn retry(&mut self)
        ensures
            final(self).total() == old(self).total(), final(self).sent() == old(self).sent(),
            final(self).closed() == old(self).closed(),
            final(self).ready() <==> old(self).ready() || old(self).offered().is_some(),
            final(self).offered().is_none(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        if matches!(self.phase, TransferPhase::Offered(_)) { self.phase = TransferPhase::Ready; }
    }

    pub fn close(&mut self)
        ensures final(self).closed(), final(self).total() == old(self).total(),
            final(self).sent() == old(self).sent(),
        no_unwind
    {
        proof { use_type_invariant(&*self); }
        self.phase = TransferPhase::Closed;
    }

    pub fn is_closed(&self) -> (closed: bool)
        ensures closed == self.closed(),
        no_unwind
    { matches!(self.phase, TransferPhase::Closed) }
}

}
