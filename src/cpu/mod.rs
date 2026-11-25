use anyhow::{Result, anyhow};
use std::panic::{self, AssertUnwindSafe};
use tokio::sync::oneshot;

pub async fn spawn<T>(work: impl FnOnce() -> T + Send + 'static) -> Result<T>
where
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    rayon::spawn(move || {
        let _ = sender.send(panic::catch_unwind(AssertUnwindSafe(work)));
    });

    receiver.await?.map_err(|_| anyhow!("CPU task panicked"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_cases() {
        assert_eq!(spawn(|| 2 + 2).await.unwrap(), 4);
        assert!(spawn(|| "x".parse::<u8>().unwrap()).await.is_err());
    }
}
