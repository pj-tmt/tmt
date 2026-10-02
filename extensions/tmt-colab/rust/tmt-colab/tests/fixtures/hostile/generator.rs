//! Deterministic bounded hostile-input smoke; not a formal audit or coverage fuzzer.
use yrs::{Doc, Transact, Update, updates::decoder::Decode};
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let mut seed = 0x830c01ab_u64;
    let mut accepted = 0;
    let mut rejected = 0;
    let mut panics = 0;
    let mut bytes_max = 0;
    let input: serde_json::Value =
        serde_json::from_slice(&std::fs::read("evidence/js-input.json").unwrap()).unwrap();
    let valid: Vec<u8> = serde_json::from_value(input["updates"][0].clone()).unwrap();
    let mut corpus = vec![
        vec![],
        vec![255; 32],
        vec![1, 255, 255, 255, 255, 15],
        vec![0, 255, 255, 255, 255, 15],
        valid.clone(),
    ];
    for n in 0..4096 {
        let mut bytes = if n % 2 == 0 {
            valid.clone()
        } else {
            Vec::new()
        };
        for _ in 0..(n % 256 + 1) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            if n % 2 == 0 {
                let i = (seed as usize) % bytes.len();
                bytes[i] ^= (seed >> 32) as u8;
            } else {
                bytes.push(seed as u8);
            }
        }
        if n % 7 == 0 {
            bytes.truncate(n % bytes.len().max(1));
        }
        corpus.push(bytes);
    }
    let chosen = std::env::args().nth(1).map(|x| x.parse::<usize>().unwrap());
    for (index, bytes) in corpus.into_iter().enumerate() {
        if chosen.is_some_and(|chosen| chosen != index) {
            continue;
        }
        if std::env::args().nth(2).as_deref() == Some("--dump") {
            std::fs::write(format!("evidence/hostile-input-{index}.bin"), &bytes).unwrap();
            continue;
        }
        bytes_max = bytes_max.max(bytes.len());
        match std::panic::catch_unwind(|| {
            let u = Update::decode_v1(&bytes).map_err(|_| ())?;
            let doc = Doc::new();
            let result = doc.transact_mut().apply_update(u).map_err(|_| ());
            result
        }) {
            Ok(Ok(())) => accepted += 1,
            Ok(Err(())) => rejected += 1,
            Err(_) => panics += 1,
        }
    }
    println!(
        "{}",
        serde_json::json!({"seed":"0x830c01ab","cases":accepted+rejected+panics,"accepted":accepted,"rejected":rejected,"panics":panics,"maxInputBytes":bytes_max,"yrs":"0.28.0","rustc":"1.95.0","note":"Malformed decode/apply can still panic; catch_unwind is evidence only, not a runtime mitigation."})
    );
    if panics > 0 {
        std::process::exit(2);
    }
}
