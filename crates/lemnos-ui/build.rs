// Runs before the crate is compiled: scans the source for Tailwind classes and
// generates the stylesheet that `tailwind::stylesheet!()` points to.
//
// No `rerun-if-changed` here on purpose: printing one would replace Cargo's
// default of rerunning on any file change in this crate, and then new classes
// in `.rs` files (or edits to `src/styles/app.css`) would stop being picked up.
fn main() {
    topcoat::tailwind::BuildConfig::new()
        .input("src/styles/app.css")
        .render()
        .unwrap();
}
