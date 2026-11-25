use anyhow::Result;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::task::{self, JoinSet};
use tokio::time::Instant;

const RETRY_DELAY: Duration = Duration::from_mins(5);

#[derive(Debug)]
pub struct Swarm<T> {
    limit: usize,
    tasks: JoinSet<Result<T>>,
    active: HashMap<task::Id, SocketAddr>,
    queued: VecDeque<SocketAddr>,
    known: HashMap<SocketAddr, Option<Instant>>,
}

impl<T: Send + 'static> Swarm<T> {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            tasks: JoinSet::new(),
            active: HashMap::new(),
            queued: VecDeque::new(),
            known: HashMap::new(),
        }
    }

    pub fn enqueue(&mut self, addrs: impl IntoIterator<Item = SocketAddr>) {
        for addr in addrs {
            let busy = match self.known.get(&addr) {
                Some(None) => true,
                Some(Some(left)) => left.elapsed() < RETRY_DELAY,
                None => false,
            };
            if !busy {
                self.known.insert(addr, None);
                self.queued.push_back(addr);
            }
        }
    }

    pub fn spawn_queued<F>(&mut self, mut connect: impl FnMut(SocketAddr) -> F)
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        while self.active.len() < self.limit
            && let Some(addr) = self.queued.pop_front()
        {
            let id = self.tasks.spawn(connect(addr)).id();
            self.active.insert(id, addr);
        }
    }

    pub async fn join_next(&mut self) -> Option<Result<T>> {
        let (id, result) = match self.tasks.join_next_with_id().await? {
            Ok((id, result)) => (id, result),
            Err(e) => (e.id(), Err(e.into())),
        };
        if let Some(addr) = self.active.remove(&id) {
            self.known.insert(addr, Some(Instant::now()));
        }

        Some(result)
    }

    pub fn has_tasks(&self) -> bool {
        !self.tasks.is_empty()
    }

    pub fn is_idle(&self) -> bool {
        self.active.is_empty() && self.queued.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    #[tokio::test(start_paused = true)]
    async fn swarm_cases() {
        let addr: SocketAddr = "1.2.3.4:5".parse().unwrap();
        let other: SocketAddr = "1.2.3.4:6".parse().unwrap();
        let mut swarm = Swarm::new(1);
        let mut spawned = Vec::new();
        let mut connect = |addr| {
            spawned.push(addr);
            async move {
                if addr == other {
                    bail!("refused")
                }
                Ok(addr)
            }
        };

        swarm.enqueue([addr, addr, other]);
        swarm.spawn_queued(&mut connect);
        assert!(swarm.has_tasks());
        swarm.enqueue([addr]);
        assert_eq!(swarm.join_next().await.unwrap().unwrap(), addr);

        swarm.spawn_queued(&mut connect);
        assert!(swarm.join_next().await.unwrap().is_err());
        assert!(swarm.is_idle() && !swarm.has_tasks());

        swarm.enqueue([addr, other]);
        assert!(swarm.is_idle());
        tokio::time::advance(RETRY_DELAY).await;
        swarm.enqueue([other]);
        swarm.spawn_queued(&mut connect);
        assert!(swarm.join_next().await.unwrap().is_err());
        assert_eq!(spawned, [addr, other, other]);
    }
}
