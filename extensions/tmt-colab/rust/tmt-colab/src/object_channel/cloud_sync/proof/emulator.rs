//! The proof's own reads and writes against the loopback emulators: Admin-bypass reads of what
//! the serves put in Firestore, a fixture member's sign-up and writes, and Admin writes of
//! documents Rules would refuse. Only the emulators are ever addressed.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

pub(super) struct Emulator {
    pub project: String,
    firestore: String,
    auth: String,
}
/// One member's Auth identity from the Auth emulator.
pub(super) struct Identity {
    pub uid: String,
    pub token: String,
}
fn http(
    host: &str,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<&Value>,
) -> (u16, Value) {
    let mut stream = TcpStream::connect(host).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let payload = body.map(Value::to_string).unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\n"
    );
    if let Some(token) = bearer {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    request.push_str(&format!(
        "Content-Length: {}\r\n\r\n{payload}",
        payload.len()
    ));
    stream.write_all(request.as_bytes()).unwrap();
    // The server closes after one answer; a timeout here is a real failure, not an end.
    let mut response = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&chunk[..n]),
            Err(error) if !response.is_empty() && complete(&response) => {
                let _ = error;
                break;
            }
            Err(error) => panic!("emulator read failed: {error}"),
        }
        if complete(&response) {
            break;
        }
    }
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response head");
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let raw = &response[split + 4..];
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(raw)
    } else {
        raw.to_vec()
    };
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}
/// Whether the bytes read so far hold a whole answer: the head and every promised body byte.
fn complete(response: &[u8]) -> bool {
    let Some(split) = response.windows(4).position(|w| w == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    let body = &response[split + 4..];
    if head.contains("transfer-encoding: chunked") {
        return body.ends_with(b"0\r\n\r\n");
    }
    head.lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .is_some_and(|n| body.len() >= n)
}
fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(line_end) = raw.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&raw[..line_end])
                .trim()
                .split(';')
                .next()
                .unwrap(),
            16,
        )
        .unwrap_or(0);
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&raw[line_end + 2..line_end + 2 + size]);
        raw = &raw[line_end + 2 + size + 2..];
    }
}
impl Emulator {
    pub(super) fn new(inputs: &super::world::Inputs) -> Self {
        Self {
            project: inputs.project.clone(),
            firestore: inputs.firestore.clone(),
            auth: inputs.auth.clone(),
        }
    }
    fn documents(&self) -> String {
        format!(
            "/v1/projects/{}/databases/(default)/documents",
            self.project
        )
    }
    /// The documents of one `x/colab/<collection>`, by Admin bypass, in the provider's order.
    pub(super) fn list(&self, collection: &str) -> Vec<Value> {
        let mut found = Vec::new();
        let mut token = String::new();
        loop {
            let path = format!(
                "{}/x/colab/{collection}?pageSize=300{token}",
                self.documents()
            );
            let (status, body) = http(&self.firestore, "GET", &path, Some("owner"), None);
            assert!(status == 200, "list {collection}: {status} {body}");
            found.extend(body["documents"].as_array().cloned().unwrap_or_default());
            match body["nextPageToken"].as_str() {
                Some(next) => token = format!("&pageToken={next}"),
                None => return found,
            }
        }
    }
    /// The collections that exist under `x/colab`.
    pub(super) fn collections(&self) -> Vec<String> {
        let path = format!("{}/x/colab:listCollectionIds", self.documents());
        let (_, body) = http(
            &self.firestore,
            "POST",
            &path,
            Some("owner"),
            Some(&json!({})),
        );
        body["collectionIds"]
            .as_array()
            .map(|ids| ids.iter().map(|i| i.as_str().unwrap().to_owned()).collect())
            .unwrap_or_default()
    }
    /// The last path segment of every document of a collection.
    pub(super) fn ids(&self, collection: &str) -> Vec<String> {
        self.list(collection)
            .iter()
            .map(|d| {
                d["name"]
                    .as_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    }
    /// A new anonymous Auth identity, as a fixture member's.
    pub(super) fn sign_up(&self) -> Identity {
        let (status, body) = http(
            &self.auth,
            "POST",
            "/identitytoolkit.googleapis.com/v1/accounts:signUp?key=fake-key",
            None,
            Some(&json!({"returnSecureToken": true})),
        );
        assert_eq!(status, 200, "sign-up: {body}");
        Identity {
            uid: body["localId"].as_str().unwrap().to_owned(),
            token: body["idToken"].as_str().unwrap().to_owned(),
        }
    }
    /// Create one document the crate built (typed fields and its own ID), as an identity (Rules
    /// apply) or by Admin bypass. Returns the HTTP status.
    pub(super) fn put(
        &self,
        document: &tmt_extension_objects::sync::Document,
        as_identity: Option<&Identity>,
    ) -> u16 {
        self.create(
            &document.target.resource,
            &document.target.id,
            &Value::Object(document.fields.clone()),
            as_identity,
        )
    }
    /// Create one document with typed fields (`fields` is Firestore REST JSON), either as an
    /// identity (Rules apply) or by Admin bypass. Returns the HTTP status.
    pub(super) fn create(
        &self,
        collection: &str,
        id: &str,
        fields: &Value,
        as_identity: Option<&Identity>,
    ) -> u16 {
        let path = format!("{}/x/colab/{collection}?documentId={id}", self.documents());
        let bearer = as_identity.map_or("owner", |i| i.token.as_str());
        http(
            &self.firestore,
            "POST",
            &path,
            Some(bearer),
            Some(&json!({ "fields": fields })),
        )
        .0
    }
}
