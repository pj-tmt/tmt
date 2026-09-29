//! Exact canonical skill viewing and a deliberately short native quick start.

use std::io::{self, Write};
use tmt_adapters::drivers::Registry;

pub fn execute(skill: Option<&str>) -> io::Result<u8> {
    let mut output = io::stdout().lock();
    if let Some(skill) = skill {
        output.write_all(
            tmt_adapters::skill_installation::bundled_skill_named(skill)
                .expect("grammar validates bundled skill name"),
        )?;
    } else {
        writeln!(
            output,
            "TMT — collaborate with terminal agents through durable exchanges.\n"
        )?;
        writeln!(
            output,
            "Native alpha: standalone runtime, no Node.js or daemon required.\n"
        )?;
        writeln!(
            output,
            "Start with tmt install, then reload your agent's skills."
        )?;
        writeln!(
            output,
            "Providers: {}. Use install all or install --dir <skills-root>.",
            Registry::builtin().names().join(", ")
        )?;
        writeln!(
            output,
            "Installation is non-interactive and does not install agent applications.\n"
        )?;
        writeln!(
            output,
            "  tmt name alice             Bind this pane temporarily; add -s to save it.\n  tmt add %14 reviewer       Bind another pane by stable ID.\n  tmt marked reviewer        Bind the pane explicitly marked in tmux.\n  tmt ls                     Show lifetime and verified presence.\n  tmt talk reviewer 'Review this patch' --timeout 300 --json\n  tmt talk reviewer 'Run the tests' --detach --json\n  tmt result <request-id> --json\n  tmt x ackall --identity coordinator\n  tmt config show --json\n"
        )?;
        writeln!(
            output,
            "Talk waits for tmt reply, not terminal output. Use exactly the supplied\nrequest ID and receipt with reply --message, --file or --stdin. Submit the\ncomplete result before showing a brief truthful user summary. Timeout ends\nonly the observer; preserve the request ID and do not automatically resend.\n"
        )?;
        writeln!(
            output,
            "Outside tmux, use an existing --identity for attributed talk or x.\nCreate saved identities with tmt identity create <name>. Use talk --inbox for\na durable non-pane request and x listen for one bounded incoming wait. Reads\nnever ack.\n\nOptional local Office: inspect with tmt office. With a compatible installed\ncompanion, explicitly use tmt office start for a browser URL. Read\ntmt learn --skill tmt-office for guidance; terminal collaboration needs no Office.\n"
        )?;
        writeln!(
            output,
            "Read tmt learn --skill for complete current safety and usage guidance.\nUse tmt help for options. Managed native installations use tmt upgrade; package-manager installations use their original manager."
        )?;
    }
    Ok(0)
}
