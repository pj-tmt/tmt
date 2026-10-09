//! Version-2 correlation preimage. Field order and lengths are wire protocol.
//! This unkeyed digest is not authentication and cannot add entropy to weak IDs.

use super::RequestRoute;
use sha2::{Digest, Sha256};

pub fn response_token(request_id: &str, attempt_id: &str, route: &RequestRoute) -> [u8; 16] {
    let mut digest = Sha256::new();
    digest.update(b"tmt/reply-receipt/v2\0");
    append_string(&mut digest, request_id);
    append_string(&mut digest, attempt_id);
    match route {
        RequestRoute::Pane(endpoint) => {
            append_string(&mut digest, &endpoint.server.server_id);
            append_string(&mut digest, &endpoint.server.socket_path);
            digest.update(endpoint.server.server_pid.to_be_bytes());
            append_string(&mut digest, &endpoint.server.server_start_time);
            append_string(&mut digest, &endpoint.pane_id);
            digest.update(endpoint.pane_pid.to_be_bytes());
        }
        RequestRoute::Inbox {
            recipient_identity_id,
        } => {
            digest.update(b"inbox\0");
            append_string(&mut digest, recipient_identity_id);
        }
    }
    let hash = digest.finalize();
    let mut token = [0; 16];
    token.copy_from_slice(&hash[..16]);
    token
}

fn append_string(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inbox_receipt_uses_only_the_current_domain() {
        let route = RequestRoute::Inbox {
            recipient_identity_id: "recipient".into(),
        };
        let current = response_token("request", "attempt", &route);
        let mut old = Sha256::new();
        old.update(b"tmux-team/reply-receipt/v2\0");
        append_string(&mut old, "request");
        append_string(&mut old, "attempt");
        old.update(b"inbox\0");
        append_string(&mut old, "recipient");
        assert_ne!(current.as_slice(), &old.finalize()[..16]);
        let mut expected = Sha256::new();
        expected.update(b"tmt/reply-receipt/v2\0");
        append_string(&mut expected, "request");
        append_string(&mut expected, "attempt");
        expected.update(b"inbox\0");
        append_string(&mut expected, "recipient");
        assert_eq!(current.as_slice(), &expected.finalize()[..16]);
    }
}
