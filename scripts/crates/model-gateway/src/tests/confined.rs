//! Helpers for tests of launches under the host backend (#903): what a fake process records is the plan command,
//! and the client's own command and the grants it carries are read apart.

use crate::ports::ProcessCommand;

/// The program of the plan command a launch under the host backend hands the fake process.
pub const LAUNCHER: &str = crate::sandbox::PLAN_PROGRAM;

/// Where the client's own words start in a confined command, and where the launcher's start: `(launcher, after
/// the "--")`. `None` for a command that is not behind the launcher.
fn split(words: &[String]) -> Option<(usize, usize)> {
    let at = words.iter().position(|word| word == LAUNCHER)?;
    let end = words.iter().skip(at).position(|word| word == "--")? + at;
    (words.get(at + 1).map(String::as_str) == Some(crate::sandbox::PLAN_VERB)).then_some((at, end + 1))
}

/// The command as the client sees it, without the launcher and its grants, and the grants (the words between
/// `confine` and `--`) apart.
pub fn unwrapped(command: &ProcessCommand) -> (ProcessCommand, Vec<String>) {
    let mut words = vec![command.command.clone()];
    words.extend(command.args.clone());
    let Some((start, after)) = split(&words) else {
        return (command.clone(), Vec::new());
    };
    let grants = words[start + 2..after - 1].to_vec();
    words.drain(start..after);
    let rest = words.split_off(1);
    (ProcessCommand { command: words.remove(0), args: rest, ..command.clone() }, grants)
}

/// The value of every occurrence of a grant flag.
pub fn grant_values(grants: &[String], flag: &str) -> Vec<String> {
    grants.iter().enumerate().filter(|(at, _)| *at > 0 && grants[*at - 1] == flag).map(|(_, value)| value.clone()).collect()
}
