//! Thin entrypoint: the work lives in the library so the unit tests can cover
//! it; this stays the one executable line they cannot (and `mcp`, which is given the real stdin).

fn main() {
    let args: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let runtime = model_gateway::default_runtime();
    if args.first().map(String::as_str) == Some("mcp") {
        std::process::exit(model_gateway::mcp::run(&args[1..], &runtime, &mut std::io::stdin().lock(), &mut std::io::stdout(), &mut std::io::stderr()));
    }
    std::process::exit(model_gateway::run_with(args.into_iter(), &runtime, &mut std::io::stdout(), &mut std::io::stderr()));
}
