use anyhow::{Result, anyhow};
use socket2::{Domain, Protocol, Socket, Type};
use std::io;
use std::net::{self, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::TcpListener;

const LISTEN_BACKLOG: i32 = 128;

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
            .flat_map(|port| {
                [
                    SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
                    SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
                ]
            })
            .find_map(|addr| bind(addr).ok())
            .ok_or_else(|| anyhow!("Could not bind to any port in range 6881-6889"))?;

        Ok(TcpListener::from_std(listener)?)
    }
}

fn bind(addr: SocketAddr) -> io::Result<net::TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(LISTEN_BACKLOG)?;

    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn listener_cases() {
        let client = Client::build().unwrap();
        let port = client.port().unwrap();
        assert!((6881..=6889).contains(&port));
        assert!(client.id.starts_with(b"-RS0001-"));

        for ip in [
            IpAddr::from(Ipv4Addr::LOCALHOST),
            Ipv6Addr::LOCALHOST.into(),
        ] {
            let stream = TcpStream::connect((ip, port)).await.unwrap();
            let (_, remote) = client.lst.accept().await.unwrap();
            assert_eq!(remote.ip().to_canonical(), ip);
            assert_eq!(remote.port(), stream.local_addr().unwrap().port());
        }
    }
}
