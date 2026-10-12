//! Exact root-local source reads, with bounded line/search projections from one admitted snapshot.
use crate::cli_management::{fail, resolve_page};
use clap::{Arg, ArgMatches, Command};
use serde::Serialize;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_colab::{
    Result,
    decoder::Decoder,
    keyring::{Keyring, Layout},
    limits,
    page::{self, lines},
    store::Store,
};
use tmt_colab_model::crypto;

const READ: CommandSpec = CommandSpec {
    name: "read",
    summary: "Read exact admitted UTF-8 source",
    examples: &[
        Example {
            command: "tmt colab page read 10000000-0000-4000-8000-000000000001 --lines 10:20 --json",
            note: "Read numbered lines and their exact patch anchor",
        },
        Example {
            command: "tmt colab page read 10000000-0000-4000-8000-000000000001 --grep 'quoted text' --context 2 --json",
            note: "Find literal text with surrounding lines",
        },
        Example {
            command: "tmt colab page read 10000000-0000-4000-8000-000000000001 --out page.html --json",
            note: "Write a source-only copy to a new private file",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "Without flags, writes source bytes unchanged to stdout with revision/head/epoch on stderr; JSON contains both. --lines A:B returns numbered lines and rangeSha256 of their exact bytes (LF terminators included, CR retained). --grep is literal, with --context 0..1000 (default 0); overlapping or adjacent ranges merge. A pattern is at most 4 KiB, matches at most 1,000, and complete ranged/search output at most 8 MiB: exceeding a bound refuses, never silently truncates. --lines and --grep are exclusive. --out writes exact whole-source or range bytes to a new 0600 file, never overwriting a path; it conflicts with --grep. JSON reports path, bytes, sha256 and the same revision. This is an unencrypted copy; full tmt colab export remains the complete page copy. Does not create or migrate Colab state.",
};

pub fn command() -> Command {
    tmt_cli_style::command(&READ)
        .arg(crate::cli_grammar::page())
        .arg(
            Arg::new("lines")
                .long("lines")
                .value_name("A:B")
                .value_parser(parse_lines)
                .conflicts_with("grep")
                .help("Read inclusive 1-based LF-delimited lines"),
        )
        .arg(
            Arg::new("grep")
                .long("grep")
                .value_name("TEXT")
                .value_parser(pattern)
                .help("Find nonempty literal text (no regex)"),
        )
        .arg(
            Arg::new("context")
                .long("context")
                .value_name("N")
                .requires("grep")
                .value_parser(
                    clap::value_parser!(u64).range(0..=limits::PAGE_READ_CONTEXT_LINES as u64),
                )
                .help("Lines before and after each literal match (default 0)"),
        )
        .arg(
            Arg::new("out")
                .long("out")
                .value_name("FILE")
                .value_parser(clap::value_parser!(PathBuf))
                .conflicts_with("grep")
                .help("Write exact source bytes to a new private file"),
        )
}
fn parse_lines(text: &str) -> std::result::Result<(usize, usize), &'static str> {
    let parse = || {
        let (a, b) = text.split_once(':')?;
        if a.is_empty() || b.is_empty() || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        let a = a.parse::<usize>().ok()?;
        let b = b.parse::<usize>().ok()?;
        (a > 0 && b >= a).then_some((a, b))
    };
    parse().ok_or("Use an inclusive line range A:B with 1 <= A <= B")
}
fn pattern(text: &str) -> std::result::Result<String, &'static str> {
    if text.is_empty() || text.len() > limits::PAGE_READ_PATTERN_BYTES {
        Err("Use nonempty literal text of at most 4 KiB")
    } else {
        Ok(text.into())
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn capacity() -> Box<dyn std::error::Error + Send + Sync> {
    fail(
        "COLAB_READ_TOO_LARGE",
        "The complete read exceeds its match or output limit. Nothing was printed or written; narrow the range or search.",
    )
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Block<'a> {
    line_start: usize,
    line_end: usize,
    range_sha256: String,
    source: &'a str,
}
fn block(source: &str, start: usize, end: usize) -> Result<Block<'_>> {
    let range = lines::line_range(source, start, end).ok_or_else(|| {
        fail(
            "COLAB_READ_RANGE",
            "The requested line range is outside this page. Nothing was printed or written.",
        )
    })?;
    Ok(Block {
        line_start: start,
        line_end: end,
        range_sha256: hex(&lines::range_sha256(source, start, end).expect("admitted range")),
        source: &source[range],
    })
}
fn search<'a>(source: &'a str, pattern: &str, context: usize) -> Result<(usize, Vec<Block<'a>>)> {
    let last = lines::line_count(source);
    let mut count = 0;
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for (offset, found) in source.match_indices(pattern) {
        count += 1;
        if count > limits::PAGE_READ_MATCHES {
            return Err(capacity());
        }
        let start = lines::line_number(source, offset)
            .saturating_sub(context)
            .max(1);
        let end = lines::line_number(source, offset + found.len() - 1)
            .saturating_add(context)
            .min(last);
        if let Some(previous) = ranges.last_mut()
            && start <= previous.1.saturating_add(1)
        {
            previous.1 = previous.1.max(end);
        } else {
            ranges.push((start, end));
        }
    }
    Ok((
        count,
        ranges
            .into_iter()
            .map(|(a, b)| block(source, a, b))
            .collect::<Result<_>>()?,
    ))
}
fn bounded_write(output: &mut Vec<u8>, text: &[u8]) -> Result<()> {
    if output.len().saturating_add(text.len()) > limits::PAGE_READ_OUTPUT_BYTES {
        return Err(capacity());
    }
    output.extend_from_slice(text);
    Ok(())
}
fn numbered(blocks: &[Block<'_>]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    for block in blocks {
        bounded_write(
            &mut output,
            format!(
                "Lines {}:{} · rangeSha256 {}\n",
                block.line_start, block.line_end, block.range_sha256
            )
            .as_bytes(),
        )?;
        if block.source.is_empty() {
            bounded_write(&mut output, format!("{}\t\n", block.line_start).as_bytes())?;
        }
        for (index, text) in block.source.split_inclusive('\n').enumerate() {
            bounded_write(
                &mut output,
                format!("{}\t", block.line_start + index).as_bytes(),
            )?;
            bounded_write(&mut output, text.as_bytes())?;
            if !text.ends_with('\n') {
                bounded_write(&mut output, b"\n")?;
            }
        }
    }
    Ok(output)
}
fn json_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    if bytes.len() > limits::PAGE_READ_OUTPUT_BYTES {
        return Err(capacity());
    }
    Ok(bytes)
}
fn metadata(value: &page::Page) -> Result<()> {
    let mut output = tmt_cli_style::stream::stderr();
    let terminal = output.terminal();
    tmt_cli_style::detail::write(
        &mut output,
        terminal,
        "PAGE SOURCE",
        &[
            ("page", value.page_id.clone()),
            ("revision", value.revision.clone()),
            ("membership", value.membership_head.revision.clone()),
            ("head", value.membership_head.statement_hash.clone()),
            ("epoch", value.epoch.clone()),
        ],
    )?;
    Ok(())
}
pub fn run(root: &Path, args: &ArgMatches) -> Result<()> {
    let layout = Layout::existing(root)?.ok_or(page::Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id = resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let value = page::read(&store, &key, &id, &mut decoder);
    let closed = store.close();
    let value = value?;
    closed?;
    let selected = args
        .get_one::<(usize, usize)>("lines")
        .map(|&(a, b)| block(&value.source, a, b))
        .transpose()?;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if let Some(path) = args.get_one::<PathBuf>("out") {
        let source = selected
            .as_ref()
            .map_or(value.source.as_str(), |b| b.source);
        if source.len() > limits::PAGE_READ_OUTPUT_BYTES {
            return Err(capacity());
        }
        let published = tmt_colab::export::publish_source(path, source.as_bytes())?;
        let mut result = serde_json::json!({"path":published,"bytes":source.len(),"sha256":hex(&crypto::digest(source.as_bytes())),"revision":value.revision,"pageId":value.page_id});
        if let Some(block) = selected {
            result["lineStart"] = block.line_start.into();
            result["lineEnd"] = block.line_end.into();
            result["rangeSha256"] = block.range_sha256.into();
        }
        if json_output {
            output.write_all(&json_bytes(&result)?)?;
        } else {
            metadata(&value)?;
            writeln!(output, "{}", published.display())?;
            writeln!(
                tmt_cli_style::stream::stderr(),
                "{}",
                tmt_colab::export::DISCLOSURE
            )?;
        }
    } else if let Some(pattern) = args.get_one::<String>("grep") {
        let context = args.get_one::<u64>("context").copied().unwrap_or(0) as usize;
        let (count, blocks) = search(&value.source, pattern, context)?;
        let bytes = if json_output {
            let mut result = serde_json::to_value(&value)?;
            result
                .as_object_mut()
                .expect("page object")
                .remove("source");
            result["matches"] = count.into();
            result["blocks"] = serde_json::to_value(&blocks)?;
            json_bytes(&result)?
        } else {
            numbered(&blocks)?
        };
        if !json_output {
            metadata(&value)?;
        }
        output.write_all(&bytes)?;
    } else if let Some(block) = selected {
        let bytes = if json_output {
            let mut result = serde_json::to_value(&value)?;
            result["source"] = block.source.into();
            result["lineStart"] = block.line_start.into();
            result["lineEnd"] = block.line_end.into();
            result["rangeSha256"] = block.range_sha256.clone().into();
            json_bytes(&result)?
        } else {
            numbered(&[block])?
        };
        if !json_output {
            metadata(&value)?;
        }
        output.write_all(&bytes)?;
    } else if json_output {
        writeln!(output, "{}", serde_json::to_string(&value)?)?;
    } else {
        metadata(&value)?;
        output.write_all(value.source.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_search_merges_context_and_hashes_only_exact_selected_bytes() {
        // Non-ASCII fixture text exercises byte offsets and exact UTF-8 hashes.
        let source = "head\r\nélan\nmiddle\nélan\ntail\n";
        let (count, blocks) = search(source, "élan", 1).unwrap();
        assert_eq!(count, 2);
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].line_start, blocks[0].line_end), (1, 5));
        assert_eq!(blocks[0].source, source);
        assert_eq!(
            blocks[0].range_sha256,
            hex(&crypto::digest(source.as_bytes()))
        );
        let (_, blocks) = search(source, "middle\nélan", 0).unwrap();
        assert_eq!((blocks[0].line_start, blocks[0].line_end), (3, 4));
        assert_eq!(blocks[0].source, "middle\nélan\n");
        assert!(search(source, ".*", 0).unwrap().1.is_empty());
    }
    #[test]
    fn match_and_complete_output_limits_refuse_without_partial_output() {
        assert!(search(&"a".repeat(limits::PAGE_READ_MATCHES + 1), "a", 0).is_err());
        let source = "\n".repeat(2 * 1024 * 1024);
        let selected = block(&source, 1, lines::line_count(&source)).unwrap();
        assert!(numbered(&[selected]).is_err());
        assert!(json_bytes(&serde_json::json!({"source":"\0".repeat(2 * 1024 * 1024)})).is_err());
    }
    #[test]
    fn grammar_keeps_search_literal_and_refuses_incompatible_forms() {
        assert!(
            command()
                .try_get_matches_from(["read", "10000000", "--grep", "a", "--context", "2"])
                .is_ok()
        );
        for flags in [
            vec!["--lines", "1:2", "--grep", "a"],
            vec!["--context", "2"],
            vec!["--grep", "a", "--out", "file"],
            vec!["--grep", "a", "--regex"],
            vec!["--lines", "0:1"],
            vec!["--lines", "1:0"],
            vec!["--grep", ""],
            vec!["--grep", "a", "--context", "1001"],
        ] {
            let args: Vec<_> = ["read", "10000000"].into_iter().chain(flags).collect();
            assert!(command().try_get_matches_from(args).is_err());
        }
    }
}
