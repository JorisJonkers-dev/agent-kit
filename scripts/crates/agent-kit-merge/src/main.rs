//! Thin entrypoint: the commands live in the library so the unit tests can
//! cover them; this stays the one executable file they cannot (the exit wrapper).

use agent_kit_merge::{cli, Io};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut stdin, mut stdout, mut stderr) = (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let mut io = Io { stdin: &mut stdin, out: &mut stdout, err: &mut stderr };
    std::process::exit(cli::run(&args, &mut io, &|name| std::env::var(name).ok()));
}
