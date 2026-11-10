//! Thin entrypoint: the work lives in the library so the unit tests can cover
//! it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(code_nav_eval::run(std::env::args_os().skip(1)));
}
