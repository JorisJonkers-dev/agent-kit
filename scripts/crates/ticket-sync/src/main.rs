//! Thin entrypoint: the answering lives in the library so the unit tests
//! can cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(ticket_sync::run(std::env::args_os().skip(1), "gh", &mut std::io::stdout(), &mut std::io::stderr()));
}
