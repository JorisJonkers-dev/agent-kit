fn main() {
    std::process::exit(ticket_export::run(std::env::args_os().skip(1)));
}
