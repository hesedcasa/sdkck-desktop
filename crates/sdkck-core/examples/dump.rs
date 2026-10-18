//! Print the native catalog's command ids, one per line (for parity checks
//! against `sdkck commands --json`).
use sdkck_core::catalog::{build, CatalogSource};
use sdkck_core::env::Env;
use sdkck_core::paths::SdkckDirs;

fn main() {
    let root = std::env::args().nth(1).expect("usage: dump <sdkck root>");
    let dirs = SdkckDirs::resolve(&Env::from_process());
    let catalog = build(&CatalogSource { root: root.as_ref(), dirs: &dirs }).unwrap();
    let verbose = std::env::args().any(|a| a == "--verbose");
    for c in &catalog.commands {
        if verbose {
            let args: Vec<_> = c.args.iter().map(|a| a.name.as_str()).collect();
            let mut flags: Vec<_> = c.flags.iter().map(|f| f.name.as_str()).collect();
            flags.sort();
            println!("{} {args:?} {flags:?}", c.id);
        } else {
            println!("{}", c.id);
        }
    }
}
