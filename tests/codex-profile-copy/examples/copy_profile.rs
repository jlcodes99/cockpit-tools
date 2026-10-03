use std::path::Path;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: copy_profile SOURCE_HOME EMPTY_TARGET_HOME");
        std::process::exit(2);
    }
    if let Err(error) =
        cockpit_codex_profile_copy_tests::copy_profile(Path::new(&args[0]), Path::new(&args[1]))
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
