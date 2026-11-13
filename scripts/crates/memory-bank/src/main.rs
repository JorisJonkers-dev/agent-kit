//! Thin entrypoint: the work lives in the library so the unit tests can cover
//! it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(memory_bank::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr()));
}
