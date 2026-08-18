#[derive(Debug)]
pub struct Client {
    pub id: [u8; 20],
}

impl Client {
    pub fn new() -> Self {
        let id = Self::generate_id();

        Self { id }
    }

    fn generate_id() -> [u8; 20] {
        let mut id = [0; 20];
        id[..8].copy_from_slice(b"-RS0001-");

        let rands: [u8; 12] = rand::random();
        id[8..].copy_from_slice(&rands);

        id
    }
}
