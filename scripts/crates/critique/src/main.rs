//! Thin entrypoint: the work lives in the library so the unit tests can cover
//! it; what stays here is the process itself, which they cannot: handing a
//! command the runner does not port yet to the node runner, and the exit.

fn main() {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let runtime = critique::default_runtime();
    if let Some(node_args) = critique::node_handoff(&args, &runtime) {
        use std::os::unix::process::CommandExt;
        // `exec` returns only when node cannot be started; the Rust runner then says the command is not ported.
        let error = std::process::Command::new("node").args(node_args).exec();
        eprintln!("critique: cannot start node for the node runner: {error}");
    }
    std::process::exit(critique::run_with(args.into_iter(), &runtime, &mut std::io::BufReader::new(std::io::stdin()), &mut std::io::stdout(), &mut std::io::stderr()));
}
