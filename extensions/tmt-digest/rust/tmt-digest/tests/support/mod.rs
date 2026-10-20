//! Shared by every integration test: the one place a run of the Digest binary
//! starts, so no test can reach the network by leaving the price fetch on.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

const PRICES: &str = "TMT_DIGEST_PRICING_URL";

/// The Digest binary with the price fetch off. A test that exercises the fetch
/// points `TMT_DIGEST_PRICING_URL` at a [`PriceServer`] after calling this.
pub fn digest() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmt-digest"));
    command.env(PRICES, "off");
    command
}

/// What a [`PriceServer`] answers every request with.
#[allow(dead_code)] // Each test crate uses a subset of the support code.
pub enum Reply {
    Body(Vec<u8>),
    /// Accept the connection and never answer.
    Stall,
}

/// A loopback server standing in for the public raw URL.
#[allow(dead_code)]
pub struct PriceServer {
    pub url: String,
    address: SocketAddr,
    hits: Arc<AtomicUsize>,
    stopped: Arc<AtomicBool>,
}

#[allow(dead_code)]
impl PriceServer {
    pub fn serve(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicBool::new(false));
        let (seen, done) = (hits.clone(), stopped.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if done.load(Ordering::SeqCst) {
                    return;
                }
                seen.fetch_add(1, Ordering::SeqCst);
                let mut stream = stream;
                let mut request = [0u8; 4096];
                let _ = stream.read(&mut request);
                match &reply {
                    Reply::Body(body) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(body);
                    }
                    Reply::Stall => std::thread::sleep(Duration::from_secs(6)),
                }
            }
        });
        Self {
            url: format!("http://{address}/pricing-v1.json"),
            address,
            hits,
            stopped,
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for PriceServer {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.address);
    }
}
