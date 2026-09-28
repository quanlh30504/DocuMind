use documind_lib::core::system_deps;

fn main() {
    let deps = system_deps::check_all();
    for d in &deps {
        println!("{:<40} {:<20} {}", d.name, d.package, if d.installed { "installed" } else { "MISSING" });
    }
    let missing = system_deps::missing_packages(&deps);
    println!("\nmissing packages: {missing:?}");
    if !missing.is_empty() {
        println!("manual command: {}", system_deps::manual_install_command(&missing));
    }
}
