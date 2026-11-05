fn main() {
    if let Err(error) =
        log_forge::run_with_registry(std::env::args(), &log_forge::GeneratorRegistry::new())
    {
        if let Some(clap) = error.downcast_ref::<clap::Error>() {
            let _ = clap.print();
            std::process::exit(clap.exit_code());
        }
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
