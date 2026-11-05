fn main() {
    if let Err(error) =
        log_forge::run_with_registry(std::env::args(), &log_forge::GeneratorRegistry::new())
    {
        if let Some(clap) = error.downcast_ref::<clap::Error>() {
            let _ = clap.print();
            std::process::exit(clap.exit_code());
        }
        // A reader such as `head` closed stdout; the output it wanted is complete.
        if is_broken_pipe(error.as_ref()) {
            std::process::exit(0);
        }
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn is_broken_pipe(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        let kind = error
            .downcast_ref::<std::io::Error>()
            .map(std::io::Error::kind)
            // serde_json does not expose its io::Error through source().
            .or_else(|| {
                error
                    .downcast_ref::<serde_json::Error>()
                    .and_then(serde_json::Error::io_error_kind)
            });
        if kind == Some(std::io::ErrorKind::BrokenPipe) {
            return true;
        }
        current = error.source();
    }
    false
}
