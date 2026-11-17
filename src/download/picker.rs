use bitvec::vec::BitVec;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug)]
pub struct Picker {
    state: Mutex<State>,
    released: watch::Sender<()>,
}

#[derive(Debug)]
struct State {
    available: BitVec,
    missing: BitVec,
}

#[derive(Debug)]
pub struct Reservation {
    picker: Arc<Picker>,
    index: usize,
    completed: bool,
}

impl Picker {
    #[must_use]
    pub fn new(missing: BitVec) -> Self {
        Self {
            state: Mutex::new(State {
                available: missing.clone(),
                missing,
            }),
            released: watch::Sender::new(()),
        }
    }

    pub fn reserve(self: &Arc<Self>, has_piece: impl Fn(usize) -> bool) -> Option<Reservation> {
        let mut state = self.state.lock();
        let index = state
            .available
            .iter_ones()
            .find(|&index| has_piece(index))?;
        state.available.set(index, false);

        Some(Reservation {
            picker: Arc::clone(self),
            index,
            completed: false,
        })
    }

    #[must_use]
    pub fn wants(&self, has_piece: impl Fn(usize) -> bool) -> bool {
        self.state.lock().missing.iter_ones().any(has_piece)
    }

    #[must_use]
    pub fn is_done(&self) -> bool {
        self.state.lock().missing.not_any()
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.released.subscribe()
    }

    fn release(&self, index: usize) {
        self.state.lock().available.set(index, true);
        self.released.send_replace(());
    }
}

impl Reservation {
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn complete(mut self) {
        self.picker.state.lock().missing.set(self.index, false);
        self.completed = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.completed {
            self.picker.release(self.index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_cases() {
        let picker = Arc::new(Picker::new(BitVec::repeat(true, 3)));
        let released = picker.subscribe();

        let first = picker.reserve(|_| true).unwrap();
        assert_eq!(first.index(), 0);
        let second = picker.reserve(|_| true).unwrap();
        assert_eq!(second.index(), 1);
        assert!(picker.reserve(|index| index < 2).is_none());

        drop(second);
        assert!(released.has_changed().unwrap());
        let retry = picker.reserve(|index| index == 1).unwrap();
        assert_eq!(retry.index(), 1);

        assert!(picker.wants(|index| index == 1));
        first.complete();
        retry.complete();
        assert!(!picker.wants(|index| index < 2));
        assert!(picker.wants(|index| index == 2));
        assert!(!picker.is_done());

        picker.reserve(|_| true).unwrap().complete();
        assert!(picker.is_done());
        assert!(picker.reserve(|_| true).is_none());
    }
}
