// SPDX-License-Identifier: GPL-3.0-only
//! AWS's published Signature Version 4 examples.
use loom_remote::sigv4::{authorization, canonical_query, sha256_hex, Credentials, Scope, EMPTY_SHA256};

fn sig(a: &str) -> &str { a.rsplit("Signature=").next().unwrap() }

/// aws-sig-v4-test-suite (service "service", region us-east-1, 2015-08-30T12:36:00Z).
#[test]
fn signature_v4_test_suite() {
    let c = Credentials::new("AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY");
    let s = Scope { amz_date: "20150830T123600Z", region: "us-east-1", service: "service" };
    let h = [("Host", "example.amazonaws.com"), ("X-Amz-Date", "20150830T123600Z")];
    // get-vanilla
    let a = authorization(&c, &s, "GET", "/", "", &h, EMPTY_SHA256);
    assert_eq!(a, "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31");
    // get-vanilla-query-order-key-case: ?Param2=value2&Param1=value1
    let q = canonical_query(&[("Param2", "value2"), ("Param1", "value1")]);
    assert_eq!(q, "Param1=value1&Param2=value2");
    assert_eq!(sig(&authorization(&c, &s, "GET", "/", &q, &h, EMPTY_SHA256)), "b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500");
    // post-vanilla
    assert_eq!(sig(&authorization(&c, &s, "POST", "/", "", &h, EMPTY_SHA256)), "5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b");
}

/// The S3 examples in "Signature Calculations for the Authorization Header"
/// (examplebucket, 2013-05-24T00:00:00Z).
#[test]
fn s3_documentation_examples() {
    let c = Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
    let s = Scope { amz_date: "20130524T000000Z", region: "us-east-1", service: "s3" };
    let host = ("host", "examplebucket.s3.amazonaws.com");
    let date = ("x-amz-date", "20130524T000000Z");
    // GET object with Range.
    let a = authorization(&c, &s, "GET", "/test.txt", "", &[host, ("range", "bytes=0-9"), ("x-amz-content-sha256", EMPTY_SHA256), date], EMPTY_SHA256);
    assert_eq!(a, "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41");
    // PUT object.
    let body = sha256_hex(b"Welcome to Amazon S3.");
    assert_eq!(body, "44ce7dd67c959e0d3524ffac1771dfbba87d2b6b4b4e99e42034a8b803f8b072");
    let a = authorization(&c, &s, "PUT", "/test%24file.text", "", &[("date", "Fri, 24 May 2013 00:00:00 GMT"), host, ("x-amz-content-sha256", &body), date, ("x-amz-storage-class", "REDUCED_REDUNDANCY")], &body);
    assert_eq!(sig(&a), "98ad721746da40c64f1a55b78f14c238d841ea1380cd77a1b5971af0ece108bd");
    // GET bucket lifecycle.
    let a = authorization(&c, &s, "GET", "/", &canonical_query(&[("lifecycle", "")]), &[host, ("x-amz-content-sha256", EMPTY_SHA256), date], EMPTY_SHA256);
    assert_eq!(sig(&a), "fea454ca298b7da1c68078a5d1bdbfbbe0d65c699e0f91ac7a200a0136783543");
    // List objects.
    let a = authorization(&c, &s, "GET", "/", &canonical_query(&[("max-keys", "2"), ("prefix", "J")]), &[host, ("x-amz-content-sha256", EMPTY_SHA256), date], EMPTY_SHA256);
    assert_eq!(sig(&a), "34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7");
}

#[test]
fn credentials_never_show_the_secret() {
    let c = Credentials::new("AKID", "very-secret").with_token("tok-secret");
    let d = format!("{c:?}");
    assert!(d.contains("AKID") && !d.contains("secret"), "{d}");
    let a = loom_remote::webdav::Auth::Basic { user: "alice".into(), password: "hunter2".into() };
    assert!(!format!("{a:?}").contains("hunter2"));
}
