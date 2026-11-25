use bytes::{Bytes, BytesMut};
use std::collections::VecDeque;

use crate::peer::{BLOCK_LEN, Piece, Request};
use crate::transfer::download::picker::Reservation;

#[derive(Debug, Default)]
pub struct Assembler {
    pieces: Vec<Assembly>,
    pending: Vec<Request>,
    retry: VecDeque<Request>,
}

#[derive(Debug)]
struct Assembly {
    reservation: Reservation,
    data: BytesMut,
    length: u32,
    next: u32,
    missing: usize,
}

#[derive(Debug)]
pub struct Assembled {
    pub reservation: Reservation,
    pub data: Bytes,
}

impl Assembler {
    pub fn next_request(
        &mut self,
        reserve: impl FnOnce() -> Option<(Reservation, u32)>,
    ) -> Option<Request> {
        if let Some(request) = self.retry.pop_front() {
            return Some(self.track(request));
        }

        if let Some(request) = self.pieces.iter_mut().find_map(Assembly::next_request) {
            return Some(self.track(request));
        }

        let (reservation, length) = reserve()?;
        let mut assembly = Assembly::new(reservation, length);
        let request = assembly.next_request()?;
        self.pieces.push(assembly);

        Some(self.track(request))
    }

    pub fn receive(&mut self, piece: &Piece) -> Option<Assembled> {
        let position = self.pending.iter().position(|request| {
            request.index == piece.index
                && request.begin == piece.begin
                && usize::try_from(request.length).is_ok_and(|len| len == piece.block.len())
        })?;
        self.pending.remove(position);

        let slot = self
            .pieces
            .iter()
            .position(|assembly| assembly.reservation.index() == piece.index)?;
        let assembly = self.pieces.get_mut(slot)?;
        let start = usize::try_from(piece.begin).ok()?;
        assembly
            .data
            .get_mut(start..start + piece.block.len())?
            .copy_from_slice(&piece.block);
        assembly.missing -= 1;

        (assembly.missing == 0).then(|| self.pieces.swap_remove(slot).into_assembled())
    }

    pub fn requeue(&mut self) {
        self.retry.extend(self.pending.drain(..));
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn is_idle(&self) -> bool {
        self.pieces.is_empty()
    }

    fn track(&mut self, request: Request) -> Request {
        self.pending.push(request);
        request
    }
}

impl Assembly {
    fn new(reservation: Reservation, length: u32) -> Self {
        Self {
            data: BytesMut::zeroed(usize::try_from(length).unwrap_or_default()),
            reservation,
            length,
            next: 0,
            missing: usize::try_from(length.div_ceil(BLOCK_LEN)).unwrap_or_default(),
        }
    }

    fn next_request(&mut self) -> Option<Request> {
        let begin = self.next;
        let length = self
            .length
            .checked_sub(begin)
            .filter(|&rest| rest > 0)?
            .min(BLOCK_LEN);
        self.next += length;

        Some(Request {
            index: self.reservation.index(),
            begin,
            length,
        })
    }

    fn into_assembled(self) -> Assembled {
        Assembled {
            reservation: self.reservation,
            data: self.data.freeze(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::download::picker::Picker;
    use bitvec::vec::BitVec;
    use std::sync::Arc;

    fn piece(index: u32, begin: u32, len: usize, byte: u8) -> Piece {
        Piece {
            index: index.into(),
            begin,
            block: Bytes::from(vec![byte; len]),
        }
    }

    #[test]
    fn assembler_cases() {
        let picker = Arc::new(Picker::new(BitVec::repeat(true, 2)));
        let length = BLOCK_LEN + 10;
        let reserve = || {
            picker
                .reserve(|_| true)
                .map(|reservation| (reservation, length))
        };
        let mut assembler = Assembler::default();

        let requests: Vec<(u32, u32, u32)> = std::iter::from_fn(|| assembler.next_request(reserve))
            .map(|request| (request.index.get(), request.begin, request.length))
            .collect();
        assert_eq!(
            requests,
            [
                (0, 0, BLOCK_LEN),
                (0, BLOCK_LEN, 10),
                (1, 0, BLOCK_LEN),
                (1, BLOCK_LEN, 10)
            ]
        );
        assert_eq!(assembler.pending(), 4);

        let block = BLOCK_LEN as usize;
        assert!(assembler.receive(&piece(0, 0, block, 1)).is_none());
        assert!(assembler.receive(&piece(0, 0, block, 1)).is_none());
        assert!(assembler.receive(&piece(0, 7, 10, 1)).is_none());
        assert_eq!(assembler.pending(), 3);

        assembler.requeue();
        assert_eq!(assembler.pending(), 0);
        let retried = assembler.next_request(|| None).unwrap();
        assert_eq!((retried.index.get(), retried.begin), (0, BLOCK_LEN));

        let complete = assembler.receive(&piece(0, BLOCK_LEN, 10, 2)).unwrap();
        assert_eq!(complete.reservation.index().get(), 0);
        assert_eq!(complete.data.len(), block + 10);
        assert_eq!(complete.data[(block - 1)..=block], [1, 2]);
        assert!(!assembler.is_idle());
    }
}
