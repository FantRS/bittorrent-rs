use anyhow::{Context, Result, anyhow, bail};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use tokio::net::{UdpSocket, lookup_host};
use tokio::time::{Instant, timeout_at};
use url::{Host, Url};
use zerocopy::network_endian::{I32, U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

use crate::peer::compact::parse_compact_peers;
use crate::tracker::{Announce, Event, TrackerResponse};

const PROTOCOL_ID: u64 = 0x0417_2710_1980;
const ACTION_CONNECT: u32 = 0;
const ACTION_ANNOUNCE: u32 = 1;
const ACTION_ERROR: u32 = 3;
const ATTEMPTS: u32 = 3;
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PACKET_LEN: usize = 2048;

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct ConnectRequest {
    protocol_id: U64,
    action: U32,
    transaction_id: U32,
}

#[derive(IntoBytes, Immutable)]
#[repr(C)]
struct AnnounceRequest {
    connection_id: U64,
    action: U32,
    transaction_id: U32,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
    downloaded: U64,
    left: U64,
    uploaded: U64,
    event: U32,
    ip: U32,
    key: U32,
    num_want: I32,
    port: U16,
}

#[derive(FromBytes, KnownLayout, Immutable)]
#[repr(C)]
struct Header {
    action: U32,
    transaction_id: U32,
}

#[derive(FromBytes, KnownLayout, Immutable)]
#[repr(C)]
struct AnnounceResponse {
    interval: U32,
    leechers: U32,
    seeders: U32,
}

pub async fn announce(url: &Url, request: &Announce) -> Result<TrackerResponse> {
    let port = url.port().context("Tracker URL has no port")?;
    let addr = match url.host().context("Tracker URL has no host")? {
        Host::Domain(host) => lookup_host((host, port))
            .await?
            .next()
            .with_context(|| format!("Could not resolve {host}"))?,
        Host::Ipv4(ip) => (ip, port).into(),
        Host::Ipv6(ip) => (ip, port).into(),
    };

    let local: SocketAddr = match addr {
        SocketAddr::V4(_) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        SocketAddr::V6(_) => (Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let socket = UdpSocket::bind(local).await?;
    socket.connect(addr).await?;

    let connection_id = connect(&socket).await?;
    let transaction_id: u32 = rand::random();
    let packet = AnnounceRequest {
        connection_id: connection_id.into(),
        action: ACTION_ANNOUNCE.into(),
        transaction_id: transaction_id.into(),
        info_hash: request.info_hash,
        peer_id: request.peer_id,
        downloaded: request.downloaded.into(),
        left: request.left.into(),
        uploaded: request.uploaded.into(),
        event: event_code(request.event).into(),
        ip: 0.into(),
        key: rand::random::<u32>().into(),
        num_want: (-1).into(),
        port: request.port.into(),
    };

    let reply = exchange(&socket, packet.as_bytes(), transaction_id, ACTION_ANNOUNCE).await?;
    let (body, peers) = AnnounceResponse::ref_from_prefix(&reply)
        .map_err(|_| anyhow!("Announce response is too short"))?;
    let peers = match addr {
        SocketAddr::V4(_) => parse_compact_peers::<[u8; 4]>(peers)?,
        SocketAddr::V6(_) => parse_compact_peers::<[u8; 16]>(peers)?,
    };

    Ok(TrackerResponse {
        interval: body.interval.get().into(),
        peers,
        tracker_id: None,
        complete: Some(body.seeders.get().into()),
        incomplete: Some(body.leechers.get().into()),
    })
}

async fn connect(socket: &UdpSocket) -> Result<u64> {
    let transaction_id: u32 = rand::random();
    let packet = ConnectRequest {
        protocol_id: PROTOCOL_ID.into(),
        action: ACTION_CONNECT.into(),
        transaction_id: transaction_id.into(),
    };

    let reply = exchange(socket, packet.as_bytes(), transaction_id, ACTION_CONNECT).await?;
    let (connection_id, _) =
        U64::read_from_prefix(&reply).map_err(|_| anyhow!("Connect response is too short"))?;

    Ok(connection_id.get())
}

async fn exchange(
    socket: &UdpSocket,
    packet: &[u8],
    transaction_id: u32,
    action: u32,
) -> Result<Vec<u8>> {
    let mut buf = vec![0; MAX_PACKET_LEN];

    for _ in 0..ATTEMPTS {
        socket.send(packet).await?;
        let deadline = Instant::now() + ATTEMPT_TIMEOUT;

        while let Ok(received) = timeout_at(deadline, socket.recv(&mut buf)).await {
            let datagram = buf.get(..received?).context("Datagram exceeds buffer")?;
            let Ok((header, body)) = Header::ref_from_prefix(datagram) else {
                continue;
            };
            if header.transaction_id.get() != transaction_id {
                continue;
            }

            match header.action.get() {
                ACTION_ERROR => bail!("{}", String::from_utf8_lossy(body)),
                received if received == action => return Ok(body.to_vec()),
                received => bail!("Unexpected tracker action: {received}"),
            }
        }
    }

    bail!("Tracker did not respond after {ATTEMPTS} attempts")
}

fn event_code(event: Option<Event>) -> u32 {
    match event {
        None => 0,
        Some(Event::Completed) => 1,
        Some(Event::Started) => 2,
        Some(Event::Stopped) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    async fn spawn_tracker(ip: IpAddr, error: Option<&'static str>) -> Url {
        let socket = UdpSocket::bind((ip, 0)).await.unwrap();
        let url = Url::parse(&format!("udp://{}", socket.local_addr().unwrap())).unwrap();
        let peer = match ip {
            IpAddr::V4(ip) => ip.octets().to_vec(),
            IpAddr::V6(ip) => ip.octets().to_vec(),
        };

        tokio::spawn(async move {
            let mut buf = [0; MAX_PACKET_LEN];
            loop {
                let (len, from) = socket.recv_from(&mut buf).await.unwrap();
                let (header, _) = Header::ref_from_prefix(buf.get(8..len).unwrap()).unwrap();
                let tid = header.transaction_id.get().to_be_bytes();

                let stray = [&1u32.to_be_bytes()[..], &[0; 4]].concat();
                socket.send_to(&stray, from).await.unwrap();

                let reply = match (header.action.get(), error) {
                    (ACTION_CONNECT, _) => {
                        [&0u32.to_be_bytes()[..], &tid, &42u64.to_be_bytes(), &[0; 4]].concat()
                    }
                    (_, Some(message)) => {
                        [&3u32.to_be_bytes()[..], &tid, message.as_bytes()].concat()
                    }
                    _ => [
                        &1u32.to_be_bytes()[..],
                        &tid,
                        &1800u32.to_be_bytes(),
                        &5u32.to_be_bytes(),
                        &9u32.to_be_bytes(),
                        &peer,
                        &6881u16.to_be_bytes(),
                    ]
                    .concat(),
                };
                socket.send_to(&reply, from).await.unwrap();
            }
        });

        url
    }

    #[tokio::test]
    async fn announce_cases() {
        let request = Announce {
            info_hash: [1; 20],
            peer_id: [2; 20],
            port: 6881,
            uploaded: 0,
            downloaded: 0,
            left: 100,
            event: Some(Event::Started),
        };
        assert_eq!(size_of::<AnnounceRequest>(), 98);

        for ip in [Ipv4Addr::LOCALHOST.into(), Ipv6Addr::LOCALHOST.into()] {
            let response = announce(&spawn_tracker(ip, None).await, &request)
                .await
                .unwrap();
            assert_eq!(response.interval, 1800);
            assert_eq!(response.complete, Some(9));
            assert_eq!(response.incomplete, Some(5));
            assert_eq!(response.peers, [SocketAddr::new(ip, 6881)]);
        }

        let tracker = spawn_tracker(Ipv4Addr::LOCALHOST.into(), Some("banned")).await;
        let error = announce(&tracker, &request).await.unwrap_err();
        assert_eq!(error.to_string(), "banned");
    }
}
