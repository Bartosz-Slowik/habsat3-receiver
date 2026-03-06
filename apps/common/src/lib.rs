use chacha20poly1305::{ChaCha20Poly1305, KeyInit as _, Nonce, aead::Aead as _};
use zerocopy::IntoBytes as _;

#[derive(
    zerocopy::IntoBytes,
    zerocopy::Immutable,
    zerocopy::FromBytes,
    Default,
    Debug,
    PartialEq,
    Clone,
    Copy,
)]
pub struct RadioMsg {
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub course_over_ground_degrees: f64,
    pub speed_over_ground_meters_per_second: f64,
    pub altitude_meters: f64,
    pub satellites: u64,
}

impl RadioMsg {
    pub fn encrypt(&self, timestamp: f64, key: &[u8; 32]) -> Vec<u8> {
        let cipher = ChaCha20Poly1305::new(key.into());

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&timestamp.to_le_bytes());
        let nonce = Nonce::from(nonce_bytes);

        let encrypted = cipher.encrypt(&nonce, self.as_bytes()).unwrap();
        let mut full = Vec::from(timestamp.to_le_bytes());
        full.extend_from_slice(&encrypted);

        full
    }

    pub fn decrypt(raw_msg: &[u8], key: &[u8; 32]) -> Option<(f64, RadioMsg)> {
        let cipher = ChaCha20Poly1305::new(key.into());

        if raw_msg.len() != size_of::<f64>() + size_of::<RadioMsg>() + 16 {
            println!("Bad message length");
            return None;
        }

        let timestamp = {
            let mut tmp = 0_f64;
            tmp.as_mut_bytes()
                .copy_from_slice(&raw_msg[..size_of::<f64>()]);

            tmp
        };

        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..size_of_val(&timestamp)].copy_from_slice(&timestamp.to_le_bytes());
        let nonce = Nonce::from(nonce_bytes);

        let bytes = cipher
            .decrypt(&nonce, &raw_msg[size_of_val(&timestamp)..])
            .inspect_err(|e| println!("Failed to decrypt: {e}"))
            .ok()?;

        let decrypted_msg = {
            let mut tmp = RadioMsg::default();
            tmp.as_mut_bytes().copy_from_slice(&bytes);
            tmp
        };

        Some((timestamp, decrypted_msg))
    }
}

#[cfg(test)]
mod tests {
    use crate::RadioMsg;

    #[test]
    fn encrypt_decrypt() {
        let msg = RadioMsg {
            latitude_degrees: 12.,
            longitude_degrees: 13.,
            course_over_ground_degrees: 14.,
            speed_over_ground_meters_per_second: 15.,
            altitude_meters: 16.,
            satellites: 17,
        };
        let timestamp = 111.;

        let key = b"aaaabbbbccccddddeeeeffffgggghhhh";

        let encrypted = msg.encrypt(timestamp, key);
        let decrypted = RadioMsg::decrypt(&encrypted, key);

        assert_eq!(decrypted.unwrap(), (timestamp, msg));
    }
}
