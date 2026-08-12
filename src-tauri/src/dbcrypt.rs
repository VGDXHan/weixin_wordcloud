//! Self-implemented SQLCipher 4 decryption for WeChat 4.x databases.
//!
//! `wx_key.dll` gives us WeChat's account-level database key. WeChat 4.x derives
//! each DB's AES key from that key plus the DB's own salt (PBKDF2-SHA512,
//! 256,000 iterations), then derives a separate 32-byte HMAC sub-key. We do all
//! of that here and write a plaintext SQLite copy that bundled SQLite can open.
//! No dependency on WeFlow's protected wcdb_api.dll.
//!
//! WeChat builds vary slightly in cipher params, so `detect` brute-forces the
//! small parameter space against page 1's HMAC and returns the working config.

use crate::error::{AppError, AppResult};
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::{Sha256, Sha512};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

fn dbg_log(msg: &str) {
    eprintln!("{msg}");
    #[cfg(debug_assertions)]
    {
        let path = std::env::temp_dir().join("weixin_wordcloud_debug.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

const PAGE_SIZE: usize = 4096;
const IV_LEN: usize = 16;
const SALT_LEN: usize = 16;
/// SQLCipher always derives a key-sized (AES-256 => 32-byte) MAC sub-key.
/// The SHA-512 digest is 64 bytes, but that does *not* make the MAC key 64 bytes.
const HMAC_KEY_LEN: usize = 32;
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug, Clone, Copy, PartialEq)]
enum HmacAlgo {
    Sha512,
    Sha256,
}

/// Resolved, working cipher configuration for an account's databases.
#[derive(Clone)]
pub struct Cipher {
    enc_key: [u8; 32],
    hmac_key: Vec<u8>,
    hmac_len: usize,
    reserve: usize,
    algo: HmacAlgo,
    pageno_be: bool,
}

fn pbkdf2_sha512(pw: &[u8], salt: &[u8], iters: u32, out_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; out_len];
    pbkdf2::pbkdf2_hmac::<Sha512>(pw, salt, iters, &mut out);
    out
}

fn hmac_salt(salt: &[u8]) -> Vec<u8> {
    salt.iter().map(|b| b ^ 0x3a).collect()
}

fn read_page1(db_path: &Path) -> AppResult<[u8; PAGE_SIZE]> {
    let mut file = std::fs::File::open(db_path)?;
    let size = file.metadata()?.len();
    if size < PAGE_SIZE as u64 {
        return Err(AppError::Decrypt(format!(
            "{} 数据库过小（{size} 字节）",
            db_path.display()
        )));
    }
    let mut page = [0u8; PAGE_SIZE];
    file.read_exact(&mut page)?;
    Ok(page)
}

fn page_hmac(algo: HmacAlgo, key: &[u8], data: &[u8], pgno: u32, be: bool) -> Vec<u8> {
    let pno = if be { pgno.to_be_bytes() } else { pgno.to_le_bytes() };
    match algo {
        HmacAlgo::Sha512 => {
            let mut m = <Hmac<Sha512> as Mac>::new_from_slice(key).unwrap();
            m.update(data);
            m.update(&pno);
            m.finalize().into_bytes().to_vec()
        }
        HmacAlgo::Sha256 => {
            let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap();
            m.update(data);
            m.update(&pno);
            m.finalize().into_bytes().to_vec()
        }
    }
}

fn aes_cbc_decrypt(key: &[u8; 32], iv: &[u8], ct: &[u8]) -> Vec<u8> {
    let mut buf = ct.to_vec();
    let mut dec = Aes256CbcDec::new_from_slices(key, iv).expect("aes key/iv");
    for chunk in buf.chunks_mut(16) {
        let block = cbc::cipher::generic_array::GenericArray::from_mut_slice(chunk);
        dec.decrypt_block_mut(block);
    }
    buf
}

/// Detect the working cipher params by validating page 1's HMAC.
pub fn detect(db_path: &Path, raw_key: &[u8; 32]) -> AppResult<Cipher> {
    let page1 = read_page1(db_path)?;
    let salt = &page1[..SALT_LEN];
    let hsalt = hmac_salt(salt);
    dbg_log(&format!(
        "[detect] db={} salt={}",
        db_path.display(),
        hex::encode(salt)
    ));

    let key_variants: Vec<[u8; 32]> = {
        // wx_key.dll returns WeChat's account-level database key. WeChat 4.x
        // normally derives each DB's AES key from it and that DB's own salt.
        let derived = pbkdf2_sha512(raw_key, salt, 256_000, 32);
        let mut d = [0u8; 32];
        d.copy_from_slice(&derived);
        // Try the canonical WeChat 4.x path first; the direct-key variant keeps
        // compatibility with memory scanners that return the per-DB AES key.
        vec![d, *raw_key]
    };

    for algo in [HmacAlgo::Sha512, HmacAlgo::Sha256] {
        let hmac_len = match algo {
            HmacAlgo::Sha512 => 64,
            HmacAlgo::Sha256 => 32,
        };
        // reserve = IV + HMAC, rounded up to AES block multiple.
        let mut reserve = IV_LEN + hmac_len;
        if reserve % 16 != 0 {
            reserve += 16 - (reserve % 16);
        }
        let hmac_region = &page1[SALT_LEN..PAGE_SIZE - reserve + IV_LEN];
        let stored = &page1[PAGE_SIZE - reserve + IV_LEN..PAGE_SIZE - reserve + IV_LEN + hmac_len];

        for enc_key in &key_variants {
            for fast_iter in [2u32, 256_000, 64_000, 1] {
                // SQLCipher derives a 32-byte MAC key for AES-256, regardless
                // of whether the selected HMAC digest is SHA-512 or SHA-256.
                let hmac_key = pbkdf2_sha512(enc_key, &hsalt, fast_iter, HMAC_KEY_LEN);
                for be in [false, true] {
                    let calc = page_hmac(algo, &hmac_key, hmac_region, 1, be);
                    if calc == stored {
                        dbg_log(&format!(
                            "[detect] MATCH algo={algo:?} derived={} fast_iter={fast_iter} pageno_be={be} reserve={reserve}",
                            enc_key != raw_key
                        ));
                        return Ok(Cipher {
                            enc_key: *enc_key,
                            hmac_key,
                            hmac_len,
                            reserve,
                            algo,
                            pageno_be: be,
                        });
                    }
                }
            }
        }
    }

    dbg_log(&format!(
        "[detect] NO MATCH. stored_hmac_head(page1)={}",
        hex::encode(&page1[PAGE_SIZE - 64..PAGE_SIZE - 64 + 16])
    ));
    Err(AppError::KeyMismatch)
}

/// Decrypt a whole SQLCipher DB into a plaintext SQLite file in `out_dir`.
pub fn decrypt_to(db_path: &Path, c: &Cipher, out_dir: &Path) -> AppResult<PathBuf> {
    let input = std::fs::File::open(db_path)?;
    let size = input.metadata()?.len();
    if size < PAGE_SIZE as u64 || size % PAGE_SIZE as u64 != 0 {
        return Err(AppError::Decrypt(format!(
            "{} 大小非法({} 字节)",
            db_path.display(),
            size
        )));
    }

    std::fs::create_dir_all(out_dir)?;
    let stem = db_path.file_name().and_then(|s| s.to_str()).unwrap_or("db");
    let out_path = out_dir.join(format!("dec_{stem}"));
    let tmp_path = out_dir.join(format!("dec_{stem}.tmp"));
    let _ = std::fs::remove_file(&tmp_path);

    let result: AppResult<()> = (|| {
        let page_count = size / PAGE_SIZE as u64;
        let mut reader = BufReader::new(input);
        let mut writer = BufWriter::new(std::fs::File::create(&tmp_path)?);
        let mut page = [0u8; PAGE_SIZE];
        let zero_reserve = vec![0u8; c.reserve];

        for i in 0..page_count {
            reader.read_exact(&mut page)?;
            let pgno = (i + 1) as u32;
            let start = if i == 0 { SALT_LEN } else { 0 };

            let region = &page[start..PAGE_SIZE - c.reserve + IV_LEN];
            let stored = &page
                [PAGE_SIZE - c.reserve + IV_LEN..PAGE_SIZE - c.reserve + IV_LEN + c.hmac_len];
            if page_hmac(c.algo, &c.hmac_key, region, pgno, c.pageno_be) != stored {
                // A blank/unused trailing page can appear; preserve it as zero.
                if page.iter().all(|&b| b == 0) {
                    writer.write_all(&page)?;
                    continue;
                }
                return Err(AppError::Decrypt(format!(
                    "{} 第 {pgno} 页 HMAC 校验失败",
                    db_path.display()
                )));
            }

            let iv = &page[PAGE_SIZE - c.reserve..PAGE_SIZE - c.reserve + IV_LEN];
            let ct = &page[start..PAGE_SIZE - c.reserve];
            let plain = aes_cbc_decrypt(&c.enc_key, iv, ct);

            if i == 0 {
                writer.write_all(SQLITE_HEADER)?;
            }
            writer.write_all(&plain)?;
            // Plain SQLite only needs the reserved bytes to exist; retaining
            // the encrypted IV/HMAC would leak unnecessary crypto metadata.
            writer.write_all(&zero_reserve)?;
        }
        writer.flush()?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::remove_file(&out_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::Other(format!(
                "无法替换旧的解密文件 {}：{error}",
                out_path.display()
            )));
        }
    }
    if let Err(error) = std::fs::rename(&tmp_path, &out_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Other(format!(
            "无法保存解密文件 {}：{error}",
            out_path.display()
        )));
    }
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sqlcipher4_with_a_32_byte_hmac_key() {
        let enc_key = [0x22; 32];
        let salt = [0x11; SALT_LEN];
        let reserve = IV_LEN + 64;
        let mut page = vec![0u8; PAGE_SIZE];
        page[..SALT_LEN].copy_from_slice(&salt);
        for (i, byte) in page[SALT_LEN..PAGE_SIZE - reserve + IV_LEN]
            .iter_mut()
            .enumerate()
        {
            *byte = (i % 251) as u8;
        }

        let mac_key = pbkdf2_sha512(&enc_key, &hmac_salt(&salt), 2, HMAC_KEY_LEN);
        let digest = page_hmac(
            HmacAlgo::Sha512,
            &mac_key,
            &page[SALT_LEN..PAGE_SIZE - reserve + IV_LEN],
            1,
            false,
        );
        page[PAGE_SIZE - 64..].copy_from_slice(&digest);

        let dir = std::env::temp_dir().join(format!(
            "weixin_wordcloud_dbcrypt_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("page1.db");
        std::fs::write(&db, page).unwrap();

        let cipher = detect(&db, &enc_key).unwrap();
        assert_eq!(cipher.algo, HmacAlgo::Sha512);
        assert_eq!(cipher.hmac_key.len(), HMAC_KEY_LEN);
        assert_eq!(cipher.hmac_len, 64);
        assert_eq!(cipher.reserve, 80);
        assert!(!cipher.pageno_be);

        let decrypted = decrypt_to(&db, &cipher, &dir.join("out")).unwrap();
        let plaintext = std::fs::read(decrypted).unwrap();
        assert_eq!(plaintext.len(), PAGE_SIZE);
        assert_eq!(&plaintext[..SQLITE_HEADER.len()], SQLITE_HEADER);
        assert!(plaintext[PAGE_SIZE - reserve..].iter().all(|byte| *byte == 0));

        let _ = std::fs::remove_dir_all(dir);
    }
}
