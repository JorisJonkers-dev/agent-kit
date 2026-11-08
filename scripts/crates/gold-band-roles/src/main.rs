//! Thin entrypoint: the rendering and convergence live in the library so the
//! unit tests can cover them; this stays the one executable line they cannot.

fn main() {
    std::process::exit(gold_band_roles::run(std::env::args_os().skip(1)));
}
