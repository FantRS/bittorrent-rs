use anyhow::{Result, anyhow};
use std::net::{self, Ipv4Addr};
use tokio::net::TcpListener;

#[derive(Debug)]
pub struct Client {
    pub id: [u8; 20],
    pub lst: TcpListener,
}

impl Client {
    pub fn build() -> Result<Self> {
        let id = Self::generate_id();
        let lst = Self::generate_lst()?;

        Ok(Self { id, lst })
    }

    pub fn port(&self) -> Result<u16> {
        Ok(self.lst.local_addr()?.port())
    }

    fn generate_id() -> [u8; 20] {
        let mut id: [u8; 20] = rand::random();
        id[..8].copy_from_slice(b"-RS0001-");
        id
    }

    fn generate_lst() -> Result<TcpListener> {
        let listener = (6881..=6889)
            .find_map(|port| net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).ok())
            .ok_or_else(|| anyhow!("Could not bind to any port in range 6881-6889"))?;
        listener.set_nonblocking(true)?;

        Ok(TcpListener::from_std(listener)?)
    }
}
