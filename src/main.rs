fn main() {
    let code = match claudex::cli::run_to_exit_code() {
        Ok(code) => code,
        Err(error) => {
            let code = claudex::cli::exit_code(&error);
            print_error(&error);
            code
        }
    };
    std::process::exit(code);
}

fn print_error(error: &anyhow::Error) {
    eprintln!("claudex: {error}");
    for cause in error.chain().skip(1) {
        eprintln!("  caused by: {cause}");
    }
}
