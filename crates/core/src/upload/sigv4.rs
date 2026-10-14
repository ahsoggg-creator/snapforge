//! AWS Signature Version 4 (header-based) for S3-compatible storage. Only what a single-object PUT needs.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = <HmacSha256 as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// RFC 3986 percent-encoding as AWS wants it: unreserved characters stay, everything else (including UTF-8 bytes)
/// becomes `%XX`; `/` stays only when encoding a path.
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub struct SignInput<'a> {
    pub method: &'a str,
    /// Already URI-encoded path, e.g. `/bucket/my%20file.png`.
    pub canonical_uri: &'a str,
    /// Already sorted and encoded `a=1&b=2`, or empty.
    pub canonical_query: &'a str,
    /// `(lower-case name, trimmed value)`; the signer sorts them.
    pub headers: &'a [(String, String)],
    pub payload_sha256: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    /// `YYYYMMDDTHHMMSSZ`
    pub amz_date: &'a str,
}

pub struct Signature {
    pub authorization: String,
    pub signature: String,
    pub signed_headers: String,
}

pub fn sign(i: &SignInput) -> Signature {
    let mut headers: Vec<&(String, String)> = i.headers.iter().collect();
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = headers.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>().join(";");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        i.method, i.canonical_uri, i.canonical_query, canonical_headers, signed_headers, i.payload_sha256
    );
    let date = &i.amz_date[..8];
    let scope = format!("{date}/{}/{}/aws4_request", i.region, i.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        i.amz_date,
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac(format!("AWS4{}", i.secret_key).as_bytes(), date.as_bytes());
    let k_region = hmac(&k_date, i.region.as_bytes());
    let k_service = hmac(&k_region, i.service.as_bytes());
    let k_signing = hmac(&k_service, b"aws4_request");
    let signature = hex(&hmac(&k_signing, string_to_sign.as_bytes()));
    Signature {
        authorization: format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            i.access_key
        ),
        signature,
        signed_headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(k: &str, v: &str) -> (String, String) {
        (k.to_owned(), v.to_owned())
    }

    /// The worked example from the AWS documentation ("Signature Version 4 signing process", IAM ListUsers).
    #[test]
    fn matches_the_aws_documentation_example_for_iam() {
        let headers = [
            h("content-type", "application/x-www-form-urlencoded; charset=utf-8"),
            h("host", "iam.amazonaws.com"),
            h("x-amz-date", "20150830T123600Z"),
        ];
        let s = sign(&SignInput {
            method: "GET",
            canonical_uri: "/",
            canonical_query: "Action=ListUsers&Version=2010-05-08",
            headers: &headers,
            payload_sha256: &sha256_hex(b""),
            access_key: "AKIDEXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "iam",
            amz_date: "20150830T123600Z",
        });
        assert_eq!(
            s.signature,
            "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
        assert_eq!(s.signed_headers, "content-type;host;x-amz-date");
    }

    /// The "PUT Object" example from the Amazon S3 documentation (header-based authentication).
    #[test]
    fn matches_the_aws_documentation_example_for_s3_put() {
        let body = b"Welcome to Amazon S3.";
        assert_eq!(
            sha256_hex(body),
            "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072"
        );
        let headers = [
            h("date", "Fri, 24 May 2013 00:00:00 GMT"),
            h("host", "examplebucket.s3.amazonaws.com"),
            h("x-amz-content-sha256", &sha256_hex(body)),
            h("x-amz-date", "20130524T000000Z"),
            h("x-amz-storage-class", "REDUCED_REDUNDANCY"),
        ];
        let s = sign(&SignInput {
            method: "PUT",
            canonical_uri: &uri_encode("/test$file.text", true),
            canonical_query: "",
            headers: &headers,
            payload_sha256: &sha256_hex(body),
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "s3",
            amz_date: "20130524T000000Z",
        });
        assert_eq!(
            s.signature,
            "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd"
        );
        assert!(s.authorization.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=date;host;x-amz-content-sha256;x-amz-date;x-amz-storage-class, Signature="
        ));
    }

    #[test]
    fn uri_encoding_follows_the_aws_rules() {
        assert_eq!(uri_encode("a b/ü.png", true), "a%20b/%C3%BC.png");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(uri_encode("keep-_.~AZaz09", true), "keep-_.~AZaz09");
    }
}
