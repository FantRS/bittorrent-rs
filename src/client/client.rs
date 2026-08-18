use crate::torrent::Torrent;

#[derive(Debug)]
pub struct Client {
    pub id: [u8; 20],
}

impl Client {
    pub fn new() -> Self {
        let id = Self::generate_id();

        Self { id }
    }

    pub fn format_url(&self, torrent: &Torrent) -> String {
        let base_url = torrent.meta.announce.as_str();
        let info_hash = urlencoding::encode_binary(&torrent.info_hash);
        let peer_id = urlencoding::encode_binary(&self.id);

        format!("{}?info_hash={}&peer_id={}", base_url, info_hash, peer_id)
    }

    fn generate_id() -> [u8; 20] {
        let mut id = [0; 20];
        id[..8].copy_from_slice(b"-RS0001-");

        let rands: [u8; 12] = rand::random();
        id[8..].copy_from_slice(&rands);

        id
    }
}
