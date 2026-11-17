use anyhow::{Result, ensure};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zerocopy::{FromBytes, FromZeros, Immutable, IntoBytes, KnownLayout};

const PSTRLEN: u8 = 19;
const PROTOCOL: &[u8; PSTRLEN as usize] = b"BitTorrent protocol";

#[derive(Debug, Clone, PartialEq, Eq, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct Handshake {
    pstrlen: u8,
    pstr: [u8; PSTRLEN as usize],
    pub reserved: [u8; 8],
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handshake {
    #[must_use]
    pub fn new(info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        Self {
            pstrlen: PSTRLEN,
            pstr: *PROTOCOL,
            reserved: [0; 8],
            info_hash,
            peer_id,
        }
    }

    pub async fn write_to<S>(&self, stream: &mut S) -> Result<()>
    where
        S: AsyncWrite + Unpin,
    {
        stream.write_all(self.as_bytes()).await?;
        Ok(())
    }

    pub async fn read_from<S>(stream: &mut S) -> Result<Self>
    where
        S: AsyncRead + Unpin,
    {
        let mut handshake = Self::new_zeroed();
        stream.read_exact(handshake.as_mut_bytes()).await?;

        ensure!(
            handshake.pstrlen == PSTRLEN && &handshake.pstr == PROTOCOL,
            "Unsupported protocol: {:?}",
            String::from_utf8_lossy(&handshake.pstr)
        );

        Ok(handshake)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[test]
    fn wire_layout() {
        let info_hash = [0xAA; 20];
        let peer_id = *b"-RS0001-123456789012";
        let bytes = Handshake::new(info_hash, peer_id).as_bytes().to_vec();

        assert_eq!(bytes.len(), 68);
        assert_eq!(bytes[0], 19);
        assert_eq!(&bytes[1..20], PROTOCOL);
        assert_eq!(&bytes[20..28], &[0; 8]);
        assert_eq!(&bytes[28..48], &info_hash);
        assert_eq!(&bytes[48..68], &peer_id);
    }

    async fn read(bytes: &[u8]) -> Result<Handshake> {
        let (mut local, mut remote) = duplex(256);
        remote.write_all(bytes).await.unwrap();
        drop(remote);
        Handshake::read_from(&mut local).await
    }

    #[tokio::test]
    async fn stream_cases() {
        let mut theirs = Handshake::new([1; 20], [3; 20]);
        theirs.reserved[5] = 0x10;

        let (mut local, mut remote) = duplex(256);
        theirs.write_to(&mut local).await.unwrap();
        assert_eq!(Handshake::read_from(&mut remote).await.unwrap(), theirs);

        assert_eq!(read(theirs.as_bytes()).await.unwrap(), theirs);

        let mut bad_len = theirs.clone();
        bad_len.pstrlen = 18;
        assert!(read(bad_len.as_bytes()).await.is_err());

        let mut bad_pstr = theirs.clone();
        bad_pstr.pstr = *b"BitTorrent protocoX";
        assert!(read(bad_pstr.as_bytes()).await.is_err());

        assert!(read(&theirs.as_bytes()[..40]).await.is_err());
    }
}
