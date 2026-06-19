fn main() {
    if let Err(error) = slj1660_mac_driver::cli::run_from_env() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
