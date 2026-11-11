//! Thin entrypoint: the work lives in the library so the unit tests can cover
//! it; this stays the one executable line they cannot.

fn main() {
    let args = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned());
    std::process::exit(build_tracker::run_with(args, &build_tracker::default_runtime(), &mut std::io::stdout(), &mut std::io::stderr()));
}
