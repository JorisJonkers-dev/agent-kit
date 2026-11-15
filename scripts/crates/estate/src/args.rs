//! A small argument reader in argparse's shape: `--flag VALUE`, `--flag=VALUE`,
//! boolean flags, positionals, `--help`, and exit status 2 with the usage on
//! stderr for a mistake.

use crate::Ctx;
use std::collections::{HashMap, HashSet};

pub struct Spec {
    pub cmd: &'static str,
    pub usage: &'static str,
    pub values: &'static [&'static str],
    pub required: &'static [&'static str],
    pub flags: &'static [&'static str],
    pub min_positional: usize,
    pub max_positional: usize,
}

#[derive(Default)]
pub struct Parsed {
    values: HashMap<String, String>,
    flags: HashSet<String>,
    pub positional: Vec<String>,
}

impl Parsed {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}

/// Reads `args` by `spec`. `Err(status)` means the usage or the error is
/// already printed and the command must return that status.
pub fn parse(spec: &Spec, args: &[String], ctx: &mut Ctx) -> Result<Parsed, i32> {
    let mut parsed = Parsed::default();
    let mut iter = args.iter();
    let mut problem: Option<String> = None;
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            ctx.say(spec.usage);
            return Err(0);
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if spec.flags.contains(&name) && inline.is_none() {
            parsed.flags.insert(name.to_string());
        } else if spec.values.contains(&name) {
            match inline.or_else(|| iter.next().cloned()) {
                Some(v) => {
                    parsed.values.insert(name.to_string(), v);
                }
                None => problem = Some(format!("argument {name}: expected one argument")),
            }
        } else if arg.starts_with("--") {
            problem = Some(format!("unrecognized arguments: {arg}"));
        } else {
            parsed.positional.push(arg.clone());
        }
    }
    let missing: Vec<&str> = spec
        .required
        .iter()
        .copied()
        .filter(|r| !parsed.values.contains_key(*r))
        .collect();
    if problem.is_none() && !missing.is_empty() {
        problem = Some(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }
    let count = parsed.positional.len();
    if problem.is_none() && (count < spec.min_positional || count > spec.max_positional) {
        problem = Some("wrong number of positional arguments".to_string());
    }
    match problem {
        None => Ok(parsed),
        Some(message) => {
            ctx.warn(spec.usage);
            ctx.warn(&format!("estate {}: error: {message}", spec.cmd));
            Err(2)
        }
    }
}
