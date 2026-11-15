//! The small part of Python's `argparse` the two helpers used: long and short
//! options with a value, flags, one optional positional, `--opt=value`, and
//! argparse's usage and error text (exit status 2 for a bad command line).

use std::collections::HashMap;

pub struct Opt {
    pub short: Option<&'static str>,
    pub long: &'static str,
    /// A flag takes no value.
    pub flag: bool,
    pub required: bool,
    pub help: &'static str,
}

pub struct Spec {
    pub prog: &'static str,
    pub synopsis: &'static str,
    pub positional: Option<&'static str>,
    pub opts: &'static [Opt],
}

#[derive(Default)]
pub struct Parsed {
    pub values: HashMap<&'static str, String>,
    pub positional: Option<String>,
}

pub enum Outcome {
    Run(Parsed),
    /// `-h`: the text to print, exit status 0.
    Help(String),
    /// A usage error: the text for stderr, exit status 2.
    Error(String),
}

impl Opt {
    fn label(&self) -> String {
        match self.short {
            Some(s) => format!("{s}/{}", self.long),
            None => self.long.to_string(),
        }
    }
}

impl Spec {
    pub fn usage(&self) -> String {
        format!("usage: {} {}\n", self.prog, self.synopsis)
    }

    fn help(&self) -> String {
        let mut text = self.usage();
        text.push_str("\noptions:\n");
        for opt in self.opts {
            text.push_str(&format!("  {:<22}{}\n", opt.label(), opt.help));
        }
        text
    }

    fn error(&self, message: &str) -> Outcome {
        Outcome::Error(format!("{}{}: error: {message}\n", self.usage(), self.prog))
    }

    fn find(&self, arg: &str) -> Option<&Opt> {
        self.opts
            .iter()
            .find(|o| o.long == arg || o.short == Some(arg))
    }

    pub fn parse(&self, args: &[String]) -> Outcome {
        let mut parsed = Parsed::default();
        let mut extra: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str();
            i += 1;
            if arg == "-h" || arg == "--help" {
                return Outcome::Help(self.help());
            }
            let (name, inline) = match arg.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n, Some(v.to_string())),
                _ => (arg, None),
            };
            let Some(opt) = self.find(name) else {
                if arg.starts_with('-') && arg.len() > 1 {
                    extra.push(arg);
                } else if self.positional.is_some() && parsed.positional.is_none() {
                    parsed.positional = Some(arg.to_string());
                } else {
                    extra.push(arg);
                }
                continue;
            };
            if opt.flag {
                parsed.values.insert(opt.long, String::new());
                continue;
            }
            let value = inline.or_else(|| {
                let next = args.get(i).filter(|v| !v.starts_with('-') || v.len() == 1);
                i += usize::from(next.is_some());
                next.cloned()
            });
            let Some(value) = value else {
                return self.error(&format!("argument {}: expected one argument", opt.label()));
            };
            parsed.values.insert(opt.long, value);
        }
        let mut missing: Vec<String> = self
            .opts
            .iter()
            .filter(|o| o.required && !parsed.values.contains_key(o.long))
            .map(Opt::label)
            .collect();
        if let Some(name) = self.positional.filter(|_| parsed.positional.is_none()) {
            missing.insert(0, name.to_string());
        }
        if !missing.is_empty() {
            return self.error(&format!(
                "the following arguments are required: {}",
                missing.join(", ")
            ));
        }
        if !extra.is_empty() {
            return self.error(&format!("unrecognized arguments: {}", extra.join(" ")));
        }
        Outcome::Run(parsed)
    }
}
