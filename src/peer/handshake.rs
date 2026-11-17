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

    pub async fn exchange<S>(&self, stream: &mut S) -> Result<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        stream.write_all(self.as_bytes()).await?;

        let mut reply = Self::new_zeroed();
        stream.read_exact(reply.as_mut_bytes()).await?;

        ensure!(
            reply.pstrlen == PSTRLEN && &reply.pstr == PROTOCOL,
            "Unsupported protocol: {:?}",
            String::from_utf8_lossy(&reply.pstr)
        );

        Ok(reply)
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

    async fn run(ours: &Handshake, reply: &[u8]) -> (Result<Handshake>, Handshake) {
        let (mut local, mut remote) = duplex(256);
        remote.write_all(reply).await.unwrap();
        remote.shutdown().await.unwrap();

        let result = ours.exchange(&mut local).await;
        let mut sent = Handshake::new_zeroed();
        remote.read_exact(sent.as_mut_bytes()).await.unwrap();

        (result, sent)
    }

    #[tokio::test]
    async fn exchange_cases() {
        let ours = Handshake::new([1; 20], [2; 20]);
        let mut theirs = Handshake::new([1; 20], [3; 20]);
        theirs.reserved[5] = 0x10;

        let (reply, sent) = run(&ours, theirs.as_bytes()).await;
        assert_eq!(reply.unwrap(), theirs);
        assert_eq!(sent, ours);

        let mut bad_len = theirs.clone();
        bad_len.pstrlen = 18;
        assert!(run(&ours, bad_len.as_bytes()).await.0.is_err());

        let mut bad_pstr = theirs.clone();
        bad_pstr.pstr = *b"BitTorrent protocoX";
        assert!(run(&ours, bad_pstr.as_bytes()).await.0.is_err());

        assert!(run(&ours, &theirs.as_bytes()[..40]).await.0.is_err());
    }
}
