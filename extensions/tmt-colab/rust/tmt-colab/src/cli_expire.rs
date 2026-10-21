//! `tmt colab expire --dry-run`: the pages the next scheduled archive would archive (#2600), judged
//! by the rules the serve's worker uses. It reads only; archiving is the serve's.
use serde_json::json;
use std::{collections::BTreeSet, io::Write, path::Path};
use tmt_colab::{Result, expirer, page, page_expiry::DAY_MS, settings};

pub fn run(root: &Path, json_output: bool) -> Result<()> {
    let space = settings::read_or_default(root).page_expiry();
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )
    .map_err(|_| "Local clock is beyond the supported range.")?;
    let plan = match page::save::open_source(
        root,
        tmt_colab::decoder::Config::new(std::env::current_exe()?),
    ) {
        Ok(mut source) => expirer::plan_for(&mut source, space, now, &mut BTreeSet::new())?,
        // No local space yet: nothing is idle.
        Err(_) => expirer::Plan::default(),
    };
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let pages: Vec<_> = plan
            .archive
            .iter()
            .map(|idle| {
                json!({"pageId":idle.page,"lastActivityAtMs":idle.last_activity_ms,
                    "archiveAfterDays":idle.days,"dueAtMs":idle.due_ms})
            })
            .collect();
        writeln!(
            out,
            "{}",
            json!({"dryRun":true,"pageExpiryDays":space.days(),"clockBehind":plan.clock_behind,
                "archive":pages,"shells":plan.shells,"more":plan.more})
        )?;
        return Ok(());
    }
    let terminal = out.terminal();
    let mut rows = Vec::new();
    for idle in &plan.archive {
        rows.push((
            idle.page.as_str(),
            format!(
                "{} days idle, limit {} days",
                now.saturating_sub(idle.last_activity_ms) / DAY_MS,
                idle.days
            ),
        ));
    }
    let heading = if plan.clock_behind {
        "NOTHING WOULD BE ARCHIVED"
    } else if rows.is_empty() {
        "NOTHING IS DUE"
    } else {
        "WOULD ARCHIVE"
    };
    if plan.clock_behind {
        rows.push((
            "clock",
            "behind the recorded page activity; the scheduled archive waits".to_owned(),
        ));
    }
    if plan.shells > 0 {
        rows.push(("shell pages", format!("{} left alone", plan.shells)));
    }
    if plan.more {
        rows.push(("more", "the next pass archives the rest".to_owned()));
    }
    rows.push((
        "page-expiry",
        space
            .days()
            .map_or_else(|| "off".to_owned(), |days| format!("{days} days")),
    ));
    tmt_cli_style::detail::write(&mut out, terminal, heading, &rows)?;
    Ok(())
}
