//! Invocation-owned filesystem fixture for Office storage tests.

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) struct TestDirectory {
    pub path: PathBuf,
}

impl TestDirectory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "tmt-office-storage-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // Refuse to reuse an existing directory; cleanup owns only this creation.
        fs::create_dir(&path).expect("create unique Office storage test directory");
        Self { path }
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            if std::thread::panicking() {
                eprintln!(
                    "Could not remove Office storage fixture {}: {error}",
                    self.path.display()
                );
            } else {
                panic!(
                    "Could not remove Office storage fixture {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

/// In-memory whiteboard PNG fixtures; the core companion tests keep their own
/// copy because test-only code cannot cross crates.
pub(crate) mod whiteboard_png {
    use tmt_office_model::office_whiteboard::{HEIGHT, WIDTH};

    pub fn png(
        width: u32,
        height: u32,
        color: png::ColorType,
        pixels: &[u8],
        text: bool,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            if text {
                encoder
                    .add_text_chunk(
                        "Comment".into(),
                        "INERT-METADATA-MUST-NOT-BE-EXPORTED".into(),
                    )
                    .unwrap();
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    pub fn solid(pixel: [u8; 4]) -> Vec<u8> {
        let pixels = pixel.repeat(WIDTH as usize * HEIGHT as usize);
        png(
            u32::from(WIDTH),
            u32::from(HEIGHT),
            png::ColorType::Rgba,
            &pixels,
            false,
        )
    }
}

/// Applies `sql` with `id` at the next preflight boundary through a separate
/// connection, as core retirement does, inside the accepted preflight window.
pub(crate) fn at_next_preflight_execute(path: &std::path::Path, sql: &'static str, id: &str) {
    let path = path.to_path_buf();
    let id = id.to_owned();
    crate::store::tests::at_next_preflight(move || {
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute(sql, [id])
            .unwrap();
    });
}
