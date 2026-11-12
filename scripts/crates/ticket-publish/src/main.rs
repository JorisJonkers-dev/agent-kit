fn main() {
    std::process::exit(ticket_publish::run(std::env::args_os().skip(1)));
}
