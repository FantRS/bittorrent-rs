use anyhow::{Result, anyhow};
use std::net::{IpAddr, SocketAddr};
use zerocopy::network_endian::U16;
use zerocopy::{FromBytes, Immutable, KnownLayout};

#[derive(FromBytes, KnownLayout, Immutable)]
#[repr(C)]
struct CompactPeer<Ip> {
    ip: Ip,
    port: U16,
}

pub(crate) fn parse_compact_peers<Ip>(bytes: &[u8]) -> Result<Vec<SocketAddr>>
where
    Ip: FromBytes + KnownLayout + Immutable + Copy,
    IpAddr: From<Ip>,
{
    let peers = <[CompactPeer<Ip>]>::ref_from_bytes(bytes)
        .map_err(|_| anyhow!("Invalid compact peers length: {}", bytes.len()))?;

    Ok(peers
        .iter()
        .map(|peer| SocketAddr::new(peer.ip.into(), peer.port.get()))
        .collect())
}
