// Copyright 2020 TiKV Project Authors. Licensed under Apache-2.0.

use engine_traits::EncryptionMethod as DBEncryptionMethod;
use kvproto::encryptionpb::EncryptionMethod;
use openssl::symm::{self, Cipher as OCipher};

use crate::Result;

#[cfg(not(feature = "prost-codec"))]
pub fn encryption_method_to_db_encryption_method(method: EncryptionMethod) -> DBEncryptionMethod {
    match method {
        EncryptionMethod::Plaintext => DBEncryptionMethod::Plaintext,
        EncryptionMethod::Aes128Ctr => DBEncryptionMethod::Aes128Ctr,
        EncryptionMethod::Aes192Ctr => DBEncryptionMethod::Aes192Ctr,
        EncryptionMethod::Aes256Ctr => DBEncryptionMethod::Aes256Ctr,
        EncryptionMethod::Unknown => DBEncryptionMethod::Unknown,
    }
}

#[cfg(not(feature = "prost-codec"))]
pub fn compat(method: EncryptionMethod) -> EncryptionMethod {
    method
}

#[cfg(feature = "prost-codec")]
pub fn encryption_method_to_db_encryption_method(
    method: i32, /* EncryptionMethod */
) -> DBEncryptionMethod {
    match method {
        1/* EncryptionMethod::Plaintext */ => DBEncryptionMethod::Plaintext,
        2/* EncryptionMethod::Aes128Ctr */ => DBEncryptionMethod::Aes128Ctr,
        3/* EncryptionMethod::Aes192Ctr */ => DBEncryptionMethod::Aes192Ctr,
        4/* EncryptionMethod::Aes256Ctr */ => DBEncryptionMethod::Aes256Ctr,
        _/* EncryptionMethod::Unknown */ => DBEncryptionMethod::Unknown,
    }
}

#[cfg(feature = "prost-codec")]
pub fn compat(method: EncryptionMethod) -> i32 {
    match method {
        EncryptionMethod::Unknown => 0,
        EncryptionMethod::Plaintext => 1,
        EncryptionMethod::Aes128Ctr => 2,
        EncryptionMethod::Aes192Ctr => 3,
        EncryptionMethod::Aes256Ctr => 4,
    }
}

pub fn get_method_key_length(method: EncryptionMethod) -> usize {
    match method {
        EncryptionMethod::Plaintext => 0,
        EncryptionMethod::Aes128Ctr => 16,
        EncryptionMethod::Aes192Ctr => 24,
        EncryptionMethod::Aes256Ctr => 32,
        unknown => panic!("bad EncryptionMethod {:?}", unknown),
    }
}

// IV as an AES input, the length should be 12 btyes for GCM mode.
const GCM_IV_12: usize = 12;

#[derive(Debug, Clone, Copy)]
pub struct Iv {
    iv: [u8; GCM_IV_12],
}

impl Iv {
    pub fn as_slice(&self) -> &[u8] {
        &self.iv
    }
}

impl<'a> From<&'a [u8]> for Iv {
    fn from(src: &'a [u8]) -> Iv {
        assert!(
            src.len() >= GCM_IV_12,
            "Nonce + Counter must be greater than 12 bytes"
        );
        let mut iv = [0; GCM_IV_12];
        iv.copy_from_slice(src);
        Iv { iv }
    }
}

impl Iv {
    /// Generate a nonce and a counter randomly.
    pub fn new() -> Iv {
        use rand::{rngs::OsRng, RngCore};

        let mut iv = [0u8; GCM_IV_12];
        OsRng.fill_bytes(&mut iv);

        Iv { iv }
    }
}

// The length GCM tag must be 16 btyes.
const GCM_TAG_LEN: usize = 16;

pub struct AesGcmTag([u8; GCM_TAG_LEN]);

impl<'a> From<&'a [u8]> for AesGcmTag {
    fn from(src: &'a [u8]) -> AesGcmTag {
        assert!(src.len() >= GCM_TAG_LEN, "AES GCM tag must be 16 bytes");
        let mut tag = [0; GCM_TAG_LEN];
        tag.copy_from_slice(src);
        AesGcmTag(tag)
    }
}

impl AesGcmTag {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

/// An Aes256-GCM crypter.
pub struct AesGcmCrypter<'k> {
    iv: Iv,
    key: &'k [u8],
}

impl<'k> AesGcmCrypter<'k> {
    /// The key length of `AesGcmCrypter` is 32 bytes.
    pub const KEY_LEN: usize = 32;

    pub fn new(key: &'k [u8], iv: Iv) -> AesGcmCrypter<'k> {
        AesGcmCrypter { iv, key }
    }

    pub fn encrypt(&self, pt: &[u8]) -> Result<(Vec<u8>, AesGcmTag)> {
        let cipher = OCipher::aes_256_gcm();
        let mut tag = AesGcmTag([0u8; GCM_TAG_LEN]);
        let ciphertext = symm::encrypt_aead(
            cipher,
            self.key,
            Some(self.iv.as_slice()),
            &[], /* AAD */
            &pt,
            &mut tag.0,
        )?;
        Ok((ciphertext, tag))
    }

    pub fn decrypt(&self, ct: &[u8], tag: AesGcmTag) -> Result<Vec<u8>> {
        let cipher = OCipher::aes_256_gcm();
        let plaintext = symm::decrypt_aead(
            cipher,
            self.key,
            Some(self.iv.as_slice()),
            &[], /* AAD */
            &ct,
            &tag.0,
        )?;
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use hex::FromHex;

    use super::*;

    #[test]
    fn test_iv() {
        let mut ivs = Vec::with_capacity(100);
        for _ in 0..100 {
            ivs.push(Iv::new());
        }
        ivs.dedup_by(|a, b| a.as_slice() == b.as_slice());
        assert_eq!(ivs.len(), 100);

        for iv in ivs {
            let iv1 = Iv::from(&iv.as_slice()[..]);
            assert_eq!(iv.as_slice(), iv1.as_slice());
        }
    }

    #[test]
    fn test_ase_256_gcm() {
        // See more http://csrc.nist.gov/groups/STM/cavp/documents/mac/gcmtestvectors.zip
        //
        // [Keylen = 256]
        // [IVlen = 96]
        // [PTlen = 256]
        // [AADlen = 0]
        // [Taglen = 128]
        //
        // Count = 0
        // Key = c3d99825f2181f4808acd2068eac7441a65bd428f14d2aab43fefc0129091139
        // IV = cafabd9672ca6c79a2fbdc22
        // CT = 84e5f23f95648fa247cb28eef53abec947dbf05ac953734618111583840bd980
        // AAD =
        // Tag = 79651c875f7941793d42bbd0af1cce7c
        // PT = 25431587e9ecffc7c37f8d6d52a9bc3310651d46fb0e3bad2726c8f2db653749

        let pt = "25431587e9ecffc7c37f8d6d52a9bc3310651d46fb0e3bad2726c8f2db653749";
        let ct = "84e5f23f95648fa247cb28eef53abec947dbf05ac953734618111583840bd980";
        let key = "c3d99825f2181f4808acd2068eac7441a65bd428f14d2aab43fefc0129091139";
        let iv = "cafabd9672ca6c79a2fbdc22";
        let tag = "79651c875f7941793d42bbd0af1cce7c";

        let pt = Vec::from_hex(pt).unwrap();
        let ct = Vec::from_hex(ct).unwrap();
        let key = Vec::from_hex(key).unwrap();
        let iv = Vec::from_hex(iv).unwrap().as_slice().into();
        let tag = Vec::from_hex(tag).unwrap();

        let crypter = AesGcmCrypter::new(&key, iv);
        let (ciphertext, gcm_tag) = crypter.encrypt(&pt).unwrap();
        assert_eq!(ciphertext, ct, "{}", hex::encode(&ciphertext));
        assert_eq!(gcm_tag.0.to_vec(), tag, "{}", hex::encode(&gcm_tag.0));
        let plaintext = crypter.decrypt(&ct, gcm_tag).unwrap();
        assert_eq!(plaintext, pt, "{}", hex::encode(&plaintext));

        // Fail to decrypt with a wrong tag.
        crypter
            .decrypt(&ct, AesGcmTag([0u8; GCM_TAG_LEN]))
            .unwrap_err();
    }
}
