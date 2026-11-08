use sha2::{Digest, Sha512};

use crate::Result;
use crate::library::random_bytes;

const ROUNDS: usize = 5000;
const SALT_LENGTH: usize = 16;
const ALPHABET: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ORDER: [(usize, usize, usize); 21] = [
    (0, 21, 42),
    (22, 43, 1),
    (44, 2, 23),
    (3, 24, 45),
    (25, 46, 4),
    (47, 5, 26),
    (6, 27, 48),
    (28, 49, 7),
    (50, 8, 29),
    (9, 30, 51),
    (31, 52, 10),
    (53, 11, 32),
    (12, 33, 54),
    (34, 55, 13),
    (56, 14, 35),
    (15, 36, 57),
    (37, 58, 16),
    (59, 17, 38),
    (18, 39, 60),
    (40, 61, 19),
    (62, 20, 41),
];

fn repeated(digest: &[u8], length: usize) -> Vec<u8> {
    digest.iter().copied().cycle().take(length).collect()
}

fn encode(output: &mut String, bytes: [u8; 3], count: usize) {
    let mut word = u32::from(bytes[0]) << 16 | u32::from(bytes[1]) << 8 | u32::from(bytes[2]);
    for _ in 0..count {
        output.push(char::from(ALPHABET[(word & 0x3f) as usize]));
        word >>= 6;
    }
}

pub fn sha512_crypt(password: &str, salt: &str) -> String {
    let password = password.as_bytes();
    let salt = &salt.as_bytes()[..salt.len().min(SALT_LENGTH)];
    let alternate = Sha512::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(password)
        .finalize();
    let mut context = Sha512::new()
        .chain_update(password)
        .chain_update(salt)
        .chain_update(repeated(&alternate, password.len()));
    let mut length = password.len();
    while length > 0 {
        match length & 1 {
            1 => context.update(alternate),
            _ => context.update(password),
        }
        length >>= 1;
    }
    let mut current = context.finalize();
    let mut password_digest = Sha512::new();
    for _ in 0..password.len() {
        password_digest.update(password);
    }
    let password_bytes = repeated(&password_digest.finalize(), password.len());
    let mut salt_digest = Sha512::new();
    for _ in 0..16 + usize::from(current[0]) {
        salt_digest.update(salt);
    }
    let salt_bytes = repeated(&salt_digest.finalize(), salt.len());
    for round in 0..ROUNDS {
        let mut context = Sha512::new();
        match round % 2 {
            1 => context.update(&password_bytes),
            _ => context.update(current),
        }
        if round % 3 != 0 {
            context.update(&salt_bytes);
        }
        if round % 7 != 0 {
            context.update(&password_bytes);
        }
        match round % 2 {
            1 => context.update(current),
            _ => context.update(&password_bytes),
        }
        current = context.finalize();
    }
    let mut output = format!("$6${}$", String::from_utf8_lossy(salt));
    for (first, second, third) in ORDER {
        encode(
            &mut output,
            [current[first], current[second], current[third]],
            4,
        );
    }
    encode(&mut output, [0, 0, current[63]], 2);
    output
}

pub(crate) fn hash_password(password: &str) -> Result<String> {
    let salt: String = random_bytes::<SALT_LENGTH>()?
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 0x3f)]))
        .collect();
    Ok(sha512_crypt(password, &salt))
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_glibc_sha512_crypt() {
        assert_eq!(
            super::sha512_crypt("Hello world!", "saltstring"),
            "$6$saltstring$svn8UoSVapNtMuq1ukKS4tPQd8iKwSMHWjl/O817G3uBnIFNjnQJuesI68u4OTLiBFdcbYEdFCoEOfaS35inz1"
        );
    }
}
