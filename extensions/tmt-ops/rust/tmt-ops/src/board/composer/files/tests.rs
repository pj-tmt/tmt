use super::*;
use crate::board::app::App;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::path::PathBuf;

/// A scratch directory under the system temp dir, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ops-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes
}

fn escaped(path: &str) -> String {
    path.replace(' ', "\\ ")
}

#[test]
fn shell_quoting_reads_back_as_one_token_per_file() {
    let read = |text| tokens(text);
    assert_eq!(read("/a/b.png"), Some(vec!["/a/b.png".into()]));
    assert_eq!(
        read("'/a/My File.png' \"/a/two words.md\""),
        Some(vec!["/a/My File.png".into(), "/a/two words.md".into()])
    );
    assert_eq!(
        read("/a/My\\ File.png\n/a/b.txt"),
        Some(vec!["/a/My File.png".into(), "/a/b.txt".into()])
    );
    assert_eq!(
        read("\"/a/q\\\"uote\\\\.png\" /a/it\\'s.png"),
        Some(vec!["/a/q\"uote\\.png".into(), "/a/it's.png".into()])
    );
    assert_eq!(read("'/a/open"), None, "an open quote is not a drop");
    assert_eq!(read("/a/trailing\\"), None);
    assert_eq!(read("   "), Some(vec![]));
}

#[test]
fn file_uris_decode_to_absolute_paths() {
    assert_eq!(
        uri_path("file:///a/b%20c.png").as_deref(),
        Some("/a/b c.png")
    );
    assert_eq!(
        uri_path("file://localhost/a/b.png").as_deref(),
        Some("/a/b.png")
    );
    assert_eq!(uri_path("file://elsewhere/a/b.png"), None);
    assert_eq!(uri_path("file:///a/%zz"), None);
    assert_eq!(uri_path("file:///a/%ff"), None, "not UTF-8");
}

#[test]
fn only_a_paste_of_existing_files_is_a_drop() {
    let dir = Scratch::new("parse");
    let one = dir.file("screenshot.png", &png(1440, 900));
    let spaced = dir.file("my notes.md", b"# Notes\n");
    let other = dir.file("plain.txt", b"plain");
    let files =
        |text: &str| parse(text).map(|files| files.into_iter().map(|f| f.path).collect::<Vec<_>>());

    assert_eq!(files(&one), Some(vec![one.clone()]));
    assert_eq!(files(&format!("'{spaced}'")), Some(vec![spaced.clone()]));
    assert_eq!(files(&escaped(&spaced)), Some(vec![spaced.clone()]));
    assert_eq!(
        files(&format!("{one} '{spaced}' {other}")),
        Some(vec![one.clone(), spaced.clone(), other.clone()])
    );
    let uri = format!("file://{}", spaced.replace(' ', "%20"));
    assert_eq!(files(&uri), Some(vec![spaced.clone()]));
    assert_eq!(parse(&one).unwrap()[0].size, png(1440, 900).len() as u64);

    assert_eq!(
        files(&format!("{one} and some words")),
        None,
        "a mix stays text"
    );
    assert_eq!(
        files(&format!("{one} {}/missing.png", dir.0.display())),
        None
    );
    assert_eq!(
        files(dir.0.to_str().unwrap()),
        None,
        "a directory is not a file"
    );
    assert_eq!(files("relative/path.png"), None);
    assert_eq!(files("~/shot.png"), None);
    assert_eq!(files("just some text"), None);
    assert_eq!(files(""), None);
    let many = (0..=MAX_FILES)
        .map(|_| one.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(files(&many), None, "more than the limit is text");
    assert_eq!(files(&format!("{one} {}", "x".repeat(MAX_PASTE))), None);
}

#[test]
fn sizes_read_like_a_file_manager() {
    assert_eq!(size(0), "0 B");
    assert_eq!(size(1023), "1023 B");
    assert_eq!(size(1024), "1 KB");
    assert_eq!(size(312 * 1024), "312 KB");
    assert_eq!(size(1536 * 1024), "1.5 MB");
    assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GB");
}

#[test]
fn image_headers_give_their_dimensions_without_decoding() {
    assert_eq!(head_of(&png(1200, 800)).dims, Some((1200, 800)));
    assert_eq!(head_of(&png(1200, 800)).kind, Some("image/png"));
    assert_eq!(head_of(&png(0, 800)).dims, None, "a zero side is no size");

    let mut gif = b"GIF89a".to_vec();
    gif.extend([0x20, 0x03, 0x58, 0x02]);
    assert_eq!(head_of(&gif).dims, Some((800, 600)));

    // SOI, an APP0 segment to skip, then SOF0: 480 high, 640 wide.
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46];
    jpeg.extend([0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80]);
    assert_eq!(head_of(&jpeg).dims, Some((640, 480)));
    assert_eq!(head_of(&jpeg).kind, Some("image/jpeg"));
    assert_eq!(
        head_of(&[0xFF, 0xD8, 0xFF, 0xE0, 0xFF]).dims,
        None,
        "truncated"
    );

    let riff = |chunk: &[u8; 4], body: &[u8]| {
        let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
        bytes.extend(chunk);
        bytes.extend([0; 4]);
        bytes.extend(body);
        bytes
    };
    let mut x = vec![0; 10];
    x[4..7].copy_from_slice(&[0x1F, 0x03, 0x00]); // 800 wide
    x[7..10].copy_from_slice(&[0x57, 0x02, 0x00]); // 600 high
    assert_eq!(head_of(&riff(b"VP8X", &x)).dims, Some((800, 600)));
    let mut lossy = vec![0; 3];
    lossy.extend([0x9D, 0x01, 0x2A, 0x20, 0x03, 0x58, 0x02]);
    assert_eq!(head_of(&riff(b"VP8 ", &lossy)).dims, Some((800, 600)));
    let bits: u32 = (800 - 1) | ((600 - 1) << 14);
    let mut lossless = vec![0x2F];
    lossless.extend(bits.to_le_bytes());
    assert_eq!(head_of(&riff(b"VP8L", &lossless)).dims, Some((800, 600)));
}

#[test]
fn text_files_show_their_first_lines_and_binaries_show_nothing() {
    let body = (0..30).map(|n| format!("line {n}\n")).collect::<String>();
    let head = head_of(body.as_bytes());
    assert_eq!(head.kind, Some("text"));
    let lines = head.lines.unwrap();
    assert_eq!(lines.len(), TEXT_LINES);
    assert_eq!(lines[0], "line 0");

    let long = head_of("é".repeat(LINE_CHARS * 2).as_bytes())
        .lines
        .unwrap();
    assert_eq!(long[0].chars().count(), LINE_CHARS);
    let cut = &"é".repeat(5000).into_bytes()[..TEXT_BYTES - 1];
    assert!(
        head_of(cut).lines.is_some(),
        "a cut character is still text"
    );

    assert_eq!(head_of(b"\0\x01\x02 binary"), Head::default());
    assert_eq!(head_of(&[0xC3, 0x28]), Head::default(), "invalid UTF-8");
    assert_eq!(
        head_of(b""),
        Head {
            kind: Some("text"),
            lines: Some(vec![]),
            ..Head::default()
        }
    );
}

#[test]
fn the_worker_reads_a_file_and_reports_one_that_cannot_be_read() {
    let dir = Scratch::new("probe");
    let image = dir.file("a.png", &png(64, 32));
    let (sent, received) = std::sync::mpsc::channel();
    let probe = Probe::spawn(move |delivery| sent.send(delivery).is_ok());
    probe.request(&image);
    probe.request(&format!("{}/gone.png", dir.0.display()));
    let first = received
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_eq!(
        (first.path.as_str(), first.head.dims),
        (image.as_str(), Some((64, 32)))
    );
    let second = received
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(second.head.failed);
}

#[test]
fn the_table_keeps_the_newest_files_and_probes_each_path_once() {
    let mut table = Table::default();
    let file = |n: usize| Dropped {
        path: format!("/f/{n}"),
        size: n as u64,
    };
    assert!(table.note(&file(1)));
    assert!(table.note(&file(1)), "still unprobed");
    table.apply(Delivery {
        path: "/f/1".into(),
        head: Head::default(),
    });
    assert!(!table.note(&file(1)), "probed once");
    assert!(
        table
            .apply(Delivery {
                path: "/nope".into(),
                head: Head::default()
            })
            .is_none()
    );
    for n in 2..=TABLE_LIMIT + 1 {
        table.note(&file(n));
    }
    assert!(table.get("/f/1").is_none(), "the oldest went first");
    assert!(table.get(&format!("/f/{}", TABLE_LIMIT + 1)).is_some());
}

#[test]
fn labels_and_popup_follow_what_is_known() {
    let mut table = Table::default();
    let file = Dropped {
        path: "/a/screenshot.png".into(),
        size: 312 * 1024,
    };
    table.note(&file);
    let info = table.get(&file.path).unwrap().clone();
    assert_eq!(label(&file.path, &info), "screenshot.png · 312 KB ×");
    assert_eq!(content(&file.path, &info, true), "screenshot.png 312 KB");
    assert!(popup(&file.path, &info).ends_with("Reading…"));

    let info = table
        .apply(Delivery {
            path: file.path.clone(),
            head: Head {
                dims: Some((1200, 800)),
                kind: Some("image/png"),
                ..Head::default()
            },
        })
        .unwrap()
        .clone();
    assert_eq!(
        label(&file.path, &info),
        "screenshot.png · 1200×800 · 312 KB ×"
    );
    assert_eq!(
        content(&file.path, &info, true),
        "screenshot.png 1200x800 312 KB"
    );
    let popup = popup(&file.path, &info);
    assert!(popup.starts_with("/a/screenshot.png\n\nSize: 312 KB (319488 bytes)"));
    assert!(popup.contains("Type: image/png") && popup.contains("Dimensions: 1200×800"));

    let failed = Info {
        size: 1,
        head: Some(Head {
            failed: true,
            ..Head::default()
        }),
        ..info
    };
    assert!(popup_of(&failed).contains("Cannot read this file."));
}

fn popup_of(info: &Info) -> String {
    popup("/a/x", info)
}

fn open_composer() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        app.input.is_some(),
        "Enter opens the composer for the selected member"
    );
    app
}

fn draw(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn click(column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn chips(app: &App) -> Vec<(String, String)> {
    let input = app.input.as_ref().unwrap();
    input
        .text
        .folds()
        .iter()
        .map(|fold| {
            (
                input.text.as_str()[fold.source.clone()].to_owned(),
                fold.label.clone(),
            )
        })
        .collect()
}

#[test]
fn a_drop_into_the_open_composer_inserts_one_chip_per_file_and_sends_their_paths() {
    let dir = Scratch::new("drop");
    let image = dir.file("screenshot.png", &png(1440, 900));
    let notes = dir.file("my notes.md", b"# Notes\n");
    let mut app = open_composer();
    app.paste("look at ");
    app.paste(&format!("{image} '{notes}'"));
    app.paste("thanks");
    let text = app.input.as_ref().unwrap().text.as_str().to_owned();
    assert_eq!(
        text,
        format!("look at {image} {notes} thanks"),
        "the editor text is what Send carries"
    );
    assert_eq!(
        chips(&app),
        vec![
            (
                image.clone(),
                format!("screenshot.png · {} ×", size(png(1440, 900).len() as u64))
            ),
            (notes.clone(), "my notes.md · 8 B ×".to_owned()),
        ]
    );
    let screen = draw(&mut app, 100, 36);
    assert!(screen.contains("screenshot.png · 29 B ×"), "{screen}");
    assert!(screen.contains("my notes.md · 8 B ×"), "{screen}");
    assert!(!screen.contains(&image), "the path is behind the chip");
}

#[test]
fn a_later_probe_widens_the_chip_with_the_image_size() {
    let dir = Scratch::new("probe-app");
    let image = dir.file("screenshot.png", &png(1440, 900));
    let mut app = open_composer();
    app.paste(&image);
    assert!(!draw(&mut app, 100, 36).contains("1440×900"));
    assert!(app.apply_file_info(Delivery {
        path: image.clone(),
        head: head_of(&png(1440, 900))
    }));
    let screen = draw(&mut app, 100, 36);
    assert!(
        screen.contains("screenshot.png · 1440×900 · 29 B ×"),
        "{screen}"
    );
    // The retained draft follows too.
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let again = app.composer.files.borrow().get(&image).cloned().unwrap();
    assert_eq!(again.head.unwrap().dims, Some((1440, 900)));
    assert!(!app.apply_file_info(Delivery {
        path: "/unknown".into(),
        head: Head::default()
    }));
}

#[test]
fn ascii_chips_use_brackets_and_keep_the_size() {
    let dir = Scratch::new("ascii");
    let image = dir.file("screenshot.png", &png(1440, 900));
    let mut app = open_composer();
    app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::from_ascii(true);
    app.paste(&image);
    app.apply_file_info(Delivery {
        path: image,
        head: head_of(&png(1440, 900)),
    });
    let screen = draw(&mut app, 100, 36);
    assert!(
        screen.contains("[screenshot.png 1440x900 29 B x]"),
        "{screen}"
    );
}

#[test]
fn a_drop_with_the_composer_closed_opens_it_for_the_selected_member() {
    let dir = Scratch::new("closed");
    let image = dir.file("a.png", &png(10, 10));
    let mut app = crate::board::app::workspace::tests::board();
    assert!(app.input.is_none());
    app.paste(&image);
    assert!(app.input.is_some(), "the drop starts a message");
    assert_eq!(chips(&app).len(), 1);
}

#[test]
fn a_drop_with_nobody_selected_says_so_and_inserts_nothing() {
    let dir = Scratch::new("none");
    let image = dir.file("a.png", &png(10, 10));
    let mut app = crate::board::app::workspace::tests::board();
    app.workspace.selected = None;
    app.paste(&image);
    assert!(app.input.is_none());
    assert_eq!(app.notice.as_deref(), Some("Select an agent to send files"));
}

#[test]
fn a_mixed_or_missing_paste_stays_ordinary_text() {
    let dir = Scratch::new("text");
    let image = dir.file("a.png", &png(10, 10));
    let mut app = open_composer();
    app.paste(&format!("{image} is the file"));
    app.paste(&format!("{}/missing.png", dir.0.display()));
    assert!(chips(&app).is_empty());
    assert!(
        app.input
            .as_ref()
            .unwrap()
            .text
            .as_str()
            .contains("is the file")
    );
}

#[test]
fn clicking_a_chip_opens_the_file_popup_and_its_mark_removes_it() {
    let dir = Scratch::new("click");
    let notes = dir.file("notes.md", b"# Title\nsecond line\n");
    let mut app = open_composer();
    app.paste(&notes);
    app.apply_file_info(Delivery {
        path: notes.clone(),
        head: head_of(b"# Title\nsecond line\n"),
    });
    draw(&mut app, 100, 36);
    let hit = app.composer.paste_hits.borrow()[0].clone();
    app.mouse(click(hit.area.x, hit.area.y), std::time::Instant::now());
    assert!(app.paste_preview_shown());
    let screen = draw(&mut app, 100, 36);
    assert!(screen.contains("File · notes.md"), "{screen}");
    assert!(
        screen.contains("Size: 20 B (20 bytes)") && screen.contains("Type: text"),
        "{screen}"
    );
    let info = app.composer.files.borrow().get(&notes).cloned().unwrap();
    assert!(popup(&notes, &info).contains(&format!("{notes}\n\nSize")));
    assert!(popup(&notes, &info).ends_with("# Title\nsecond line"));
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!app.paste_preview_shown());
    draw(&mut app, 100, 36);
    let hit = app.composer.paste_hits.borrow()[0].clone();
    app.mouse(click(hit.remove.x, hit.remove.y), std::time::Instant::now());
    assert!(chips(&app).is_empty());
    assert!(!app.input.as_ref().unwrap().text.as_str().contains(&notes));
}

pub(in crate::board) const SCENES: &[&str] = &[
    "file-pending",
    "file-single",
    "file-mixed",
    "file-ascii",
    "file-tiny",
];

/// The oracle's file-chip scenes. Paths are synthetic and fixed, so the capture never depends
/// on the machine: the chips show names, sizes and header dimensions only.
pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
    width: u16,
    height: u16,
) -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().look = look;
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    for squad in &workspace.roster.squads {
        workspace.squad_looks.insert(squad.room_id.clone(), look);
    }
    if scene == "file-ascii" {
        app.cron_glyphs = crate::board::overview::cron_counts::GlyphSet::Ascii;
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.paste("check ");
    let shot = Dropped {
        path: "/oracle/shots/screenshot.png".into(),
        size: 312 * 1024,
    };
    let mut dropped = vec![shot.clone()];
    if matches!(scene, "file-mixed" | "file-ascii") {
        dropped.push(Dropped {
            path: "/oracle/notes/my notes.md".into(),
            size: 4096,
        });
        dropped.push(Dropped {
            path: "/oracle/data/a-very-long-file-name-that-clips-the-chip-in-narrow-widths.json"
                .into(),
            size: 1536 * 1024,
        });
    }
    app.insert_dropped(dropped);
    if scene != "file-pending" {
        app.apply_file_info(Delivery {
            path: shot.path,
            head: Head {
                dims: Some((1200, 800)),
                kind: Some("image/png"),
                ..Head::default()
            },
        });
    }
    app.paste("thanks");
    draw(&mut app, width, height);
    app
}

pub(in crate::board) fn capture_metadata(app: &App) -> serde_json::Value {
    let rect = |area: ratatui::layout::Rect| [area.x, area.y, area.width, area.height];
    let chips = app
        .composer
        .paste_hits
        .borrow()
        .iter()
        .map(|hit| {
            serde_json::json!({
                "area": rect(hit.area), "remove": rect(hit.remove), "label": hit.label,
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "chips": chips })
}
