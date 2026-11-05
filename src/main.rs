fn main() {
    if let Err(error) =
        log_forge::run_with_registry(std::env::args(), &log_forge::GeneratorRegistry::new())
    {
        if let Some(clap) = error.downcast_ref::<clap::Error>() {
            let _ = clap.print();
            std::process::exit(clap.exit_code());
        }
        // A reader such as `head` closed stdout; the output it wanted is complete.
        if log_forge::is_stdout_closed(error.as_ref()) {
            std::process::exit(0);
        }
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
