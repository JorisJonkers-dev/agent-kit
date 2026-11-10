//! Thin entrypoint: the checking lives in the library so the unit tests can
//! cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(inventory_check::run(std::env::args_os().skip(1)));
}
