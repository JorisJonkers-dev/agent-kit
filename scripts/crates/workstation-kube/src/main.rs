//! Thin entrypoint: the work lives in the library so the unit tests cover it.

fn main() {
    std::process::exit(workstation_kube::run(std::env::args_os().skip(1)));
}
