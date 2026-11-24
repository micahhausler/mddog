//! The clipboard: through the desktop's copy command, and through the terminal for a viewer over SSH.
use std::io::Write;
use std::process::{Command, Stdio};

type Helper = &'static [&'static str];

/// Puts `text` on the clipboard with the desktop's copy command, and returns the OSC 52 sequence that does it
/// through the terminal.
pub fn copy(text: &str) -> String {
    let text = pasteable(text);
    let seq = osc52(&text);
    let helpers = helpers();
    if !helpers.is_empty() {
        std::thread::spawn(move || first_that_copies(&helpers, &text));
    }
    seq
}

fn pasteable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\r')).collect()
}

/// OSC 52 puts `text` on the clipboard of the terminal the user sits at, also over SSH.
fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x1b\\", crate::term::kitty::base64(text.as_bytes()))
}

#[cfg(target_os = "macos")]
fn helpers() -> Vec<Helper> {
    vec![&["pbcopy"]]
}

#[cfg(not(target_os = "macos"))]
fn helpers() -> Vec<Helper> {
    let set = |k| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    desktop_helpers(set("WAYLAND_DISPLAY"), set("DISPLAY"))
}

#[cfg(any(test, not(target_os = "macos")))]
fn desktop_helpers(wayland: bool, x11: bool) -> Vec<Helper> {
    let mut out: Vec<Helper> = Vec::new();
    if wayland {
        out.push(&["wl-copy", "--type", "text/plain;charset=utf-8"]);
    }
    if x11 {
        out.extend::<[Helper; 2]>([&["xclip", "-selection", "clipboard", "-in"], &["xsel", "--clipboard", "--input"]]);
    }
    out
}

fn first_that_copies(helpers: &[Helper], text: &str) -> Option<Helper> {
    helpers.iter().copied().find(|h| put(h, text))
}

fn put(helper: Helper, text: &str) -> bool {
    let [program, args @ ..] = helper else { return false };
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    if cfg!(target_os = "macos") {
        cmd.env("LC_CTYPE", "UTF-8");
    }
    let Ok(mut child) = cmd.spawn() else { return false };
    let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let exited = child.wait().is_ok_and(|s| s.success());
    written && exited
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_text_drops_control_characters() {
        assert_eq!(pasteable("ls\x1b[201~echo\u{9b}hi\x07\n\tok\r\n"), "ls[201~echohi\n\tok\r\n");
    }

    #[test]
    fn osc52_carries_the_text_in_base64() {
        assert_eq!(osc52("é✓"), "\x1b]52;c;w6ninJM=\x1b\\");
    }

    #[test]
    fn the_session_picks_the_copy_commands() {
        let names = |w, x| desktop_helpers(w, x).iter().map(|h| h[0]).collect::<Vec<_>>();
        assert_eq!(names(true, true), ["wl-copy", "xclip", "xsel"]);
        assert_eq!(names(true, false), ["wl-copy"]);
        assert_eq!(names(false, true), ["xclip", "xsel"]);
        assert!(names(false, false).is_empty(), "over SSH only OSC 52 copies");
        assert!(desktop_helpers(false, true)[0].contains(&"clipboard"), "xclip's default is the primary selection");
    }

    #[test]
    fn a_failing_or_missing_command_falls_through_to_the_next() {
        let dir = std::env::temp_dir().join(format!("mddog-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("got");
        let sink: &'static str = Box::leak(out.to_string_lossy().into_owned().into_boxed_str());
        let missing: Helper = &["mddog-no-such-copy-command"];
        let failing: Helper = &["false"];
        let works: &'static [&'static str] = Box::leak(Box::new(["sh", "-c", "cat > \"$0\"", sink]));
        assert_eq!(first_that_copies(&[missing, failing, works], "é✓ text"), Some(works));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "é✓ text");
        assert_eq!(first_that_copies(&[missing, failing], "x"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
