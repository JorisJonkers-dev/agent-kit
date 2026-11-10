//! Thin entrypoint: the answering lives in the library so the unit tests can
//! cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(gold_band_registry::run(std::env::args_os().skip(1)));
}
