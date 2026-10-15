use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Huawei Cloud SDK signing algorithm identifier.
pub const SIGNING_ALGORITHM: &str = "SDK-HMAC-SHA256";

/// SHA256 of an empty body (Huawei SDK constant).
pub const EMPTY_BODY_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

type HmacSha256 = Hmac<Sha256>;

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn hmac_sha256(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

/// Huawei urlEncode: keeps A-Z a-z 0-9 - _ . ~ +, percent-encodes the rest
/// (uppercase hex), matching the table recovered from the official CLI.
pub fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'+' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

/// Canonical URI: split on '/', escape each segment, rejoin, force trailing '/'.
pub fn canonical_uri_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }
    let encoded: Vec<String> = path.split('/').map(url_escape).collect();
    let mut joined = encoded.join("/");
    if !joined.ends_with('/') {
        joined.push('/');
    }
    joined
}

/// Canonical query string: sort keys, escape key and value, join with '&'.
pub fn canonical_query(raw_query: &str) -> String {
    if raw_query.is_empty() {
        return String::new();
    }
    let parsed = url::form_urlencoded::parse(raw_query.as_bytes());
    let mut pairs: Vec<(String, String)> =
        parsed.map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
        .iter()
        .map(|(k, v)| {
            format!(
                "{}={}",
                url::form_urlencoded::byte_serialize(k.as_bytes()).collect::<String>(),
                url::form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Result of signing: the X-Sdk-Date value and the Authorization header value.
pub struct Signature {
    pub x_sdk_date: String,
    pub authorization: String,
}

/// Sign a request using the Huawei Cloud SDK-HMAC-SHA256 scheme.
///
/// `signed_headers` must contain (lowercase-name, trimmed-value) pairs sorted
/// by name, excluding `authorization`. The caller is responsible for setting
/// X-Sdk-Date / X-Sdk-Content-Sha256 / Host headers before building this list.
pub fn sign(
    ak: &str,
    sk: &str,
    method: &str,
    uri_path: &str,
    raw_query: &str,
    signed_headers: &[(String, String)],
    payload_hash: &str,
    x_sdk_date: &str,
) -> Signature {
    let canonical_headers: String = signed_headers
        .iter()
        .map(|(name, value)| format!("{}:{}\n", name, value))
        .collect();
    let signed_names: String = signed_headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method,
        canonical_uri_path(uri_path),
        canonical_query(raw_query),
        canonical_headers,
        signed_names,
        payload_hash,
    );

    let string_to_sign = format!(
        "{}\n{}\n{}",
        SIGNING_ALGORITHM,
        x_sdk_date,
        sha256_hex(canonical_request.as_bytes()),
    );

    let signature = hex::encode(hmac_sha256(sk.as_bytes(), &string_to_sign));
    let authorization = format!(
        "{} Access={}, SignedHeaders={}, Signature={}",
        SIGNING_ALGORITHM, ak, signed_names, signature,
    );

    Signature {
        x_sdk_date: x_sdk_date.to_string(),
        authorization,
    }
}

/// Current UTC timestamp in the X-Sdk-Date format (YYYYMMDDTHHMMSSZ).
pub fn sdk_date_now() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}
